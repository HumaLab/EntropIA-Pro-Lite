//! RAG (Retrieval-Augmented Generation) chat sobre la base documental
//! (extracciones OCR + transcripciones).
//!
//! Recuperación híbrida (embeddings + FTS5 fusionados con Reciprocal Rank
//! Fusion) que alimenta un prompt de fragmentos numerados para que el modelo
//! responda con citas `[n]`.
//!
//! Pro es 100% LOCAL: la respuesta se genera con el motor Gemma local
//! (`crate::llm::engine::LlmEngine`) y el embedding de la consulta con el
//! proveedor de embeddings local (BGE-M3 ONNX). El branch OpenRouter existe
//! solo cuando `llm_mode` lo selecciona explícitamente; el camino por defecto
//! funciona SIN ninguna API key configurada.

#[cfg(test)]
mod baseline;
#[cfg(test)]
mod baseline_press;
pub mod commands;
pub(crate) mod intent;
pub(crate) mod params;
pub(crate) mod query_rewrite;
pub(crate) mod reranker;
pub(crate) mod retrieval;
pub(crate) mod scope;
pub(crate) mod store;

use serde::{Deserialize, Serialize};

/// Un turno previo de la conversación, reconstruido desde la base de
/// conversaciones persistidas (`rag_messages`). Tipo interno: nunca cruza
/// el boundary de serialización hacia el frontend.
#[derive(Debug, Clone)]
pub struct RagChatTurn {
    pub role: String,
    pub content: String,
}

/// Respuesta final que recibe el frontend. `conversation_id` es el id real
/// de la conversación persistida (fresco si no existía o fue borrada).
/// Es `None` cuando la persistencia falló DESPUÉS de una respuesta exitosa
/// del LLM: la respuesta se devuelve igual, pero no hay id que adoptar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagAnswer {
    pub answer: String,
    pub sources: Vec<RagSource>,
    pub model: String,
    pub conversation_id: Option<String>,
    /// Why the Biblioteca leg contributed nothing (`no_library_synced`,
    /// `no_embeddings`, `embedding_unavailable`, `failed`); `None` when the
    /// scope did not ask for it or it ran normally. Not persisted: it describes
    /// this answer's retrieval, not the conversation.
    pub bibliography_notice: Option<String>,
    /// Short, secret-free cause behind `failed` / `embedding_unavailable`, so
    /// the notice says what broke instead of only that something did.
    pub bibliography_notice_detail: Option<String>,
    /// Lo que la recuperación del corpus tuvo que resignar para esta
    /// respuesta (por ejemplo `whole_pages_only`: sin espacio de embedding
    /// resoluble, se recuperaron páginas enteras en lugar de pasajes). Vacío
    /// cuando corrió completa. No se persiste; la interfaz lo declara (F4).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retrieval_degradation: Vec<RagDegradation>,
}

/// Una degradación declarada de la recuperación: un código estable para la
/// interfaz y la causa corta, sin secretos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RagDegradation {
    pub code: String,
    pub detail: String,
}

/// Resumen de una conversación persistida para el listado del frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagConversationSummary {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
}

/// Conversación completa con sus mensajes en orden cronológico.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagConversation {
    pub id: String,
    pub title: String,
    pub messages: Vec<RagMessage>,
}

/// Un mensaje persistido de una conversación. `sources` solo trae contenido
/// en los mensajes del asistente (vacío para los del usuario).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub sources: Vec<RagSource>,
    pub created_at: i64,
}

/// Procedencia canónica del bloque de evidencia empaquetado para una cita.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSourceProvenance {
    pub retrieval_unit: String,
    pub source_kind: String,
    pub source_id: String,
    pub chunk_ids: Vec<String>,
    pub start_char: usize,
    pub end_char: usize,
}

/// Una fuente citada. `index` es 1-based y coincide con las citas `[n]`
/// incluidas en el texto de la respuesta.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSource {
    pub index: u32,
    pub asset_id: String,
    pub item_id: String,
    pub item_title: String,
    pub collection_id: String,
    pub collection_name: String,
    pub snippet: String,
    pub score: f64,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    #[serde(default)]
    pub provenance: Option<RagSourceProvenance>,
    /// Present only for a passage of the Zotero library (Biblioteca scope).
    /// Absent means a corpus source, which is also what every conversation
    /// persisted before this field existed reads as. Optional and ignored by
    /// older builds, so synced messages stay readable on both sides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bibliography: Option<RagBibliographySource>,
}

/// Where a bibliography citation lives, stored with the source so the list
/// renders (work, location, scope) without touching the local-only catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagBibliographySource {
    /// `bibliographic_chunks.id`: how the passage is reopened on this device.
    pub chunk_id: String,
    pub item_key: String,
    pub library_name: String,
    /// `user` or `group`, with the native id: the identity Zotero uses.
    pub library_type: String,
    pub library_native_id: String,
    /// CSL family names, comma-separated (may be empty).
    pub authors: String,
    pub year: Option<i64>,
    pub location: Option<RagBibliographyLocation>,
}

/// `pages` for a PDF (a page range) or `paragraphs` for an HTML snapshot
/// (a paragraph range of its single text; a snapshot has no real pages).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagBibliographyLocation {
    pub kind: String,
    pub from: i64,
    pub to: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rag_source_without_provenance_deserializes_as_legacy_asset_source() {
        let source: RagSource = serde_json::from_value(serde_json::json!({
            "index": 1,
            "assetId": "asset-legacy",
            "itemId": "item-legacy",
            "itemTitle": "Acta histórica",
            "collectionId": "collection-legacy",
            "collectionName": "Archivo",
            "snippet": "fragmento persistido",
            "score": 0.75,
            "startSeconds": 1.5,
            "endSeconds": 4.0
        }))
        .expect("persisted RagSource JSON without provenance must remain readable");

        assert_eq!(source.asset_id, "asset-legacy");
        assert_eq!(source.snippet, "fragmento persistido");
        assert!(source.provenance.is_none());
    }

    #[test]
    fn rag_answer_carries_the_retrieval_degradation_for_the_ui() {
        let answer = RagAnswer {
            answer: String::new(),
            sources: Vec::new(),
            model: "modelo".to_string(),
            conversation_id: None,
            bibliography_notice: None,
            bibliography_notice_detail: None,
            retrieval_degradation: vec![RagDegradation {
                code: "whole_pages_only".to_string(),
                detail: "Proveedor de embeddings no soportado: mystery.".to_string(),
            }],
        };

        let json = serde_json::to_value(&answer).expect("answer serializes");

        assert_eq!(
            json["retrievalDegradation"],
            serde_json::json!([{
                "code": "whole_pages_only",
                "detail": "Proveedor de embeddings no soportado: mystery."
            }])
        );
    }
}
