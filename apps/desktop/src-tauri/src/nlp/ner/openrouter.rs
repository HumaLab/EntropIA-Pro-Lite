use serde::Deserialize;

use super::types::{sanitize_entity_value, Entity, EntitySource, EntityType};
use super::{find_entity_span, is_suppressed_by_protected, normalize_entity_value};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NerPayload {
    Array(Vec<RawNerEntity>),
    Object { entities: Vec<RawNerEntity> },
}

#[derive(Debug, Deserialize)]
struct RawNerEntity {
    #[serde(default, alias = "entity", alias = "text")]
    value: String,
    #[serde(default, alias = "entity_type", alias = "category", alias = "label")]
    #[serde(rename = "type")]
    entity_type: String,
    #[serde(default)]
    start_offset: Option<usize>,
    #[serde(default)]
    end_offset: Option<usize>,
    #[serde(default)]
    confidence: Option<f32>,
}

pub fn build_ner_prompt(template: &str, text: &str) -> String {
    crate::llm::prompt::render_template(template, text)
}

pub async fn extract_entities_with_openrouter(
    api_key: String,
    generation: crate::llm::generation::FlowGenerationConfig,
    prompt_template: &str,
    text: &str,
    protected_entities: &[Entity],
) -> Result<Vec<Entity>, String> {
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Err(openrouter_ner_unavailable(
            "OpenRouter API key no configurada para NER",
        ));
    }

    let model_name = generation.model.clone();
    let client = crate::llm::openrouter::OpenRouterClient::new(api_key, model_name.clone());
    let raw = client
        .generate(&build_ner_prompt(prompt_template, text), &generation.params)
        .await
        .map_err(|error| openrouter_ner_unavailable(&error))?;

    parse_openrouter_entities(text, protected_entities, &raw, &model_name)
}

pub fn parse_openrouter_entities(
    text: &str,
    protected_entities: &[Entity],
    raw_response: &str,
    model_name: &str,
) -> Result<Vec<Entity>, String> {
    let content = strip_markdown_fences(raw_response);
    let json = extract_json_payload(&content)?;
    let payload: NerPayload = serde_json::from_str(json)
        .map_err(|error| format!("OpenRouter NER failed to parse JSON: {error}."))?;

    let raw_entities = match payload {
        NerPayload::Array(items) => items,
        NerPayload::Object { entities } => entities,
    };

    let mut deduped_keys = std::collections::HashSet::new();
    let mut entities = Vec::new();
    for raw in raw_entities {
        let value = sanitize_entity_value(&raw.value);
        if value.is_empty() {
            continue;
        }
        let Some(entity_type) = parse_openrouter_entity_type(&raw.entity_type) else {
            continue;
        };
        let (start_offset, end_offset) = match (raw.start_offset, raw.end_offset) {
            (Some(start), Some(end)) if end >= start => (start, end),
            _ => find_entity_span(text, &value).unwrap_or((0, 0)),
        };

        let entity = Entity {
            entity_type,
            value,
            start_offset,
            end_offset,
            confidence: raw.confidence.unwrap_or(0.95).clamp(0.0, 1.0),
            source: EntitySource::Llm,
            model_name: Some(model_name.to_string()),
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

pub fn openrouter_ner_unavailable(reason: &str) -> String {
    format!("OpenRouter NER unavailable: {reason}. Configure OpenRouter API key/model.")
}

fn parse_openrouter_entity_type(value: &str) -> Option<EntityType> {
    match value.trim().to_ascii_uppercase().as_str() {
        "PER" | "PERSON" | "PERSONA" => Some(EntityType::Person),
        "LOC" | "LOCATION" | "PLACE" | "LUGAR" => Some(EntityType::Place),
        "ORG" | "ORGANIZATION" | "ORGANISATION" | "INSTITUTION" | "INSTITUCION" | "INSTITUCIÓN" => {
            Some(EntityType::Organization)
        }
        "DATE" | "FECHA" => Some(EntityType::Date),
        "MISC" | "OTHER" | "OTRO" => Some(EntityType::Misc),
        _ => None,
    }
}

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

fn extract_json_payload(content: &str) -> Result<&str, String> {
    let start = content
        .find('[')
        .or_else(|| content.find('{'))
        .ok_or_else(|| "OpenRouter NER did not return JSON content.".to_string())?;
    let end = content
        .rfind(']')
        .or_else(|| content.rfind('}'))
        .ok_or_else(|| "OpenRouter NER did not return a closed JSON payload.".to_string())?;

    if end < start {
        return Err("OpenRouter NER returned malformed JSON boundaries.".to_string());
    }

    Ok(&content[start..=end])
}
