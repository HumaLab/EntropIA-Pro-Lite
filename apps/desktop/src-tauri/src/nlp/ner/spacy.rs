use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::AppHandle;

use super::types::{sanitize_entity_value, Entity, EntitySource, EntityType};
use super::{is_suppressed_by_protected, normalize_entity_value};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const DEFAULT_SPACY_MODEL: &str = "es_core_news_md";

// Flat confidence assigned to spaCy NER entities. spaCy's default pipeline does
// not expose per-entity confidence, so this is a trust score for the local model.
// It sits exactly at the UI's automatic-entity display floor (ItemView.svelte
// shows confidence >= 0.85) so es_core_news_md entities are surfaced.
const SPACY_ENTITY_CONFIDENCE: f32 = 0.85;

#[derive(Debug, Deserialize)]
struct RawSpacyEntity {
    value: String,
    #[serde(rename = "type")]
    entity_type: String,
    start_offset: Option<usize>,
    end_offset: Option<usize>,
    confidence: Option<f32>,
}

pub fn extract_entities_with_spacy(
    app_handle: &AppHandle,
    settings_db_path: &Path,
    text: &str,
    protected_entities: &[Entity],
) -> Result<Vec<Entity>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }

    let python_path = resolve_spacy_python(settings_db_path)
        .ok_or_else(|| "spaCy Python no disponible con modelo es_core_news_md".to_string())?;
    let script_path = resolve_spacy_script(app_handle)?;

    let mut cmd = Command::new(&python_path);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.arg(&script_path)
        .arg("--model")
        .arg(DEFAULT_SPACY_MODEL)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|error| {
        format!(
            "No se pudo iniciar spaCy NER (python={}, script={}): {error}",
            python_path.display(),
            script_path.display()
        )
    })?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|error| format!("No se pudo enviar texto a spaCy NER: {error}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|error| format!("spaCy NER falló al esperar salida: {error}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "spaCy NER no disponible (exit code {}). Stderr: {} Stdout: {}",
            output.status.code().unwrap_or(-1),
            stderr.trim(),
            stdout.trim()
        ));
    }

    parse_spacy_entities(text, protected_entities, &stdout)
}

pub fn parse_spacy_entities(
    _text: &str,
    protected_entities: &[Entity],
    raw_response: &str,
) -> Result<Vec<Entity>, String> {
    let json = extract_sentinel_json(raw_response)?;
    let raw_entities: Vec<RawSpacyEntity> = serde_json::from_str(json)
        .map_err(|error| format!("spaCy NER devolvió JSON inválido: {error}"))?;

    let mut deduped_keys = std::collections::HashSet::new();
    let mut entities = Vec::new();
    for raw in raw_entities {
        let value = sanitize_entity_value(&raw.value);
        if value.is_empty() {
            continue;
        }
        let Some(entity_type) = parse_spacy_entity_type(&raw.entity_type) else {
            continue;
        };
        let entity = Entity {
            entity_type,
            value,
            start_offset: raw.start_offset.unwrap_or(0),
            end_offset: raw.end_offset.unwrap_or(0),
            confidence: raw
                .confidence
                .unwrap_or(SPACY_ENTITY_CONFIDENCE)
                .clamp(0.0, 1.0),
            source: EntitySource::RuleBased,
            model_name: Some(DEFAULT_SPACY_MODEL.to_string()),
        };

        if is_suppressed_by_protected(&entity, protected_entities) {
            continue;
        }

        let key = (
            normalize_entity_value(&entity.value),
            entity.entity_type.as_str().to_string(),
        );
        if deduped_keys.insert(key) {
            entities.push(entity);
        }
    }

    entities.sort_by_key(|entity| entity.start_offset);
    Ok(entities)
}

fn resolve_spacy_python(settings_db_path: &Path) -> Option<PathBuf> {
    crate::python_discovery::which_python_for_module(
        "nlp/spacy",
        "spacy_ner_es",
        "spaCy es_core_news_md",
        "import spacy; spacy.load('es_core_news_md'); print('ok')",
        Some(settings_db_path),
    )
}

fn resolve_spacy_script(app_handle: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(Some(root)) = crate::runtime::RuntimeManager::new().hydrated_runtime_root(app_handle)
    {
        let managed = crate::runtime::managed_script_path(&root, "spacy_ner.py");
        if managed.exists() {
            return Ok(managed);
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let canonical = manifest_dir.join("scripts/spacy_ner.py");
    canonical
        .exists()
        .then_some(canonical)
        .ok_or_else(|| "Script spaCy NER no encontrado: scripts/spacy_ner.py".to_string())
}

fn extract_sentinel_json(output: &str) -> Result<&str, String> {
    const BEGIN: &str = "===SPACY_NER_JSON_BEGIN===";
    const END: &str = "===SPACY_NER_JSON_END===";
    let start = output
        .find(BEGIN)
        .map(|idx| idx + BEGIN.len())
        .ok_or_else(|| "spaCy NER no devolvió marcador JSON inicial".to_string())?;
    let rest = &output[start..];
    let end = rest
        .find(END)
        .ok_or_else(|| "spaCy NER no devolvió marcador JSON final".to_string())?;
    Ok(rest[..end].trim())
}

fn parse_spacy_entity_type(value: &str) -> Option<EntityType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "person" | "persona" | "per" => Some(EntityType::Person),
        "place" | "location" | "lugar" | "loc" | "gpe" => Some(EntityType::Place),
        "date" | "fecha" => Some(EntityType::Date),
        "organization" | "organizacion" | "organización" | "institution" | "org" => {
            Some(EntityType::Organization)
        }
        "misc" | "other" | "otro" => Some(EntityType::Misc),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spacy_sentinel_json() {
        let raw = r#"noise
===SPACY_NER_JSON_BEGIN===
[{"value":"Buenos Aires","type":"place","start_offset":0,"end_offset":12,"confidence":0.91}]
===SPACY_NER_JSON_END===
"#;
        let entities = parse_spacy_entities("Buenos Aires", &[], raw).expect("valid spaCy JSON");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].value, "Buenos Aires");
        assert_eq!(entities[0].entity_type, EntityType::Place);
        assert_eq!(entities[0].source, EntitySource::RuleBased);
        assert_eq!(entities[0].model_name.as_deref(), Some(DEFAULT_SPACY_MODEL));
    }
}
