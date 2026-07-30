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

pub mod commands;
pub(crate) mod params;
#[cfg(not(feature = "local-ml"))]
pub(crate) mod reranker;
pub(crate) mod retrieval;
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
}
