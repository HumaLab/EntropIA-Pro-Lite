pub mod openrouter;
#[cfg(feature = "local-ml")]
pub mod spacy;
pub mod types;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::nlp::text_provider;

use self::types::{sanitize_entity_value, Entity};

#[allow(unused_imports)]
pub use self::types::{EntitySource, EntityType};

#[derive(Clone)]
pub struct EntityExtractionBatch {
    pub text: String,
    pub entities: Vec<Entity>,
}

pub struct NerExtractionInput {
    pub text: String,
    pub protected_entities: Vec<Entity>,
}

pub struct OpenRouterExtractionInput {
    pub text: String,
    pub protected_entities: Vec<Entity>,
    pub api_key: String,
    pub generation: crate::llm::generation::FlowGenerationConfig,
    pub prompt_template: String,
}

pub fn prepare_ner_candidates_for_item(
    conn: &Connection,
    item_id: &str,
) -> Result<NerExtractionInput, String> {
    Ok(NerExtractionInput {
        text: text_provider::get_item_text(conn, item_id)?,
        protected_entities: load_protected_entities(conn, item_id)?,
    })
}

pub fn prepare_ner_candidates_for_asset(
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
) -> Result<NerExtractionInput, String> {
    Ok(NerExtractionInput {
        text: text_provider::get_asset_text(conn, asset_id)?,
        protected_entities: load_protected_entities(conn, item_id)?,
    })
}

#[allow(dead_code)] // Future: LLM entity review pipeline (not yet wired)
#[derive(Serialize)]
struct EntityReviewCandidate<'a> {
    value: &'a str,
    #[serde(rename = "type")]
    entity_type: &'a str,
    confidence: f32,
}

#[allow(dead_code)] // Future: LLM entity review pipeline (not yet wired)
#[derive(Deserialize)]
struct ReviewedEntity {
    #[serde(default, alias = "entity", alias = "text")]
    value: String,
    #[serde(default, alias = "entity_type")]
    #[serde(rename = "type")]
    entity_type: String,
    #[serde(default)]
    confidence: Option<f32>,
}

#[allow(dead_code)] // Future: LLM entity review pipeline (not yet wired)
pub fn serialize_review_candidates(entities: &[Entity]) -> Result<String, String> {
    let payload = entities
        .iter()
        .map(|entity| EntityReviewCandidate {
            value: entity.value.as_str(),
            entity_type: entity.entity_type.as_str(),
            confidence: entity.confidence,
        })
        .collect::<Vec<_>>();

    serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize entity review candidates: {e}"))
}

#[allow(dead_code)] // Future: LLM entity review pipeline (not yet wired)
pub fn apply_llm_review(
    text: &str,
    candidate_entities: &[Entity],
    protected_entities: &[Entity],
    raw_review_json: &str,
) -> Result<Vec<Entity>, String> {
    let content = strip_markdown_fences(raw_review_json);
    let start = content.find('[').or_else(|| content.find('{'));
    let end = content.rfind(']').or_else(|| content.rfind('}'));

    let Some(start_idx) = start else {
        return Err("LLM entity review did not return JSON content".to_string());
    };
    let Some(end_idx) = end else {
        return Err("LLM entity review did not return a closed JSON payload".to_string());
    };

    let slice = &content[start_idx..=end_idx];
    let reviewed_entities = if slice.starts_with('[') {
        serde_json::from_str::<Vec<ReviewedEntity>>(slice)
            .map_err(|e| format!("Failed to parse LLM entity review array: {e}"))?
    } else {
        vec![serde_json::from_str::<ReviewedEntity>(slice)
            .map_err(|e| format!("Failed to parse LLM entity review object: {e}"))?]
    };

    let mut deduped_keys = std::collections::HashSet::new();
    let mut final_entities = Vec::new();

    for reviewed in reviewed_entities {
        let value = sanitize_entity_value(&reviewed.value);
        if value.is_empty() {
            continue;
        }

        let Some(entity_type) = parse_entity_type_alias(&reviewed.entity_type) else {
            continue;
        };

        let confidence = reviewed.confidence.unwrap_or(0.95).clamp(0.0, 1.0);
        let mut entity = Entity {
            entity_type,
            value,
            start_offset: 0,
            end_offset: 0,
            confidence,
            source: EntitySource::Llm,
            model_name: Some("gemma-4-E2B-it-Q4_K_M".to_string()),
        };

        if let Some(existing) = candidate_entities.iter().find(|candidate| {
            same_entity_family(&candidate.entity_type, &entity.entity_type)
                && normalize_entity_value(&candidate.value) == normalize_entity_value(&entity.value)
        }) {
            entity.start_offset = existing.start_offset;
            entity.end_offset = existing.end_offset;
            entity.confidence = entity.confidence.max(existing.confidence);
        } else if let Some((start_offset, end_offset)) = find_entity_span(text, &entity.value) {
            entity.start_offset = start_offset;
            entity.end_offset = end_offset;
        }

        if is_suppressed_by_protected(&entity, protected_entities) {
            continue;
        }

        let dedupe_key = (
            normalize_entity_value(&entity.value),
            entity.entity_type.as_str().to_string(),
        );
        if deduped_keys.insert(dedupe_key) {
            final_entities.push(entity);
        }
    }

    Ok(final_entities)
}

pub fn persist_entities_for_item(
    conn: &Connection,
    item_id: &str,
    entities: &[Entity],
) -> Result<(), String> {
    // Wrap delete+insert in a single transaction so a mid-write failure can never
    // leave entities deleted-but-not-reinserted (atomicity). The empty-result guard
    // that prevents wiping good entities on a poorer re-run lives at the call site,
    // where the text-present context is known.
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to start entity persist transaction: {e}"))?;
    delete_automatic_entities(&tx, item_id)?;
    insert_entities_for_item(&tx, item_id, entities)?;
    tx.commit()
        .map_err(|e| format!("Failed to commit entity persist transaction: {e}"))
}

pub fn persist_entities_for_asset(
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
    entities: &[Entity],
) -> Result<(), String> {
    // Atomic delete+insert — see persist_entities_for_item for rationale.
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to start entity persist transaction: {e}"))?;
    delete_automatic_entities_for_asset(&tx, item_id, asset_id)?;
    insert_entities_for_asset(&tx, item_id, asset_id, entities)?;
    tx.commit()
        .map_err(|e| format!("Failed to commit entity persist transaction: {e}"))
}

fn insert_entities_for_item(
    conn: &Connection,
    item_id: &str,
    entities: &[Entity],
) -> Result<(), String> {
    for entity in entities {
        conn.execute(
            r#"
            INSERT INTO entities (
                id, item_id, entity_type, value, start_offset, end_offset,
                confidence, source, model_name, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                uuid::Uuid::new_v4().to_string(),
                item_id,
                entity.entity_type.as_str(),
                entity.value.as_str(),
                entity.start_offset as i64,
                entity.end_offset as i64,
                entity.confidence as f64,
                entity.source.as_str(),
                entity.model_name.clone(),
                now_millis(),
            ],
        )
        .map_err(|e| format!("Failed to insert entity: {e}"))?;
    }

    Ok(())
}

fn insert_entities_for_asset(
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
    entities: &[Entity],
) -> Result<(), String> {
    for entity in entities {
        conn.execute(
            r#"
            INSERT INTO entities (
                id, item_id, asset_id, entity_type, value, start_offset, end_offset,
                confidence, source, model_name, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                uuid::Uuid::new_v4().to_string(),
                item_id,
                asset_id,
                entity.entity_type.as_str(),
                entity.value.as_str(),
                entity.start_offset as i64,
                entity.end_offset as i64,
                entity.confidence as f64,
                entity.source.as_str(),
                entity.model_name.clone(),
                now_millis(),
            ],
        )
        .map_err(|e| format!("Failed to insert entity: {e}"))?;
    }

    Ok(())
}

fn delete_automatic_entities(conn: &Connection, item_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM entities WHERE item_id = ?1 AND COALESCE(source, '') NOT IN ('manual', 'manual_deleted')",
        params![item_id],
    )
    .map_err(|e| format!("Failed to delete automatic entities: {e}"))?;
    Ok(())
}

/// Delete automatic entities for a specific asset, preserving manual entities
/// and entities that belong to other assets or the item level.
fn delete_automatic_entities_for_asset(
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM entities WHERE item_id = ?1 AND asset_id = ?2 AND COALESCE(source, '') NOT IN ('manual', 'manual_deleted')",
        params![item_id, asset_id],
    )
    .map_err(|e| format!("Failed to delete automatic entities for asset: {e}"))?;
    Ok(())
}

fn load_protected_entities(conn: &Connection, item_id: &str) -> Result<Vec<Entity>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT entity_type, value, start_offset, end_offset, confidence, source, model_name
            FROM entities
            WHERE item_id = ?1 AND COALESCE(source, '') IN ('manual', 'manual_deleted')
            "#,
        )
        .map_err(|e| format!("Failed to prepare protected entity query: {e}"))?;

    let rows = stmt
        .query_map(params![item_id], |row| {
            let entity_type_str: String = row.get(0)?;
            let entity_type = parse_entity_type(&entity_type_str).unwrap_or(EntityType::Misc);
            let source_str: Option<String> = row.get(5)?;
            let source = match source_str.as_deref() {
                Some("llm") => EntitySource::Llm,
                _ => EntitySource::RuleBased,
            };

            Ok(Entity {
                entity_type,
                value: row.get(1)?,
                start_offset: row.get::<_, i64>(2)?.max(0) as usize,
                end_offset: row.get::<_, i64>(3)?.max(0) as usize,
                confidence: row.get::<_, f64>(4)? as f32,
                source,
                model_name: row.get(6)?,
            })
        })
        .map_err(|e| format!("Failed to query protected entities: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect protected entities: {e}"))
}

pub(crate) fn openrouter_settings(
    conn: &Connection,
) -> Result<(String, crate::llm::generation::FlowGenerationConfig, String), String> {
    let api_key = crate::settings::get_setting(conn, "openrouter_api_key")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            openrouter::openrouter_ner_unavailable("OpenRouter API key no configurada")
        })?;

    Ok((
        api_key,
        crate::llm::generation::generation_config_from_settings(
            conn,
            crate::llm::generation::GenerationFlow::Ner,
        ),
        crate::llm::generation::prompt_template_from_settings(
            conn,
            crate::llm::generation::GenerationFlow::Ner,
        ),
    ))
}

#[allow(dead_code)] // Future: used by apply_llm_review (not yet wired)
fn strip_markdown_fences(text: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }

    let without_opening = trimmed
        .strip_prefix("```")
        .unwrap_or(trimmed)
        .trim_start_matches("json")
        .trim_start_matches("JSON")
        .trim();

    without_opening
        .strip_suffix("```")
        .unwrap_or(without_opening)
        .trim()
        .to_string()
}

fn parse_entity_type(value: &str) -> Option<EntityType> {
    match value {
        "person" => Some(EntityType::Person),
        "place" => Some(EntityType::Place),
        "date" => Some(EntityType::Date),
        "institution" => Some(EntityType::Institution),
        "organization" => Some(EntityType::Organization),
        "misc" => Some(EntityType::Misc),
        _ => None,
    }
}

#[allow(dead_code)] // Future: used by apply_llm_review (not yet wired)
fn parse_entity_type_alias(value: &str) -> Option<EntityType> {
    match value.trim().to_lowercase().as_str() {
        "person" | "persona" => Some(EntityType::Person),
        "place" | "location" | "lugar" => Some(EntityType::Place),
        "date" | "fecha" => Some(EntityType::Date),
        "institution" | "institucion" | "institución" => Some(EntityType::Institution),
        "organization" | "organizacion" | "organización" => Some(EntityType::Organization),
        "misc" | "other" | "otro" => Some(EntityType::Misc),
        _ => parse_entity_type(value.trim()),
    }
}

#[allow(dead_code)] // Future: used by apply_llm_review (not yet wired)
fn find_entity_span(text: &str, value: &str) -> Option<(usize, usize)> {
    let needle = value.trim();
    if needle.is_empty() {
        return None;
    }

    let haystack_lower = text.to_lowercase();
    let needle_lower = needle.to_lowercase();
    let byte_start = haystack_lower.find(&needle_lower)?;
    let byte_end = byte_start + needle_lower.len();
    Some((
        text[..byte_start].chars().count(),
        text[..byte_end].chars().count(),
    ))
}

fn is_suppressed_by_protected(candidate: &Entity, protected_entities: &[Entity]) -> bool {
    protected_entities.iter().any(|protected| {
        same_entity_family(&candidate.entity_type, &protected.entity_type)
            && (same_normalized_value(candidate, protected) || spans_overlap(candidate, protected))
    })
}

fn same_entity_family(a: &EntityType, b: &EntityType) -> bool {
    match (a, b) {
        (EntityType::Organization, EntityType::Institution)
        | (EntityType::Institution, EntityType::Organization) => true,
        _ => a == b,
    }
}

fn same_normalized_value(a: &Entity, b: &Entity) -> bool {
    normalize_entity_value(&a.value) == normalize_entity_value(&b.value)
}

fn normalize_entity_value(value: &str) -> String {
    sanitize_entity_value(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn spans_overlap(a: &Entity, b: &Entity) -> bool {
    a.start_offset < b.end_offset && b.start_offset < a.end_offset
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::openrouter::{build_ner_prompt, parse_openrouter_entities};
    use super::*;

    #[test]
    fn openrouter_ner_strips_json_fences_and_normalizes_aliases() {
        let text = "Don Manuel Belgrano llegó a Buenos Aires con el Cabildo el 25 de mayo.";
        let raw = r#"```json
        [
          {"value":"Manuel Belgrano","type":"PER","confidence":0.97},
          {"entity":"Buenos Aires","category":"LOC"},
          {"text":"Cabildo","label":"ORG"},
          {"value":"25 de mayo","type":"DATE"}
        ]
        ```"#;

        let entities = parse_openrouter_entities(text, &[], raw, "google/gemma-3-4b-it")
            .expect("valid fenced JSON should parse");

        assert_eq!(entities.len(), 4);
        assert_eq!(entities[0].entity_type, EntityType::Person);
        assert_eq!(entities[0].value, "Manuel Belgrano");
        assert_eq!(entities[1].entity_type, EntityType::Place);
        assert_eq!(entities[2].entity_type, EntityType::Organization);
        assert_eq!(entities[3].entity_type, EntityType::Date);
        assert!(entities
            .iter()
            .all(|entity| entity.source == EntitySource::Llm));
    }

    #[test]
    fn openrouter_ner_rejects_bad_json_without_spacy_fallback() {
        // Delimited but malformed payload: reaches the serde parse step (rather than
        // the no-delimiters guard) so the contract under test — a parse failure must
        // surface, never silently fall back — is actually exercised.
        let error =
            parse_openrouter_entities("texto", &[], "{not valid json}", "google/gemma-3-4b-it")
                .expect_err("bad JSON should not silently fall back");

        assert!(error.contains("OpenRouter NER"));
        assert!(error.contains("failed to parse JSON"));
    }

    #[test]
    fn openrouter_ner_preserves_manual_entity_protection() {
        let protected = vec![Entity {
            entity_type: EntityType::Person,
            value: "Manuel Belgrano".to_string(),
            start_offset: 4,
            end_offset: 20,
            confidence: 1.0,
            source: EntitySource::RuleBased,
            model_name: None,
        }];
        let raw =
            r#"[{"value":"Manuel Belgrano","type":"PER"},{"value":"Buenos Aires","type":"LOC"}]"#;

        let entities = parse_openrouter_entities(
            "Don Manuel Belgrano viajó a Buenos Aires.",
            &protected,
            raw,
            "google/gemma-3-4b-it",
        )
        .expect("valid JSON should parse");

        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].value, "Buenos Aires");
        assert_eq!(entities[0].entity_type, EntityType::Place);
    }

    #[test]
    fn openrouter_ner_prompt_requests_only_supported_categories() {
        let prompt = build_ner_prompt(
            crate::llm::prompt::DEFAULT_NER_PROMPT,
            "Juan llegó a Rosario.",
        );

        assert!(prompt.contains("PER"));
        assert!(prompt.contains("LOC"));
        assert!(prompt.contains("ORG"));
        assert!(prompt.contains("DATE"));
        assert!(prompt.contains("MISC"));
        assert!(prompt.contains("JSON"));
        assert!(prompt.contains("Juan llegó a Rosario."));
        assert!(!prompt.contains("{text}"));
    }

    // ── Asset entity persistence contract ─────────────────────────────────────

    fn entities_test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE entities (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              asset_id TEXT,
              entity_type TEXT NOT NULL,
              value TEXT NOT NULL,
              start_offset INTEGER NOT NULL,
              end_offset INTEGER NOT NULL,
              confidence REAL NOT NULL,
              source TEXT,
              model_name TEXT,
              created_at INTEGER NOT NULL
            );
            "#,
        )
        .expect("entities schema");
        conn
    }

    fn seed_entity(conn: &Connection, item_id: &str, asset_id: &str, value: &str, source: &str) {
        conn.execute(
            "INSERT INTO entities (id, item_id, asset_id, entity_type, value, start_offset, end_offset, confidence, source, model_name, created_at) \
             VALUES (?1, ?2, ?3, 'person', ?4, 0, 1, 0.9, ?5, NULL, 0)",
            params![
                uuid::Uuid::new_v4().to_string(),
                item_id,
                asset_id,
                value,
                source
            ],
        )
        .expect("seed entity");
    }

    fn asset_entity_values(conn: &Connection, item_id: &str, asset_id: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT value FROM entities WHERE item_id = ?1 AND asset_id = ?2 ORDER BY value",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![item_id, asset_id], |r| r.get::<_, String>(0))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    fn new_person(value: &str) -> Entity {
        Entity {
            entity_type: EntityType::Person,
            value: value.to_string(),
            start_offset: 0,
            end_offset: 5,
            confidence: 0.85,
            source: EntitySource::RuleBased,
            model_name: None,
        }
    }

    #[test]
    fn persist_entities_for_asset_replaces_automatic_and_keeps_manual() {
        let conn = entities_test_conn();
        seed_entity(&conn, "item-1", "asset-1", "Gemma Persona", "llm");
        seed_entity(&conn, "item-1", "asset-1", "Manual Persona", "manual");

        persist_entities_for_asset(&conn, "item-1", "asset-1", &[new_person("Nueva Persona")])
            .expect("persist should succeed");

        let values = asset_entity_values(&conn, "item-1", "asset-1");
        assert!(
            values.iter().any(|v| v == "Manual Persona"),
            "manual entity must be preserved, got: {values:?}"
        );
        assert!(
            values.iter().any(|v| v == "Nueva Persona"),
            "new automatic entity must be inserted, got: {values:?}"
        );
        assert!(
            !values.iter().any(|v| v == "Gemma Persona"),
            "old automatic entity must be replaced, got: {values:?}"
        );
    }

    #[test]
    fn persist_entities_for_asset_with_empty_is_destructive_to_automatic() {
        // persist is UNCONDITIONALLY destructive to automatic entities. This is
        // exactly why the worker (nlp/mod.rs) must NOT call it with an empty result
        // when the source text is present — the call-site data-loss guard prevents
        // wiping a richer earlier Gemma result. This test pins that contract so the
        // guard's necessity stays documented.
        let conn = entities_test_conn();
        seed_entity(&conn, "item-1", "asset-1", "Gemma Persona", "llm");
        seed_entity(&conn, "item-1", "asset-1", "Manual Persona", "manual");

        persist_entities_for_asset(&conn, "item-1", "asset-1", &[])
            .expect("persist should succeed");

        let values = asset_entity_values(&conn, "item-1", "asset-1");
        assert!(
            values.iter().any(|v| v == "Manual Persona"),
            "manual entity must survive an empty persist, got: {values:?}"
        );
        assert!(
            !values.iter().any(|v| v == "Gemma Persona"),
            "automatic entity is cleared by an empty persist (guarded at the call site), got: {values:?}"
        );
    }
}
