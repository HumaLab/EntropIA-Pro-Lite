/// Embedding computation via BGE-M3 providers.
///
/// Lightweight EntropIA embeddings are provider-explicit: `api` calls
/// OpenRouter `baai/bge-m3`, while `local` loads an ONNX BGE-M3 model from disk.
/// Both providers must return 1024-dimensional vectors. The engine intentionally
/// does NOT fall back to Python or fastembed; if the selected provider is not
/// configured, callers receive an explicit degraded state.
#[cfg(feature = "local-ml")]
use ndarray::{Array2, ArrayViewD, Axis};
#[cfg(feature = "local-ml")]
use ort::{
    inputs,
    session::{builder::GraphOptimizationLevel, Session},
    value::TensorRef,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
#[cfg(feature = "local-ml")]
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
#[cfg(feature = "local-ml")]
use tokenizers::Tokenizer;

use super::text_provider;

const SQLITE_BUSY_RETRY_ATTEMPTS: usize = 4;
const SQLITE_BUSY_RETRY_SLEEP_MS: u64 = 10;
const RAG_EMBEDDING_STATE_TABLE: &str = "rag_asset_embedding_state";
const RAG_REPAIR_BACKOFF_INITIAL_MS: i64 = 15 * 60 * 1000;
const RAG_REPAIR_BACKOFF_MAX_MS: i64 = 24 * 60 * 60 * 1000;
pub const EMBEDDING_PROVIDER_SETTING_KEY: &str = "embedding_provider";
pub const OPENROUTER_EMBEDDING_MODEL_SETTING_KEY: &str = "openrouter_embedding_model";
pub const LOCAL_EMBEDDING_MODEL_DIR_SETTING_KEY: &str = "local_embedding_model_dir";
#[cfg(feature = "local-ml")]
pub const LOCAL_EMBEDDING_MAX_LENGTH_SETTING_KEY: &str = "local_embedding_max_length";
pub const CANONICAL_EMBEDDING_MODEL: &str = "baai/bge-m3";
pub const CANONICAL_EMBEDDING_CONTRACT_V1: &str = "bge-m3-6000-char-weighted-mean-l2-v1";
pub const CANONICAL_EMBEDDING_DIMENSIONS: usize = 1024;
pub const DEFAULT_OPENROUTER_EMBEDDING_MODEL: &str = CANONICAL_EMBEDDING_MODEL;
pub const OPENROUTER_EMBEDDING_DIMENSIONS: usize = CANONICAL_EMBEDDING_DIMENSIONS;
/// Provider-neutral character ceiling for each BGE-M3 request.
///
/// BGE-M3 accepts at most 8192 tokens. A 6000-Unicode-scalar ceiling leaves
/// headroom for tokenization expansion without adding a tokenizer to Lite.
const EMBEDDING_CHUNK_MAX_CHARS: usize = 6000;
const RAG_CHUNK_MAX_CHARS: usize = 800;
const RAG_CHUNK_OVERLAP_CHARS: usize = 100;
pub const RAG_CHUNKING_CONTRACT_V1: &str = "rag-chunk-800-100-char-v1";

fn current_epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn is_sqlite_busy_or_locked(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(inner.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

fn retry_sqlite_busy_locked<T, F>(mut op: F) -> rusqlite::Result<T>
where
    F: FnMut() -> rusqlite::Result<T>,
{
    let mut delay_ms = SQLITE_BUSY_RETRY_SLEEP_MS;
    for attempt in 0..SQLITE_BUSY_RETRY_ATTEMPTS {
        match op() {
            Ok(value) => return Ok(value),
            Err(error)
                if is_sqlite_busy_or_locked(&error) && attempt + 1 < SQLITE_BUSY_RETRY_ATTEMPTS =>
            {
                std::thread::sleep(Duration::from_millis(delay_ms));
                delay_ms = delay_ms.saturating_mul(2);
            }
            Err(error) => return Err(error),
        }
    }

    unreachable!("retry loop must return before exhausting attempts")
}

fn next_rag_retry_at_ms(now_ms: i64, failure_count: i64) -> i64 {
    let mut backoff_ms = RAG_REPAIR_BACKOFF_INITIAL_MS;
    for _ in 1..failure_count.max(1) {
        backoff_ms = backoff_ms.saturating_mul(2);
        if backoff_ms >= RAG_REPAIR_BACKOFF_MAX_MS {
            backoff_ms = RAG_REPAIR_BACKOFF_MAX_MS;
            break;
        }
    }
    now_ms.saturating_add(backoff_ms)
}

pub(crate) fn ensure_rag_embedding_state_schema(conn: &Connection) -> Result<(), String> {
    retry_sqlite_busy_locked(|| {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {RAG_EMBEDDING_STATE_TABLE} (\
                asset_id TEXT PRIMARY KEY,\
                item_id TEXT NOT NULL,\
                rag_incomplete INTEGER NOT NULL DEFAULT 0,\
                failure_count INTEGER NOT NULL DEFAULT 0,\
                next_retry_at_ms INTEGER NOT NULL DEFAULT 0,\
                last_error TEXT,\
                updated_at_ms INTEGER NOT NULL DEFAULT 0\
            );\
            CREATE INDEX IF NOT EXISTS idx_{RAG_EMBEDDING_STATE_TABLE}_due ON {RAG_EMBEDDING_STATE_TABLE}(rag_incomplete, next_retry_at_ms);"
        ))?;
        Ok(())
    })
    .map_err(|error| format!("Failed to ensure RAG embedding state schema: {error}"))
}

fn mark_rag_embedding_incomplete_at(
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    ensure_rag_embedding_state_schema(conn)?;
    retry_sqlite_busy_locked(|| {
        conn.execute(
            &format!(
                r#"INSERT INTO {RAG_EMBEDDING_STATE_TABLE} (
                    asset_id, item_id, rag_incomplete, failure_count, next_retry_at_ms, last_error, updated_at_ms
                 ) VALUES (?1, ?2, 1, 0, 0, NULL, ?3)
                 ON CONFLICT(asset_id) DO UPDATE SET
                    item_id = excluded.item_id,
                    rag_incomplete = 1,
                    updated_at_ms = excluded.updated_at_ms"#
            ),
            params![asset_id, item_id, now_ms],
        )?;
        Ok(())
    })
    .map_err(|error| format!("Failed to mark RAG embedding incomplete for {asset_id}: {error}"))
}

fn clear_rag_embedding_state_at(conn: &Connection, asset_id: &str) -> Result<(), String> {
    ensure_rag_embedding_state_schema(conn)?;
    retry_sqlite_busy_locked(|| {
        conn.execute(
            &format!("DELETE FROM {RAG_EMBEDDING_STATE_TABLE} WHERE asset_id = ?1"),
            params![asset_id],
        )?;
        Ok(())
    })
    .map_err(|error| format!("Failed to clear RAG embedding state for {asset_id}: {error}"))
}

pub(crate) fn record_rag_embedding_failure_at(
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
    now_ms: i64,
    error: &str,
) -> Result<(), String> {
    ensure_rag_embedding_state_schema(conn)?;
    let failure_count = retry_sqlite_busy_locked(|| {
        conn.query_row(
            &format!("SELECT failure_count FROM {RAG_EMBEDDING_STATE_TABLE} WHERE asset_id = ?1"),
            params![asset_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
    })
    .map_err(|error| format!("Failed to read RAG embedding failure state for {asset_id}: {error}"))?
    .unwrap_or(0)
        + 1;
    let next_retry_at_ms = next_rag_retry_at_ms(now_ms, failure_count);

    retry_sqlite_busy_locked(|| {
        conn.execute(
            &format!(
                r#"INSERT INTO {RAG_EMBEDDING_STATE_TABLE} (
                    asset_id, item_id, rag_incomplete, failure_count, next_retry_at_ms, last_error, updated_at_ms
                 ) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6)
                 ON CONFLICT(asset_id) DO UPDATE SET
                    item_id = excluded.item_id,
                    rag_incomplete = 1,
                    failure_count = excluded.failure_count,
                    next_retry_at_ms = excluded.next_retry_at_ms,
                    last_error = excluded.last_error,
                    updated_at_ms = excluded.updated_at_ms"#
            ),
            params![asset_id, item_id, failure_count, next_retry_at_ms, error, now_ms],
        )?;
        Ok(())
    })
    .map_err(|error| format!("Failed to record RAG embedding failure for {asset_id}: {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagChunkSourceKind {
    Extraction,
    Transcription,
}

impl RagChunkSourceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Extraction => "extraction",
            Self::Transcription => "transcription",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RagChunkSource {
    pub asset_id: String,
    pub item_id: String,
    pub source_kind: RagChunkSourceKind,
    pub source_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RagChunkDraft {
    pub id: String,
    pub asset_id: String,
    pub item_id: String,
    pub source_kind: RagChunkSourceKind,
    pub source_id: String,
    pub chunk_ordinal: usize,
    pub text_content: String,
    pub start_char: usize,
    pub end_char: usize,
    pub source_text_hash: String,
    pub chunking_contract: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RagChunkEmbeddingSpec {
    pub model: &'static str,
    pub contract: &'static str,
    pub dimensions: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagChunkBackfillOutcome {
    Current,
    Replaced,
}
const OPENROUTER_EMBEDDINGS_URL: &str = "https://openrouter.ai/api/v1/embeddings";
#[cfg(feature = "local-ml")]
const DEFAULT_LOCAL_EMBEDDING_MAX_LENGTH: usize = 8192;
#[cfg(feature = "local-ml")]
const LOCAL_EMBEDDING_MODEL_FILE: &str = "model.onnx";
#[cfg(feature = "local-ml")]
const LOCAL_EMBEDDING_ONNX_DATA_FILE: &str = "model.onnx_data";
#[cfg(feature = "local-ml")]
const LOCAL_EMBEDDING_TOKENIZER_FILE: &str = "tokenizer.json";
#[cfg(feature = "local-ml")]
const BGE_M3_SOURCE_REPO: &str = "BAAI/bge-m3";
#[cfg(feature = "local-ml")]
const BGE_M3_RESOLVE_BASE_URL: &str = "https://huggingface.co/BAAI/bge-m3/resolve/main";
#[cfg(feature = "local-ml")]
const DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;
#[cfg(feature = "local-ml")]
const DOWNLOAD_TIMEOUT_SECS: u64 = 900;

#[cfg(feature = "local-ml")]
static LOCAL_ORT_INIT: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    Api,
    // In lean the variant is never constructed (no local embedding engine) but it
    // is still matched/compared (the not(local-ml) arm returns an error), so it must
    // stay in the enum — allow it to be "never constructed" rather than cfg-gating.
    #[cfg_attr(not(feature = "local-ml"), allow(dead_code))]
    Local,
}

impl EmbeddingProvider {
    fn from_setting(value: Option<&str>) -> Result<Self, String> {
        match value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            #[cfg(feature = "local-ml")]
            None | Some("local") | Some("offline") | Some("onnx") => Ok(Self::Local),
            #[cfg(not(feature = "local-ml"))]
            None => Ok(Self::Api),
            #[cfg(not(feature = "local-ml"))]
            // Lite/lean builds do not compile the local ONNX engine. Existing Pro/dev
            // databases may still carry `embedding_provider=local`; normalize those
            // legacy values to API instead of blocking the embedding worker forever.
            Some("local") | Some("offline") | Some("onnx") => Ok(Self::Api),
            Some("api") | Some("openrouter") => Ok(Self::Api),
            Some(other) => Err(format!(
                "Proveedor de embeddings no soportado: {other}. Usá 'api' o 'local'."
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEmbeddingCandidate {
    pub asset_id: String,
    pub item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEmbeddingCoverageSummary {
    pub total_assets: i64,
    pub assets_with_text: i64,
    pub assets_with_embedding: i64,
    pub assets_missing_embedding: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LocalEmbeddingModelFileInfo {
    pub filename: String,
    pub source_path: String,
    pub destination: String,
    pub size_bytes: Option<u64>,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LocalEmbeddingModelInfo {
    pub exists: bool,
    pub available: bool,
    pub can_auto_download: bool,
    pub directory: String,
    pub path: String,
    pub size_bytes: Option<u64>,
    pub required_files: Vec<LocalEmbeddingModelFileInfo>,
    pub missing_files: Vec<LocalEmbeddingModelFileInfo>,
    pub source_repo: String,
}

#[derive(Clone, serde::Serialize)]
pub struct EmbeddingDownloadProgressPayload {
    pub pct: u8,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub file: String,
}

#[derive(Clone, serde::Serialize)]
pub struct EmbeddingDownloadCompletePayload {
    pub path: String,
}

#[derive(Clone, serde::Serialize)]
pub struct EmbeddingDownloadErrorPayload {
    pub error: String,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Embedding engine configuration — resolved from app settings.
#[derive(Clone)]
pub struct EmbeddingConfig {
    /// Selected provider. `api` is OpenRouter; `local` is ONNX BGE-M3.
    pub provider: EmbeddingProvider,
    /// OpenRouter API key. Never log this value. Required only for `api`.
    pub api_key: String,
    /// Embedding model name. Defaults to `baai/bge-m3` for both providers.
    pub model_name: String,
    /// Local model directory. Defaults to app-data `models/embeddings/bge-m3`.
    #[cfg(feature = "local-ml")]
    pub local_model_dir: Option<PathBuf>,
    /// Local ONNX model path. Defaults to `<local_model_dir>/model.onnx`.
    #[cfg(feature = "local-ml")]
    pub local_model_path: Option<PathBuf>,
    /// Local tokenizer path. Defaults to `<local_model_dir>/tokenizer.json`.
    #[cfg(feature = "local-ml")]
    pub local_tokenizer_path: Option<PathBuf>,
    /// Local tokenizer/model token cap.
    #[cfg(feature = "local-ml")]
    pub local_max_length: usize,
}

impl EmbeddingConfig {
    /// Huella estable de la configuración, usada como clave del engine cacheado.
    ///
    /// La API key entra HASHEADA, no en claro: lo único que importa es si
    /// cambió, y una clave de cache no debería transportar un secreto.
    fn cache_fingerprint(&self) -> String {
        #[cfg_attr(not(feature = "local-ml"), allow(unused_mut))]
        let mut fingerprint = format!(
            "provider={:?}|model={}|key={:016x}",
            self.provider,
            self.model_name,
            rolling_hash64(self.api_key.as_bytes())
        );
        #[cfg(feature = "local-ml")]
        {
            use std::fmt::Write as _;
            let _ = write!(
                fingerprint,
                "|dir={:?}|model_path={:?}|tokenizer={:?}|max_len={}",
                self.local_model_dir,
                self.local_model_path,
                self.local_tokenizer_path,
                self.local_max_length
            );
        }
        fingerprint
    }

    #[cfg(test)]
    fn openrouter(api_key: String, model_name: String) -> Self {
        Self {
            provider: EmbeddingProvider::Api,
            api_key,
            model_name,
            #[cfg(feature = "local-ml")]
            local_model_dir: None,
            #[cfg(feature = "local-ml")]
            local_model_path: None,
            #[cfg(feature = "local-ml")]
            local_tokenizer_path: None,
            #[cfg(feature = "local-ml")]
            local_max_length: DEFAULT_LOCAL_EMBEDDING_MAX_LENGTH,
        }
    }

    #[cfg(all(test, feature = "local-ml"))]
    fn local(model_name: String, model_dir: Option<PathBuf>) -> Self {
        Self {
            provider: EmbeddingProvider::Local,
            api_key: String::new(),
            model_name,
            local_model_dir: model_dir,
            local_model_path: None,
            local_tokenizer_path: None,
            local_max_length: DEFAULT_LOCAL_EMBEDDING_MAX_LENGTH,
        }
    }
}

/// Embedding engine — dispatches to the selected BGE-M3 provider.
pub struct EmbeddingEngine {
    backend: EmbeddingBackend,
    cache: Mutex<HashMap<u64, Vec<f32>>>,
}

enum EmbeddingBackend {
    OpenRouter(OpenRouterEmbeddingClient),
    #[cfg(feature = "local-ml")]
    Local(LocalBgeM3EmbeddingEngine),
}

struct OpenRouterEmbeddingClient {
    api_key: String,
    model_name: String,
    endpoint_url: String,
}

#[cfg(feature = "local-ml")]
struct LocalBgeM3EmbeddingEngine {
    model_name: String,
    max_length: usize,
    tokenizer: Mutex<Tokenizer>,
    session: Mutex<Session>,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// Engine compartido por proceso. Un solo slot: la app tiene UNA configuración
/// de embeddings activa, así que un HashMap sería complejidad sin uso.
static ENGINE_CACHE: OnceLock<Mutex<Option<CachedEmbeddingEngine>>> = OnceLock::new();

struct CachedEmbeddingEngine {
    fingerprint: String,
    engine: Arc<EmbeddingEngine>,
}

/// Devuelve el engine compartido para `config`, inicializándolo la primera vez.
///
/// Existe porque construir el engine NO es barato: en el camino local levanta
/// una sesión ONNX de BGE-M3 y un tokenizer. Antes de esto, cada pregunta del
/// chat RAG reconstruía todo eso y descartaba además el cache interno de
/// texto→embedding del engine.
///
/// Espeja el patrón que el reranker ya usa (`cached_cross_encoder`):
/// inicialización single-flight bajo el lock, y los FALLOS NO SE CACHEAN, así
/// que instalar o reparar el modelo se recupera en la consulta siguiente.
///
/// Si el mutex quedó envenenado devuelve `Err`, que los llamadores tratan como
/// degradación (el chat sigue con FTS) en vez de propagar un panic.
pub fn get_or_init_engine(config: EmbeddingConfig) -> Result<Arc<EmbeddingEngine>, String> {
    let fingerprint = config.cache_fingerprint();
    let cache = ENGINE_CACHE.get_or_init(|| Mutex::new(None));

    let mut guard = cache
        .lock()
        .map_err(|_| "Embedding engine cache lock poisoned".to_string())?;

    if let Some(cached) = guard.as_ref() {
        if cached.fingerprint == fingerprint {
            return Ok(Arc::clone(&cached.engine));
        }
    }

    // La config cambió (o es la primera vez): reconstruimos. Un engine viejo con
    // otra huella se descarta acá, que es lo que hace que cambiar de proveedor
    // o de modelo en Configuración tenga efecto sin reiniciar la app.
    let engine = Arc::new(EmbeddingEngine::init(config)?);
    *guard = Some(CachedEmbeddingEngine {
        fingerprint,
        engine: Arc::clone(&engine),
    });
    Ok(engine)
}

impl EmbeddingEngine {
    /// Initialize the selected provider without contacting remote APIs.
    ///
    /// Prefer [`get_or_init_engine`] on hot paths: this constructor builds a
    /// fresh ONNX session every call.
    pub fn init(config: EmbeddingConfig) -> Result<Self, String> {
        match config.provider {
            EmbeddingProvider::Api => {
                Self::init_openrouter_with_endpoint(config, OPENROUTER_EMBEDDINGS_URL.to_string())
            }
            #[cfg(feature = "local-ml")]
            EmbeddingProvider::Local => Self::init_local(config),
            #[cfg(not(feature = "local-ml"))]
            EmbeddingProvider::Local => Err(
                "Embeddings locales no disponibles en este build. Configurá OpenRouter ('api') en Configuración."
                    .to_string(),
            ),
        }
    }

    fn init_openrouter_with_endpoint(
        config: EmbeddingConfig,
        endpoint_url: String,
    ) -> Result<Self, String> {
        if config.api_key.trim().is_empty() {
            return Err("OpenRouter API key no configurada para embeddings".to_string());
        }
        if config.model_name.trim().is_empty() {
            return Err("OpenRouter embedding model no configurado".to_string());
        }

        eprintln!(
            "[nlp/embeddings] OpenRouter embedding engine configured: model={}, dimensions={}",
            config.model_name, OPENROUTER_EMBEDDING_DIMENSIONS,
        );

        Ok(Self {
            backend: EmbeddingBackend::OpenRouter(OpenRouterEmbeddingClient {
                api_key: config.api_key,
                model_name: config.model_name,
                endpoint_url,
            }),
            cache: Mutex::new(HashMap::new()),
        })
    }

    #[cfg(feature = "local-ml")]
    fn init_local(config: EmbeddingConfig) -> Result<Self, String> {
        let local = LocalBgeM3EmbeddingEngine::init(&config)?;
        eprintln!(
            "[nlp/embeddings] Local BGE-M3 embedding engine configured: model={}, dimensions={}",
            local.model_name, OPENROUTER_EMBEDDING_DIMENSIONS,
        );

        Ok(Self {
            backend: EmbeddingBackend::Local(local),
            cache: Mutex::new(HashMap::new()),
        })
    }

    #[cfg(test)]
    fn init_with_endpoint(
        mut config: EmbeddingConfig,
        endpoint_url: String,
    ) -> Result<Self, String> {
        config.provider = EmbeddingProvider::Api;
        Self::init_openrouter_with_endpoint(config, endpoint_url)
    }

    /// Compute embedding for a single text string via the selected BGE-M3 provider.
    ///
    /// Returns a 1024-dimensional float vector. Errors are non-fatal —
    /// callers should treat them as degradation.
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>, String> {
        let key = rolling_hash64(text.as_bytes());
        if let Ok(cache) = self.cache.lock() {
            if let Some(hit) = cache.get(&key) {
                return Ok(hit.clone());
            }
        }

        let chunks = chunk_embedding_text(text)?;
        if chunks.len() > 1 {
            eprintln!(
                "[nlp/embeddings] text exceeded {EMBEDDING_CHUNK_MAX_CHARS} chars, splitting into {} chunks",
                chunks.len()
            );
        }

        let mut chunk_vectors = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let vector = match &self.backend {
                EmbeddingBackend::OpenRouter(client) => client.embed_chunk(chunk)?,
                #[cfg(feature = "local-ml")]
                EmbeddingBackend::Local(local) => local.embed_chunk(chunk)?,
            };
            chunk_vectors.push(vector);
        }
        let vector = aggregate_chunk_embeddings(&chunks, chunk_vectors)?;

        if let Ok(mut cache) = self.cache.lock() {
            // Tiny bounded cache to avoid repeated work/API calls for identical text.
            if cache.len() >= 128 {
                if let Some(first_key) = cache.keys().next().copied() {
                    cache.remove(&first_key);
                }
            }
            cache.insert(key, vector.clone());
        }

        Ok(vector)
    }

    pub fn provider_name(&self) -> &'static str {
        match &self.backend {
            EmbeddingBackend::OpenRouter(_) => "api",
            #[cfg(feature = "local-ml")]
            EmbeddingBackend::Local(_) => "local",
        }
    }
}

pub(crate) fn config_cache_key(config: &EmbeddingConfig) -> String {
    let api_key_hash = rolling_hash64(config.api_key.as_bytes());

    #[cfg(feature = "local-ml")]
    {
        let path_key = |path: &Option<PathBuf>| {
            path.as_ref()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default()
        };

        format!(
            "{:?}|{}|{}|{}|{}|{}|{}",
            config.provider,
            config.model_name,
            api_key_hash,
            path_key(&config.local_model_dir),
            path_key(&config.local_model_path),
            path_key(&config.local_tokenizer_path),
            config.local_max_length,
        )
    }

    #[cfg(not(feature = "local-ml"))]
    {
        format!(
            "{:?}|{}|{}",
            config.provider, config.model_name, api_key_hash,
        )
    }
}

impl OpenRouterEmbeddingClient {
    fn embed_chunk(&self, chunk: &str) -> Result<Vec<f32>, String> {
        let request = EmbeddingRequest {
            model: self.model_name.as_str(),
            input: chunk,
        };

        let client = reqwest::blocking::Client::builder()
            .user_agent("EntropIA-Desktop/0.1 (historical-research-app)")
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| format!("Failed to build OpenRouter embedding client: {e}"))?;

        let response = client
            .post(&self.endpoint_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://hlab.com.ar/")
            .header("X-Title", "EntropIA")
            .json(&request)
            .send()
            .map_err(|e| format!("OpenRouter embedding request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(format!("OpenRouter embedding API error ({status}): {body}"));
        }

        let parsed: EmbeddingResponse = response
            .json()
            .map_err(|e| format!("Failed to parse OpenRouter embedding response: {e}"))?;

        parsed
            .data
            .into_iter()
            .next()
            .map(|entry| entry.embedding)
            .ok_or_else(|| "OpenRouter embedding response returned no vectors".to_string())
    }
}

fn chunk_embedding_text(text: &str) -> Result<Vec<&str>, String> {
    if text.trim().is_empty() {
        return Err("Embedding input is empty".to_string());
    }

    let mut chunks = Vec::new();
    let mut chunk_start = 0;
    let mut chunk_chars = 0;
    for (byte_index, _) in text.char_indices() {
        if chunk_chars == EMBEDDING_CHUNK_MAX_CHARS {
            chunks.push(&text[chunk_start..byte_index]);
            chunk_start = byte_index;
            chunk_chars = 0;
        }
        chunk_chars += 1;
    }
    chunks.push(&text[chunk_start..]);
    Ok(chunks)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

pub fn canonical_rag_chunk_id(
    asset_id: &str,
    source_kind: RagChunkSourceKind,
    source_id: &str,
    chunk_ordinal: usize,
) -> String {
    let identity = format!(
        "{asset_id}\u{0}{}\u{0}{source_id}\u{0}{chunk_ordinal}",
        source_kind.as_str()
    );
    format!("ragchk-{}", sha256_hex(identity.as_bytes()))
}

pub fn plan_rag_chunks(source: &RagChunkSource) -> Result<Vec<RagChunkDraft>, String> {
    if source.text.trim().is_empty() {
        return Err("RAG chunk source text is empty".to_string());
    }

    let mut char_to_byte = source
        .text
        .char_indices()
        .map(|(byte_index, _)| byte_index)
        .collect::<Vec<_>>();
    char_to_byte.push(source.text.len());
    let char_count = char_to_byte.len() - 1;
    let source_text_hash = sha256_hex(source.text.as_bytes());
    let mut chunks = Vec::with_capacity(char_count.div_ceil(RAG_CHUNK_MAX_CHARS));
    let mut start_char = 0;

    while start_char < char_count {
        let end_char = (start_char + RAG_CHUNK_MAX_CHARS).min(char_count);
        let start_byte = char_to_byte[start_char];
        let end_byte = char_to_byte[end_char];
        let chunk_ordinal = chunks.len();
        chunks.push(RagChunkDraft {
            id: canonical_rag_chunk_id(
                &source.asset_id,
                source.source_kind,
                &source.source_id,
                chunk_ordinal,
            ),
            asset_id: source.asset_id.clone(),
            item_id: source.item_id.clone(),
            source_kind: source.source_kind,
            source_id: source.source_id.clone(),
            chunk_ordinal,
            text_content: source.text[start_byte..end_byte].to_string(),
            start_char,
            end_char,
            source_text_hash: source_text_hash.clone(),
            chunking_contract: RAG_CHUNKING_CONTRACT_V1,
        });
        if end_char == char_count {
            break;
        }
        start_char = end_char - RAG_CHUNK_OVERLAP_CHARS;
    }

    Ok(chunks)
}

pub fn backfill_rag_chunks<F>(
    conn: &Connection,
    source: &RagChunkSource,
    embedding: RagChunkEmbeddingSpec,
    mut embed: F,
) -> Result<RagChunkBackfillOutcome, String>
where
    F: FnMut(&str) -> Result<Vec<f32>, String>,
{
    let chunks = plan_rag_chunks(source)?;
    let source_kind = source.source_kind.as_str();
    let existing = {
        let mut stmt = conn
            .prepare(
                "SELECT id, item_id, chunk_ordinal, text_content, start_char, end_char,
                        source_text_hash, chunking_contract, embedding_model,
                        embedding_contract, dimensions
                 FROM rag_chunks
                 WHERE asset_id = ?1 AND source_kind = ?2 AND source_id = ?3
                 ORDER BY chunk_ordinal",
            )
            .map_err(|error| format!("Failed to prepare current RAG chunk query: {error}"))?;
        let rows = stmt
            .query_map(
                params![source.asset_id, source_kind, source.source_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                },
            )
            .map_err(|error| format!("Failed to query current RAG chunks: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to read current RAG chunks: {error}"))?
    };
    let current = existing.len() == chunks.len()
        && existing.iter().zip(&chunks).all(|(row, chunk)| {
            row.0 == chunk.id
                && row.1 == chunk.item_id
                && row.2 == chunk.chunk_ordinal as i64
                && row.3 == chunk.text_content
                && row.4 == chunk.start_char as i64
                && row.5 == chunk.end_char as i64
                && row.6 == chunk.source_text_hash
                && row.7 == chunk.chunking_contract
                && row.8 == embedding.model
                && row.9 == embedding.contract
                && row.10 == embedding.dimensions as i64
        });
    if current {
        return Ok(RagChunkBackfillOutcome::Current);
    }

    let mut blobs = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        let vector = embed(&chunk.text_content)?;
        if vector.len() != embedding.dimensions || vector.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "RAG chunk {} embedding does not satisfy {} finite dimensions",
                chunk.id, embedding.dimensions
            ));
        }
        blobs.push(floats_to_blob(&vector));
    }

    let persist = |db: &Connection| -> rusqlite::Result<()> {
        db.execute(
            "DELETE FROM rag_chunks
             WHERE asset_id = ?1 AND source_kind = ?2 AND source_id = ?3",
            params![source.asset_id, source_kind, source.source_id],
        )?;
        let mut insert = db.prepare(
            "INSERT INTO rag_chunks(
                id, asset_id, item_id, source_kind, source_id, chunk_ordinal,
                text_content, start_char, end_char, source_text_hash,
                chunking_contract, embedding, embedding_model,
                embedding_contract, dimensions
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
             )",
        )?;
        for (chunk, blob) in chunks.iter().zip(&blobs) {
            insert.execute(params![
                chunk.id,
                chunk.asset_id,
                chunk.item_id,
                source_kind,
                chunk.source_id,
                chunk.chunk_ordinal as i64,
                chunk.text_content,
                chunk.start_char as i64,
                chunk.end_char as i64,
                chunk.source_text_hash,
                chunk.chunking_contract,
                blob,
                embedding.model,
                embedding.contract,
                embedding.dimensions as i64,
            ])?;
        }
        Ok(())
    };

    retry_sqlite_busy_locked(|| {
        if conn.is_autocommit() {
            let tx = conn.unchecked_transaction()?;
            persist(&tx)?;
            tx.commit()?;
        } else {
            persist(conn)?;
        }
        Ok(())
    })
    .map_err(|error| {
        format!(
            "Failed to replace RAG chunks for {}: {error}",
            source.source_id
        )
    })?;

    Ok(RagChunkBackfillOutcome::Replaced)
}

fn aggregate_chunk_embeddings(chunks: &[&str], vectors: Vec<Vec<f32>>) -> Result<Vec<f32>, String> {
    if chunks.len() != vectors.len() {
        return Err(format!(
            "Embedding provider returned {} vectors for {} chunks",
            vectors.len(),
            chunks.len()
        ));
    }
    if chunks.is_empty() {
        return Err("Embedding preprocessing produced no chunks".to_string());
    }

    let mut weighted = vec![0.0_f64; OPENROUTER_EMBEDDING_DIMENSIONS];
    let mut total_weight = 0_usize;
    for (index, (chunk, vector)) in chunks.iter().zip(vectors).enumerate() {
        if vector.len() != OPENROUTER_EMBEDDING_DIMENSIONS {
            return Err(format!(
                "Embedding provider vector {index} returned {} dimensions; expected {}",
                vector.len(),
                OPENROUTER_EMBEDDING_DIMENSIONS
            ));
        }
        let vector = l2_normalize(vector)?;
        let weight = chunk.chars().filter(|value| !value.is_whitespace()).count();
        total_weight += weight;
        for (slot, value) in weighted.iter_mut().zip(vector) {
            *slot += value as f64 * weight as f64;
        }
    }

    if total_weight == 0 {
        return Err("Embedding input contains no non-whitespace characters".to_string());
    }

    for value in &mut weighted {
        *value /= total_weight as f64;
    }
    l2_normalize(weighted.into_iter().map(|value| value as f32).collect())
}

#[cfg(feature = "local-ml")]
impl LocalBgeM3EmbeddingEngine {
    fn init(config: &EmbeddingConfig) -> Result<Self, String> {
        let paths = resolve_local_embedding_paths(config);

        let model_dir = paths.model_path.parent().ok_or_else(|| {
            format!(
                "Local BGE-M3 model has no parent directory: {}",
                paths.model_path.display()
            )
        })?;

        if let Some(error) = local_embedding_model_incomplete_error(model_dir) {
            return Err(error);
        }

        ensure_local_ort_init(model_dir)?;

        let tokenizer = Tokenizer::from_file(&paths.tokenizer_path).map_err(|e| {
            format!(
                "Failed to load local BGE-M3 tokenizer from {}: {e}",
                paths.tokenizer_path.display()
            )
        })?;

        let session = Session::builder()
            .map_err(|e| format!("Failed to create local BGE-M3 ORT session builder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|e| format!("Failed to configure local BGE-M3 ORT optimization: {e}"))?
            .commit_from_file(&paths.model_path)
            .map_err(|e| {
                format!(
                    "Failed to load local BGE-M3 ONNX model {}: {e}",
                    paths.model_path.display()
                )
            })?;

        Ok(Self {
            model_name: config.model_name.clone(),
            max_length: config
                .local_max_length
                .clamp(8, DEFAULT_LOCAL_EMBEDDING_MAX_LENGTH),
            tokenizer: Mutex::new(tokenizer),
            session: Mutex::new(session),
        })
    }

    fn embed_chunk(&self, text: &str) -> Result<Vec<f32>, String> {
        let encoding = {
            let tokenizer = self
                .tokenizer
                .lock()
                .map_err(|_| "Local BGE-M3 tokenizer mutex poisoned".to_string())?;
            tokenizer
                .encode(text, true)
                .map_err(|e| format!("Failed to tokenize text for local BGE-M3: {e}"))?
        };

        let token_count = encoding.get_ids().len().min(self.max_length);
        if token_count == 0 {
            return Err("Local BGE-M3 tokenizer returned no tokens".to_string());
        }

        let input_ids = array_from_u32(&encoding.get_ids()[..token_count])?;
        let attention_mask = array_from_u32(&encoding.get_attention_mask()[..token_count])?;
        let type_ids = array_from_u32(&encoding.get_type_ids()[..token_count])?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| "Local BGE-M3 ONNX session mutex poisoned".to_string())?;

        let outputs = match session.inputs.len() {
            2 => session
                .run(inputs![
                    TensorRef::from_array_view(&input_ids).map_err(|e| format!(
                        "Failed to create local BGE-M3 input_ids tensor: {e}"
                    ))?,
                    TensorRef::from_array_view(&attention_mask).map_err(|e| {
                        format!("Failed to create local BGE-M3 attention_mask tensor: {e}")
                    })?,
                ])
                .map_err(|e| format!("Local BGE-M3 ONNX inference failed: {e}"))?,
            3 => session
                .run(inputs![
                    TensorRef::from_array_view(&input_ids).map_err(|e| format!(
                        "Failed to create local BGE-M3 input_ids tensor: {e}"
                    ))?,
                    TensorRef::from_array_view(&attention_mask).map_err(|e| {
                        format!("Failed to create local BGE-M3 attention_mask tensor: {e}")
                    })?,
                    TensorRef::from_array_view(&type_ids).map_err(|e| {
                        format!("Failed to create local BGE-M3 token_type_ids tensor: {e}")
                    })?,
                ])
                .map_err(|e| format!("Local BGE-M3 ONNX inference failed: {e}"))?,
            count => {
                return Err(format!(
                    "Unsupported local BGE-M3 ONNX input count: expected 2 or 3 inputs, got {count}"
                ))
            }
        };

        let output = outputs[0]
            .try_extract_array::<f32>()
            .map_err(|e| format!("Failed to extract local BGE-M3 ONNX output: {e}"))?;
        let vector = embedding_vector_from_onnx_output(output)?;

        if vector.len() != OPENROUTER_EMBEDDING_DIMENSIONS {
            return Err(format!(
                "Local BGE-M3 model '{}' returned {} dimensions; expected {}",
                self.model_name,
                vector.len(),
                OPENROUTER_EMBEDDING_DIMENSIONS,
            ));
        }

        Ok(vector)
    }
}

pub fn config_from_settings(conn: &Connection) -> Result<EmbeddingConfig, String> {
    let provider_setting = crate::settings::get_setting(conn, EMBEDDING_PROVIDER_SETTING_KEY);
    let provider = EmbeddingProvider::from_setting(provider_setting.as_deref())?;

    let api_key = crate::settings::get_setting(conn, "openrouter_api_key")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();

    let model_name = crate::settings::get_setting(conn, OPENROUTER_EMBEDDING_MODEL_SETTING_KEY)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_OPENROUTER_EMBEDDING_MODEL.to_string());

    #[cfg(feature = "local-ml")]
    let local_model_dir = if provider == EmbeddingProvider::Local {
        let configured = crate::settings::get_setting(conn, LOCAL_EMBEDDING_MODEL_DIR_SETTING_KEY);
        Some(resolve_local_embedding_model_dir(
            configured.as_deref(),
            app_data_dir_from_connection(conn).as_deref(),
        ))
    } else {
        crate::settings::get_setting(conn, LOCAL_EMBEDDING_MODEL_DIR_SETTING_KEY)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };

    #[cfg(feature = "local-ml")]
    let local_max_length =
        crate::settings::get_setting(conn, LOCAL_EMBEDDING_MAX_LENGTH_SETTING_KEY)
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_LOCAL_EMBEDDING_MAX_LENGTH)
            .clamp(8, DEFAULT_LOCAL_EMBEDDING_MAX_LENGTH);

    if provider == EmbeddingProvider::Api && api_key.is_empty() {
        #[cfg(feature = "local-ml")]
        return Err(
            "OpenRouter API key no configurada. Configurá OpenRouter para generar embeddings BGE-M3 o cambiá el proveedor a Local ONNX con el modelo BGE-M3 instalado."
                .to_string(),
        );
        #[cfg(not(feature = "local-ml"))]
        return Err(
            "OpenRouter API key no configurada. Configurá OpenRouter para generar embeddings."
                .to_string(),
        );
    }

    Ok(EmbeddingConfig {
        provider,
        api_key,
        model_name,
        #[cfg(feature = "local-ml")]
        local_model_dir,
        #[cfg(feature = "local-ml")]
        local_model_path: None,
        #[cfg(feature = "local-ml")]
        local_tokenizer_path: None,
        #[cfg(feature = "local-ml")]
        local_max_length,
    })
}

#[cfg(feature = "local-ml")]
struct LocalEmbeddingPaths {
    model_path: PathBuf,
    tokenizer_path: PathBuf,
}

#[cfg(feature = "local-ml")]
fn resolve_local_embedding_paths(config: &EmbeddingConfig) -> LocalEmbeddingPaths {
    let model_dir = config
        .local_model_dir
        .clone()
        .unwrap_or_else(default_local_embedding_model_dir);
    LocalEmbeddingPaths {
        model_path: config
            .local_model_path
            .clone()
            .unwrap_or_else(|| model_dir.join(LOCAL_EMBEDDING_MODEL_FILE)),
        tokenizer_path: config
            .local_tokenizer_path
            .clone()
            .unwrap_or_else(|| model_dir.join(LOCAL_EMBEDDING_TOKENIZER_FILE)),
    }
}

#[cfg(feature = "local-ml")]
pub fn resolve_local_embedding_model_dir(
    configured: Option<&str>,
    app_data_dir: Option<&Path>,
) -> PathBuf {
    let app_data_default = || default_local_embedding_model_dir_in_app_data(app_data_dir);
    let Some(value) = configured.map(str::trim).filter(|value| !value.is_empty()) else {
        return app_data_default();
    };

    let configured_path = PathBuf::from(value);
    if configured_path.is_absolute() {
        return configured_path;
    }

    if is_legacy_resource_embedding_dir(value) {
        return app_data_default();
    }

    match app_data_dir {
        Some(root) => root.join("models").join("embeddings").join(configured_path),
        None => configured_path,
    }
}

#[cfg(feature = "local-ml")]
pub fn get_local_embedding_model_info(model_dir: Option<PathBuf>) -> LocalEmbeddingModelInfo {
    let directory = model_dir.unwrap_or_else(default_local_embedding_model_dir);
    std::fs::create_dir_all(&directory).ok();

    let required_files = required_local_embedding_files(&directory);
    let missing_files: Vec<LocalEmbeddingModelFileInfo> = required_files
        .iter()
        .filter(|file| !file.exists)
        .cloned()
        .collect();
    let model_path = directory.join(LOCAL_EMBEDDING_MODEL_FILE);
    let size_bytes = required_files
        .iter()
        .filter_map(|file| file.size_bytes)
        .reduce(|left, right| left.saturating_add(right));
    let available = missing_files.is_empty();

    LocalEmbeddingModelInfo {
        exists: available,
        available,
        can_auto_download: true,
        directory: directory.to_string_lossy().to_string(),
        path: model_path.to_string_lossy().to_string(),
        size_bytes,
        required_files,
        missing_files,
        source_repo: BGE_M3_SOURCE_REPO.to_string(),
    }
}

#[cfg(feature = "local-ml")]
fn required_local_embedding_files(directory: &Path) -> Vec<LocalEmbeddingModelFileInfo> {
    [
        (
            LOCAL_EMBEDDING_MODEL_FILE,
            "onnx/model.onnx",
            Some(724_923_u64),
        ),
        (
            LOCAL_EMBEDDING_ONNX_DATA_FILE,
            "onnx/model.onnx_data",
            Some(2_266_820_608_u64),
        ),
        (
            LOCAL_EMBEDDING_TOKENIZER_FILE,
            "onnx/tokenizer.json",
            Some(17_082_821_u64),
        ),
    ]
    .into_iter()
    .map(|(filename, source_path, expected_size)| {
        let destination = directory.join(filename);
        let exists = destination.exists();
        let actual_size = exists
            .then(|| {
                std::fs::metadata(&destination)
                    .ok()
                    .map(|metadata| metadata.len())
            })
            .flatten();
        LocalEmbeddingModelFileInfo {
            filename: filename.to_string(),
            source_path: source_path.to_string(),
            destination: destination.to_string_lossy().to_string(),
            size_bytes: actual_size.or(expected_size),
            exists,
        }
    })
    .collect()
}

#[cfg(feature = "local-ml")]
pub fn download_local_embedding_model_files(
    model_dir: &Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    std::fs::create_dir_all(model_dir).map_err(|e| {
        format!(
            "Failed to create local BGE-M3 model directory {}: {e}",
            model_dir.display()
        )
    })?;

    let files = required_local_embedding_files(model_dir);
    let missing: Vec<_> = files.into_iter().filter(|file| !file.exists).collect();
    if missing.is_empty() {
        let _ = app_handle.emit(
            "embedding:download_complete",
            EmbeddingDownloadCompletePayload {
                path: model_dir.to_string_lossy().to_string(),
            },
        );
        return Ok(());
    }

    for file in missing {
        let url = format!(
            "{BGE_M3_RESOLVE_BASE_URL}/{}?download=true",
            file.source_path
        );
        let dest = PathBuf::from(&file.destination);
        download_local_embedding_file(&url, &dest, &file.filename, app_handle)?;
    }

    let info = get_local_embedding_model_info(Some(model_dir.to_path_buf()));
    if !info.available {
        return Err(format!(
            "Local BGE-M3 install incomplete. Missing: {}",
            info.missing_files
                .iter()
                .map(|file| file.filename.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let _ = app_handle.emit(
        "embedding:download_complete",
        EmbeddingDownloadCompletePayload {
            path: model_dir.to_string_lossy().to_string(),
        },
    );
    Ok(())
}

#[cfg(feature = "local-ml")]
fn download_local_embedding_file(
    url: &str,
    dest: &Path,
    filename: &str,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let tmp_path = dest.with_extension("download.tmp");
    let _ = std::fs::remove_file(&tmp_path);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("Failed to create BGE-M3 download client: {e}"))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to start BGE-M3 asset download for {filename}: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "BGE-M3 asset download failed for {filename} with HTTP {status}"
        ));
    }

    let total_bytes = response.content_length();
    let mut reader = response;
    let mut file = std::fs::File::create(&tmp_path).map_err(|e| {
        format!(
            "Failed to create temp BGE-M3 asset {}: {e}",
            tmp_path.display()
        )
    })?;
    let mut downloaded_bytes = 0_u64;
    let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_SIZE];
    let mut last_reported_pct = 0_u8;

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("Failed while reading BGE-M3 asset {filename}: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("Failed while writing BGE-M3 asset {filename}: {e}"))?;
        downloaded_bytes += read as u64;
        if let Some(total) = total_bytes.filter(|total| *total > 0) {
            let pct = ((downloaded_bytes.saturating_mul(100)) / total).min(100) as u8;
            if pct >= last_reported_pct.saturating_add(5) || pct == 100 {
                last_reported_pct = pct;
                let _ = app_handle.emit(
                    "embedding:download_progress",
                    EmbeddingDownloadProgressPayload {
                        pct,
                        downloaded_bytes,
                        total_bytes,
                        file: filename.to_string(),
                    },
                );
            }
        }
    }

    drop(file);
    let downloaded_size = std::fs::metadata(&tmp_path)
        .map_err(|e| format!("Failed to inspect downloaded BGE-M3 asset {filename}: {e}"))?
        .len();
    if downloaded_size == 0 {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("Downloaded BGE-M3 asset {filename} is empty"));
    }

    std::fs::rename(&tmp_path, dest).map_err(|e| {
        format!(
            "Failed to finalize BGE-M3 asset from {} to {}: {e}",
            tmp_path.display(),
            dest.display()
        )
    })
}

#[cfg(feature = "local-ml")]
fn default_local_embedding_model_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("ENTROPIA_LOCAL_EMBEDDING_MODEL_DIR") {
        return PathBuf::from(path);
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        return PathBuf::from(manifest_dir).join("resources/models/embeddings/bge-m3");
    }

    PathBuf::from("resources/models/embeddings/bge-m3")
}

#[cfg(feature = "local-ml")]
fn default_local_embedding_model_dir_in_app_data(app_data_dir: Option<&Path>) -> PathBuf {
    app_data_dir
        .map(|root| root.join("models").join("embeddings").join("bge-m3"))
        .unwrap_or_else(default_local_embedding_model_dir)
}

#[cfg(feature = "local-ml")]
fn app_data_dir_from_connection(conn: &Connection) -> Option<PathBuf> {
    let db_path: String = conn
        .query_row("PRAGMA database_list", [], |row| {
            let name: String = row.get(1)?;
            let file: String = row.get(2)?;
            Ok((name, file))
        })
        .ok()
        .and_then(|(name, file)| (name == "main" && !file.trim().is_empty()).then_some(file))?;
    PathBuf::from(db_path).parent().map(Path::to_path_buf)
}

#[cfg(feature = "local-ml")]
fn is_legacy_resource_embedding_dir(value: &str) -> bool {
    let normalized = value.trim().replace('\\', "/").to_ascii_lowercase();
    let normalized = normalized.trim_start_matches("./");
    normalized == "resources/models/embeddings/bge-m3"
        || normalized.ends_with("/resources/models/embeddings/bge-m3")
}

#[cfg(feature = "local-ml")]
pub(crate) fn ensure_local_ort_init(model_dir: &Path) -> Result<(), String> {
    if LOCAL_ORT_INIT.get().is_some() {
        return Ok(());
    }

    initialize_local_ort(model_dir.to_path_buf())?;
    let _ = LOCAL_ORT_INIT.set(());
    Ok(())
}

#[cfg(feature = "local-ml")]
fn initialize_local_ort(model_dir: PathBuf) -> Result<(), String> {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        ort::init()
            .commit()
            .map_err(|e| format!("Failed to initialize ORT from ORT_DYLIB_PATH: {e}"))?;
        return Ok(());
    }

    let dylib_path = find_ort_dylib(&model_dir).ok_or_else(|| {
        format!(
            "No ONNX Runtime dynamic library found near local BGE-M3 model directory {}. Expected onnxruntime.dll / libonnxruntime.* or set ORT_DYLIB_PATH.",
            model_dir.display()
        )
    })?;

    ort::init_from(dylib_path.display().to_string())
        .commit()
        .map_err(|e| {
            format!(
                "Failed to initialize ORT from {}: {e}",
                dylib_path.display()
            )
        })?;

    Ok(())
}

#[cfg(feature = "local-ml")]
fn find_ort_dylib(model_dir: &Path) -> Option<PathBuf> {
    runtime_candidates(model_dir)
        .into_iter()
        .find(|path| path.exists())
}

#[cfg(feature = "local-ml")]
fn runtime_candidates(model_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push_names = |base: &Path| {
        for name in runtime_file_names() {
            candidates.push(base.join(name));
        }
    };

    push_names(model_dir);
    if let Some(parent) = model_dir.parent() {
        push_names(parent);
        // Reuse the existing app-local ORT DLL when BGE-M3 lives in
        // resources/models/embeddings and ORT is bundled in a sibling model dir.
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    push_names(&path);
                }
            }
        }
    }

    if let Some(app_data_root) = app_data_root_from_local_model_dir(model_dir) {
        let runtime_root = app_data_root.join("runtime");
        if let Ok(entries) = std::fs::read_dir(&runtime_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    push_names(&path.join("resources").join("lib"));
                    for capi_dir in managed_venv_onnxruntime_capi_dirs(&path) {
                        push_names(&capi_dir);
                    }
                }
            }
        }
        push_names(&app_data_root.join("resources").join("lib"));

        // Dev-fallback venv (runtime-dev/system-python): the deps installer puts
        // onnxruntime here (pulled by faster-whisper / paddleocr), so reuse it
        // when the managed runtime-pack is not yet hydrated. ONNX Runtime keeps a
        // stable C ABI across 1.x, so a newer dll loads fine under ort rc.10.
        let dev_venv = app_data_root.join("runtime-dev").join("system-python");
        if cfg!(windows) {
            push_names(
                &dev_venv
                    .join("Lib")
                    .join("site-packages")
                    .join("onnxruntime")
                    .join("capi"),
            );
        } else if let Ok(entries) = std::fs::read_dir(dev_venv.join("lib")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    push_names(&path.join("site-packages").join("onnxruntime").join("capi"));
                }
            }
        }
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_dir = PathBuf::from(manifest_dir);
        push_names(&manifest_dir.join("resources").join("lib"));
        push_names(&manifest_dir.join("resources").join("models").join("ner"));
    }

    candidates
}

#[cfg(feature = "local-ml")]
fn managed_venv_onnxruntime_capi_dirs(managed_root: &Path) -> Vec<PathBuf> {
    let venv = managed_root.join("venv").join("entropia-env");
    if cfg!(windows) {
        return vec![venv
            .join("Lib")
            .join("site-packages")
            .join("onnxruntime")
            .join("capi")];
    }

    let mut candidates = Vec::new();
    let lib_dir = venv.join("lib");
    if let Ok(entries) = std::fs::read_dir(&lib_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                candidates.push(path.join("site-packages").join("onnxruntime").join("capi"));
            }
        }
    } else {
        candidates.push(
            lib_dir
                .join("site-packages")
                .join("onnxruntime")
                .join("capi"),
        );
    }

    candidates
}

#[cfg(feature = "local-ml")]
fn app_data_root_from_local_model_dir(model_dir: &Path) -> Option<PathBuf> {
    let category_dir = model_dir.parent()?;
    let models_dir = category_dir.parent()?;
    if !models_dir
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("models")
    {
        return None;
    }

    models_dir.parent().map(Path::to_path_buf)
}

#[cfg(feature = "local-ml")]
fn runtime_file_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["onnxruntime.dll"]
    }

    #[cfg(target_os = "linux")]
    {
        &["libonnxruntime.so", "libonnxruntime.so.1"]
    }

    #[cfg(target_os = "macos")]
    {
        &["libonnxruntime.dylib"]
    }
}

#[cfg(feature = "local-ml")]
fn array_from_u32(values: &[u32]) -> Result<Array2<i64>, String> {
    Array2::from_shape_vec(
        (1, values.len()),
        values.iter().map(|value| *value as i64).collect(),
    )
    .map_err(|e| format!("Failed to build local BGE-M3 ONNX input tensor: {e}"))
}

#[cfg(feature = "local-ml")]
fn embedding_vector_from_onnx_output(output: ArrayViewD<'_, f32>) -> Result<Vec<f32>, String> {
    let shape = output.shape();
    match shape {
        [dim] if *dim == OPENROUTER_EMBEDDING_DIMENSIONS => Ok(output.iter().copied().collect()),
        [batch, dim] if *batch == 1 && *dim == OPENROUTER_EMBEDDING_DIMENSIONS => {
            Ok(output.iter().copied().collect())
        }
        [batch, tokens, hidden]
            if *batch == 1 && *tokens > 0 && *hidden == OPENROUTER_EMBEDDING_DIMENSIONS =>
        {
            let batch = output.index_axis(Axis(0), 0);
            let cls = batch.index_axis(Axis(0), 0);
            Ok(cls.iter().copied().collect())
        }
        _ => Err(format!(
            "Unexpected local BGE-M3 ONNX output shape: {shape:?}; expected [1024], [1,1024], or [1,tokens,1024]"
        )),
    }
}

fn l2_normalize(mut vector: Vec<f32>) -> Result<Vec<f32>, String> {
    if vector.iter().any(|value| !value.is_finite()) {
        return Err("Embedding provider returned a non-finite vector".to_string());
    }

    let norm = vector
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt();

    if !norm.is_finite() || norm <= f64::EPSILON {
        return Err("Embedding provider returned a zero or invalid vector".to_string());
    }

    for value in &mut vector {
        *value = (*value as f64 / norm) as f32;
    }

    Ok(vector)
}

// ── OFF-arm (lean) local-embedding stubs — keep command surface compiling ──
// Names + signatures match the local-ml impls above so nlp/commands.rs and
// rag/commands.rs build identically; behavior degrades to "configure OpenRouter".
// LocalEmbeddingModelInfo / EmbeddingDownloadErrorPayload are already declared
// ungated earlier in this file, so these stubs name existing types only.

#[cfg(not(feature = "local-ml"))]
pub fn resolve_local_embedding_model_dir(
    configured: Option<&str>,
    app_data_dir: Option<&Path>,
) -> PathBuf {
    configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            app_data_dir
                .map(|root| root.join("models").join("embeddings").join("bge-m3"))
                .unwrap_or_else(|| PathBuf::from("models/embeddings/bge-m3"))
        })
}

#[cfg(not(feature = "local-ml"))]
pub fn get_local_embedding_model_info(model_dir: Option<PathBuf>) -> LocalEmbeddingModelInfo {
    let directory = model_dir.unwrap_or_else(|| PathBuf::from("models/embeddings/bge-m3"));
    LocalEmbeddingModelInfo {
        exists: false,
        available: false,
        can_auto_download: false,
        directory: directory.to_string_lossy().to_string(),
        path: String::new(),
        size_bytes: None,
        required_files: Vec::new(),
        missing_files: Vec::new(),
        source_repo: "remote-openrouter".to_string(),
    }
}

#[cfg(not(feature = "local-ml"))]
pub fn download_local_embedding_model_files(
    _model_dir: &Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let error = "Descarga de modelo local no disponible en este build. Configurá OpenRouter en Configuración para usar embeddings remotos."
        .to_string();
    let _ = app_handle.emit(
        "embedding:download_error",
        EmbeddingDownloadErrorPayload {
            error: error.clone(),
        },
    );
    Err(error)
}

pub fn backfill_asset_rag_chunks(
    engine: &EmbeddingEngine,
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
) -> Result<Vec<RagChunkBackfillOutcome>, String> {
    let sources = {
        let mut stmt = conn
            .prepare(
                "SELECT id, 'extraction', text_content
                 FROM extractions
                 WHERE asset_id = ?1 AND LENGTH(TRIM(COALESCE(text_content, ''))) > 0
                 UNION ALL
                 SELECT id, 'transcription', text_content
                 FROM transcriptions
                 WHERE asset_id = ?1 AND LENGTH(TRIM(COALESCE(text_content, ''))) > 0
                 ORDER BY 2, 1",
            )
            .map_err(|error| format!("Failed to prepare asset RAG source query: {error}"))?;
        let rows = stmt
            .query_map(params![asset_id], |row| {
                let kind: String = row.get(1)?;
                Ok(RagChunkSource {
                    asset_id: asset_id.to_string(),
                    item_id: item_id.to_string(),
                    source_kind: if kind == "extraction" {
                        RagChunkSourceKind::Extraction
                    } else {
                        RagChunkSourceKind::Transcription
                    },
                    source_id: row.get(0)?,
                    text: row.get(2)?,
                })
            })
            .map_err(|error| format!("Failed to query asset RAG sources: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to read asset RAG sources: {error}"))?
    };
    if sources.is_empty() {
        return Err(format!(
            "No source text available for asset '{asset_id}' (run OCR/transcription first)"
        ));
    }

    let embedding = RagChunkEmbeddingSpec {
        model: CANONICAL_EMBEDDING_MODEL,
        contract: CANONICAL_EMBEDDING_CONTRACT_V1,
        dimensions: CANONICAL_EMBEDDING_DIMENSIONS,
    };
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("Failed to start asset RAG chunk backfill: {error}"))?;
    let mut outcomes = Vec::with_capacity(sources.len());
    for source in &sources {
        outcomes.push(backfill_rag_chunks(&tx, source, embedding, |text| {
            engine.embed_text(text)
        })?);
    }
    let current_sources = sources
        .iter()
        .map(|source| (source.source_kind.as_str(), source.source_id.as_str()))
        .collect::<std::collections::HashSet<_>>();
    let persisted_sources = {
        let mut stmt = tx
            .prepare(
                "SELECT DISTINCT source_kind, source_id
                 FROM rag_chunks
                 WHERE asset_id = ?1",
            )
            .map_err(|error| format!("Failed to prepare stale RAG source query: {error}"))?;
        let rows = stmt
            .query_map(params![asset_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("Failed to query stale RAG sources: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to read stale RAG sources: {error}"))?
    };
    for (source_kind, source_id) in persisted_sources {
        if !current_sources.contains(&(source_kind.as_str(), source_id.as_str())) {
            retry_sqlite_busy_locked(|| {
                tx.execute(
                    "DELETE FROM rag_chunks
                     WHERE asset_id = ?1 AND source_kind = ?2 AND source_id = ?3",
                    params![asset_id, source_kind, source_id],
                )?;
                Ok(())
            })
            .map_err(|error| format!("Failed to remove orphaned RAG chunk source: {error}"))?;
        }
    }
    tx.commit()
        .map_err(|error| format!("Failed to commit asset RAG chunk backfill: {error}"))?;
    Ok(outcomes)
}

/// Compute embedding for a single asset's text and store it.
///
/// Uses only the extraction/transcription text for the given `asset_id`,
/// not the entire item. The embedding is stored under `asset_id` in
/// `vec_assets`.
pub fn compute_and_store_for_asset(
    engine: Option<&EmbeddingEngine>,
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
) -> Result<(), String> {
    compute_and_store_for_asset_with_unavailable_reason(engine, conn, item_id, asset_id, None)
}

pub fn compute_and_store_for_asset_with_unavailable_reason(
    engine: Option<&EmbeddingEngine>,
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
    unavailable_reason: Option<&str>,
) -> Result<(), String> {
    let text = text_provider::get_asset_text(conn, asset_id)?;
    if text.trim().is_empty() {
        return Err(format!(
            "No source text available for asset '{asset_id}' (run OCR/transcription first)"
        ));
    }

    let engine = match engine {
        Some(e) => e,
        None => {
            return Err(embedding_degradation_log(
                item_id,
                &embedding_engine_unavailable_reason(unavailable_reason),
            ));
        }
    };

    let provider = engine.provider_name();
    eprintln!(
        "[nlp/embeddings] EMBED start provider={provider} item_id={item_id} asset_id={asset_id} chars={}",
        text.chars().count()
    );

    let vector = match engine.embed_text(&text) {
        Ok(v) => {
            eprintln!(
                "[nlp/embeddings] EMBED computed provider={provider} item_id={item_id} asset_id={asset_id} dims={}",
                v.len()
            );
            v
        }
        Err(e) => {
            eprintln!(
                "[nlp/embeddings] EMBED error provider={provider} item_id={item_id} asset_id={asset_id}: {e}"
            );
            return Err(embedding_degradation_log(item_id, &e));
        }
    };

    let blob = floats_to_blob(&vector);
    persist_asset_embedding_with_rag_state_at(
        conn,
        item_id,
        asset_id,
        &blob,
        current_epoch_millis(),
        |db, item_id, asset_id| {
            backfill_asset_rag_chunks(engine, db, item_id, asset_id).map(|_| ())
        },
    )?;
    eprintln!(
        "[nlp/embeddings] EMBED persisted provider={provider} item_id={item_id} asset_id={asset_id} bytes={}",
        blob.len()
    );
    Ok(())
}

pub fn embedding_engine_unavailable_reason(last_init_error: Option<&str>) -> String {
    match last_init_error.map(str::trim).filter(|value| !value.is_empty()) {
        Some(error) => format!(
            "No BGE-M3 embedding engine configured. Last initialization error: {error}"
        ),
        None => "No BGE-M3 embedding engine configured. Set OpenRouter API credentials for the api provider or install/select the local BGE-M3 ONNX assets for the local provider.".to_string(),
    }
}

pub fn summarize_asset_embedding_coverage(
    conn: &Connection,
) -> Result<AssetEmbeddingCoverageSummary, String> {
    conn.query_row(
        r#"
        WITH asset_text AS (
            SELECT
                a.id AS asset_id,
                EXISTS(
                    SELECT 1
                    FROM extractions e
                    WHERE e.asset_id = a.id
                      AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0
                )
                OR EXISTS(
                    SELECT 1
                    FROM transcriptions t
                    WHERE t.asset_id = a.id
                      AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0
                ) AS has_text,
                EXISTS(
                    SELECT 1
                    FROM vec_assets v
                    WHERE v.asset_id = a.id
                      AND v.embedding_model = ?1
                      AND v.embedding_contract = ?2
                      AND v.dimensions = ?3
                ) AS has_embedding
            FROM assets a
        )
        SELECT
            COUNT(*) AS total_assets,
            SUM(CASE WHEN has_text THEN 1 ELSE 0 END) AS assets_with_text,
            SUM(CASE WHEN has_embedding THEN 1 ELSE 0 END) AS assets_with_embedding,
            SUM(CASE WHEN has_text AND NOT has_embedding THEN 1 ELSE 0 END) AS assets_missing_embedding
        FROM asset_text
        "#,
        params![
            CANONICAL_EMBEDDING_MODEL,
            CANONICAL_EMBEDDING_CONTRACT_V1,
            CANONICAL_EMBEDDING_DIMENSIONS as i64
        ],
        |row| {
            Ok(AssetEmbeddingCoverageSummary {
                total_assets: row.get(0)?,
                assets_with_text: row.get(1)?,
                assets_with_embedding: row.get(2)?,
                assets_missing_embedding: row.get(3)?,
            })
        },
    )
    .map_err(|e| format!("Failed to summarize asset embedding coverage: {e}"))
}

pub(crate) fn list_asset_embedding_candidates_at(
    conn: &Connection,
    force: bool,
    limit: Option<usize>,
    now_ms: i64,
) -> Result<Vec<AssetEmbeddingCandidate>, String> {
    ensure_rag_embedding_state_schema(conn)?;

    let mut sql = String::from(
        r#"
        SELECT a.id, a.item_id
        FROM assets a
        WHERE (
            EXISTS(
                SELECT 1
                FROM extractions e
                WHERE e.asset_id = a.id
                  AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0
            )
            OR EXISTS(
                SELECT 1
                FROM transcriptions t
                WHERE t.asset_id = a.id
                  AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0
            )
        )
        AND (
            ?1 = 1
            OR EXISTS(
                SELECT 1
                FROM rag_asset_embedding_state s
                WHERE s.asset_id = a.id
                  AND s.rag_incomplete = 1
                  AND s.next_retry_at_ms <= ?5
            )
            OR (
                NOT EXISTS(
                    SELECT 1
                    FROM vec_assets v
                    WHERE v.asset_id = a.id
                      AND v.embedding_model = ?2
                      AND v.embedding_contract = ?3
                      AND v.dimensions = ?4
                )
                AND NOT EXISTS(
                    SELECT 1
                    FROM rag_asset_embedding_state s
                    WHERE s.asset_id = a.id
                      AND s.rag_incomplete = 1
                )
            )
        )
        ORDER BY a.created_at ASC, a.id ASC
        "#,
    );

    if let Some(limit) = limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }

    let rows = retry_sqlite_busy_locked(|| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![
                if force { 1_i64 } else { 0_i64 },
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS as i64,
                now_ms,
            ],
            |row| {
                Ok(AssetEmbeddingCandidate {
                    asset_id: row.get(0)?,
                    item_id: row.get(1)?,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .map_err(|e| format!("Failed to query asset embedding backfill candidates: {e}"))?;

    Ok(rows)
}

pub fn list_asset_embedding_candidates(
    conn: &Connection,
    force: bool,
    limit: Option<usize>,
) -> Result<Vec<AssetEmbeddingCandidate>, String> {
    list_asset_embedding_candidates_at(conn, force, limit, current_epoch_millis())
}

fn upsert_vec_asset(
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
    blob: &[u8],
) -> Result<(), String> {
    retry_sqlite_busy_locked(|| {
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(asset_id) DO UPDATE SET item_id=excluded.item_id, embedding=excluded.embedding, embedding_model=excluded.embedding_model, embedding_contract=excluded.embedding_contract, dimensions=excluded.dimensions",
            params![
                asset_id,
                item_id,
                blob,
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS as i64
            ],
        )?;
        Ok(())
    })
    .map_err(|error| {
        format!(
            "[nlp/embeddings] Failed to persist asset embedding for {asset_id}: {error}"
        )
    })
}

fn persist_asset_embedding_with_rag_state_at<F>(
    conn: &Connection,
    item_id: &str,
    asset_id: &str,
    blob: &[u8],
    now_ms: i64,
    mut backfill_rag_chunks: F,
) -> Result<(), String>
where
    F: FnMut(&Connection, &str, &str) -> Result<(), String>,
{
    ensure_rag_embedding_state_schema(conn)?;
    mark_rag_embedding_incomplete_at(conn, item_id, asset_id, now_ms)?;
    upsert_vec_asset(conn, item_id, asset_id, blob)?;
    backfill_rag_chunks(conn, item_id, asset_id)?;
    clear_rag_embedding_state_at(conn, asset_id)?;
    Ok(())
}

fn rolling_hash64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Serialize `Vec<f32>` to little-endian bytes for sqlite-vec BLOB storage.
fn floats_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn embedding_degradation_log(item_id: &str, reason: &str) -> String {
    format!("[nlp/embeddings] Skipping embedding for {item_id}: {reason}")
}

#[cfg(feature = "local-ml")]
fn local_embedding_model_incomplete_error(model_dir: &Path) -> Option<String> {
    let info = get_local_embedding_model_info(Some(model_dir.to_path_buf()));
    if info.available {
        return None;
    }

    let missing = info
        .missing_files
        .iter()
        .map(|file| {
            format!(
                "{} (expected at {})",
                file.filename,
                PathBuf::from(&file.destination).display()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    Some(format!(
        "Local BGE-M3 model incomplete at {}. Missing required files: {missing}. Install BGE-M3 from Settings or place all required files ({LOCAL_EMBEDDING_MODEL_FILE}, {LOCAL_EMBEDDING_ONNX_DATA_FILE}, {LOCAL_EMBEDDING_TOKENIZER_FILE}) in that folder. The EMBED action only uses the configured BGE-M3 provider.",
        info.directory
    ))
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn sqlite_failure(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
    }

    #[test]
    fn retry_sqlite_busy_locked_retries_busy_then_locked_then_succeeds() {
        let mut attempts = 0;

        let value = retry_sqlite_busy_locked(|| {
            attempts += 1;
            match attempts {
                1 => Err(sqlite_failure(rusqlite::ffi::SQLITE_BUSY)),
                2 => Err(sqlite_failure(rusqlite::ffi::SQLITE_LOCKED)),
                _ => Ok("success"),
            }
        })
        .expect("busy and locked failures should be retried");

        assert_eq!(value, "success");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn retry_sqlite_busy_locked_stops_after_locked_exhaustion() {
        let mut attempts = 0;

        let error = retry_sqlite_busy_locked(|| {
            attempts += 1;
            Err::<(), _>(sqlite_failure(rusqlite::ffi::SQLITE_LOCKED))
        })
        .expect_err("locked failure should surface after finite retries");

        assert_eq!(attempts, SQLITE_BUSY_RETRY_ATTEMPTS);
        assert!(matches!(
            error,
            rusqlite::Error::SqliteFailure(inner, None)
                if inner.code == rusqlite::ErrorCode::DatabaseLocked
        ));
    }

    #[test]
    fn retry_sqlite_busy_locked_does_not_retry_non_lock_errors() {
        let mut attempts = 0;

        let error = retry_sqlite_busy_locked(|| {
            attempts += 1;
            Err::<(), _>(rusqlite::Error::InvalidQuery)
        })
        .expect_err("non-lock failure should surface immediately");

        assert_eq!(attempts, 1);
        assert!(matches!(error, rusqlite::Error::InvalidQuery));
    }

    #[test]
    fn floats_to_blob_produces_correct_byte_count() {
        let v = vec![1.0_f32, 2.0_f32, 3.0_f32];
        let blob = floats_to_blob(&v);
        assert_eq!(blob.len(), 3 * 4, "Each f32 should produce 4 bytes");
    }

    #[test]
    fn floats_to_blob_round_trips_correctly() {
        let original = vec![1.5_f32, -0.5_f32, 100.0_f32];
        let blob = floats_to_blob(&original);
        let recovered: Vec<f32> = blob
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(recovered, original);
    }

    #[test]
    fn empty_vec_produces_empty_blob() {
        let blob = floats_to_blob(&[]);
        assert!(blob.is_empty());
    }

    #[test]
    fn embedding_degradation_log_includes_item_id_and_reason() {
        let message = embedding_degradation_log("item-42", "No embedding engine configured");
        assert!(
            message.contains("item-42"),
            "log message must include item id for operational diagnosis"
        );
        assert!(
            message.contains("No embedding engine configured"),
            "log message must include degradation reason"
        );
    }

    #[test]
    fn embedding_degradation_log_keeps_expected_prefix_for_grepability() {
        let message = embedding_degradation_log("item-99", "OpenRouter embedding failed");
        assert!(
            message.starts_with("[nlp/embeddings] Skipping embedding for "),
            "log message prefix should remain stable for observability tooling"
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn config_from_settings_defaults_to_local_bge_m3() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('openrouter_api_key', 'sk-test');",
        )
        .expect("settings table should be created");

        let config = config_from_settings(&conn).expect("config should resolve");

        assert_eq!(config.provider, EmbeddingProvider::Local);
        assert_eq!(config.model_name, DEFAULT_OPENROUTER_EMBEDDING_MODEL);
    }

    #[test]
    fn config_from_settings_allows_embedding_model_override() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('openrouter_api_key', 'sk-test');\
             INSERT INTO app_settings(key, value) VALUES ('openrouter_embedding_model', 'custom/model');",
        )
        .expect("settings table should be created");

        let config = config_from_settings(&conn).expect("config should resolve");

        assert_eq!(config.model_name, "custom/model");
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn config_from_settings_allows_local_provider_without_openrouter_key() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'local');\
             INSERT INTO app_settings(key, value) VALUES ('local_embedding_model_dir', 'C:/models/bge-m3');",
        )
        .expect("settings table should be created");

        let config = config_from_settings(&conn).expect("local config should not require API key");

        assert_eq!(config.provider, EmbeddingProvider::Local);
        assert_eq!(config.model_name, DEFAULT_OPENROUTER_EMBEDDING_MODEL);
        assert_eq!(
            config.local_model_dir,
            Some(PathBuf::from("C:/models/bge-m3"))
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn config_from_settings_uses_app_data_default_for_local_provider_when_dir_is_empty() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp.path().join("entropia.sqlite");
        let conn = Connection::open(&db_path).expect("sqlite file should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'local');",
        )
        .expect("settings table should be created");

        let config = config_from_settings(&conn).expect("local config should resolve");

        assert_eq!(
            config.local_model_dir,
            Some(temp.path().join("models").join("embeddings").join("bge-m3"))
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn config_from_settings_ignores_legacy_resource_relative_dir_for_local_provider() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp.path().join("entropia.sqlite");
        let conn = Connection::open(&db_path).expect("sqlite file should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'local');\
             INSERT INTO app_settings(key, value) VALUES ('local_embedding_model_dir', 'resources/models/embeddings/bge-m3');",
        )
        .expect("settings table should be created");

        let config = config_from_settings(&conn).expect("local config should resolve");

        assert_eq!(
            config.local_model_dir,
            Some(temp.path().join("models").join("embeddings").join("bge-m3"))
        );
    }

    #[test]
    fn config_from_settings_rejects_unknown_embedding_provider() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'mystery');",
        )
        .expect("settings table should be created");

        let error = match config_from_settings(&conn) {
            Ok(_) => panic!("unknown provider should fail"),
            Err(error) => error,
        };

        assert!(error.contains("Proveedor de embeddings no soportado"));
        assert!(error.contains("api"));
        assert!(error.contains("local"));
    }

    #[test]
    fn config_from_settings_requires_openrouter_key_when_api_provider_is_selected() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'api');",
        )
        .expect("settings table should be created");

        let error = match config_from_settings(&conn) {
            Ok(_) => panic!("missing key should fail"),
            Err(error) => error,
        };

        assert!(error.contains("OpenRouter API key"));
        #[cfg(feature = "local-ml")]
        assert!(error.contains("Local ONNX"));
        #[cfg(not(feature = "local-ml"))]
        assert!(!error.contains("Local ONNX"));
    }

    #[cfg(not(feature = "local-ml"))]
    #[test]
    fn config_from_settings_normalizes_legacy_local_provider_to_api_in_lean_build() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'local');\
             INSERT INTO app_settings(key, value) VALUES ('openrouter_api_key', 'sk-test');",
        )
        .expect("settings table should be created");

        let config = config_from_settings(&conn).expect("legacy local provider should normalize");

        assert_eq!(config.provider, EmbeddingProvider::Api);
        assert_eq!(config.model_name, DEFAULT_OPENROUTER_EMBEDDING_MODEL);
    }

    #[cfg(not(feature = "local-ml"))]
    #[test]
    fn config_from_settings_lean_missing_api_key_does_not_suggest_local_provider() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'local');",
        )
        .expect("settings table should be created");

        let error = match config_from_settings(&conn) {
            Ok(_) => panic!("missing API key should fail"),
            Err(error) => error,
        };

        assert!(error.contains("OpenRouter API key"));
        assert!(!error.contains("Local ONNX"));
    }

    #[tokio::test]
    async fn init_can_drop_embedding_engine_inside_tokio_context() {
        let engine = EmbeddingEngine::init_with_endpoint(
            EmbeddingConfig::openrouter(
                "sk-test".to_string(),
                DEFAULT_OPENROUTER_EMBEDDING_MODEL.to_string(),
            ),
            "http://127.0.0.1:9".to_string(),
        )
        .expect("engine init should not create a blocking runtime");

        drop(engine);
    }

    // ── Engine cache (F2) ────────────────────────────────────────────────────

    /// `ENGINE_CACHE` es un único slot global: cualquier test que lo toque
    /// desaloja al de al lado. Rust corre los tests en paralelo, así que los
    /// que observan identidad del engine tienen que serializarse.
    static ENGINE_CACHE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn engine_cache_returns_the_same_instance_for_an_unchanged_config() {
        let _serialized = ENGINE_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let config = EmbeddingConfig::openrouter("sk-test".to_string(), "baai/bge-m3".to_string());

        let first = get_or_init_engine(config.clone()).expect("first init should succeed");
        let second = get_or_init_engine(config).expect("second init should succeed");

        assert!(
            Arc::ptr_eq(&first, &second),
            "the engine must be reused, not rebuilt per query"
        );
    }

    #[test]
    fn engine_cache_rebuilds_when_the_config_changes() {
        let _serialized = ENGINE_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let first = get_or_init_engine(EmbeddingConfig::openrouter(
            "sk-test".to_string(),
            "baai/bge-m3".to_string(),
        ))
        .expect("first init should succeed");

        // Cambiar de modelo en Configuración debe tener efecto sin reiniciar.
        let second = get_or_init_engine(EmbeddingConfig::openrouter(
            "sk-test".to_string(),
            "baai/bge-m3-other".to_string(),
        ))
        .expect("second init should succeed");

        assert!(
            !Arc::ptr_eq(&first, &second),
            "a changed config must not keep serving the previous engine"
        );
    }

    #[test]
    fn engine_cache_failures_are_not_cached() {
        let _serialized = ENGINE_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Una config inválida (sin API key) falla; arreglarla debe funcionar en
        // el intento siguiente, así que el fallo no puede quedar memoizado.
        let broken = EmbeddingConfig::openrouter(String::new(), "baai/bge-m3".to_string());
        assert!(get_or_init_engine(broken).is_err());

        let repaired =
            EmbeddingConfig::openrouter("sk-repaired".to_string(), "baai/bge-m3".to_string());
        assert!(
            get_or_init_engine(repaired).is_ok(),
            "a repaired config must initialize instead of replaying the failure"
        );
    }

    #[test]
    fn cache_fingerprint_never_contains_the_api_key_in_plaintext() {
        let secret = "sk-super-secret-value";
        let fingerprint =
            EmbeddingConfig::openrouter(secret.to_string(), "baai/bge-m3".to_string())
                .cache_fingerprint();

        assert!(
            !fingerprint.contains(secret),
            "the cache key must not carry the secret: {fingerprint}"
        );
        // Pero sigue distinguiendo claves distintas.
        let other = EmbeddingConfig::openrouter("sk-other".to_string(), "baai/bge-m3".to_string())
            .cache_fingerprint();
        assert_ne!(fingerprint, other);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn init_local_provider_reports_missing_bge_m3_assets() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let error = match EmbeddingEngine::init(EmbeddingConfig::local(
            DEFAULT_OPENROUTER_EMBEDDING_MODEL.to_string(),
            Some(temp.path().to_path_buf()),
        )) {
            Ok(_) => panic!("local provider should fail when ONNX assets are absent"),
            Err(error) => error,
        };

        assert!(error.contains("Local BGE-M3 model incomplete"));
        assert!(error.contains(&temp.path().to_string_lossy().to_string()));
        assert!(error.contains(LOCAL_EMBEDDING_MODEL_FILE));
        assert!(error.contains(LOCAL_EMBEDDING_ONNX_DATA_FILE));
        assert!(error.contains(LOCAL_EMBEDDING_TOKENIZER_FILE));
        assert!(error.contains("Install BGE-M3 from Settings"));
        assert!(error.contains("configured BGE-M3 provider"));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn compute_and_store_for_asset_reports_last_local_init_error_when_engine_missing() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            r#"
            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE transcriptions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT NOT NULL,
              language TEXT,
              duration_ms INTEGER,
              model TEXT NOT NULL,
              segments TEXT,
              confidence REAL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE vec_assets (
              asset_id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              embedding BLOB NOT NULL
            );
            "#,
        )
        .expect("schema should be created");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES ('asset-local', 'item-local', 'local.txt', 'txt', 1)",
            [],
        )
        .expect("asset should insert");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES ('ext-local', 'asset-local', 'texto para embedding', 2)",
            [],
        )
        .expect("extraction should insert");

        let error = compute_and_store_for_asset_with_unavailable_reason(
            None,
            &conn,
            "item-local",
            "asset-local",
            Some("Local BGE-M3 model incomplete at C:/Users/test/AppData/Roaming/com.entropia.desktop/models/embeddings/bge-m3. Missing required files: model.onnx, model.onnx_data, tokenizer.json. Install BGE-M3 from Settings."),
        )
        .expect_err("missing engine should surface remembered initialization error");

        assert!(error.contains("item-local"));
        assert!(error.contains("Local BGE-M3 model incomplete"));
        assert!(error.contains(
            "C:/Users/test/AppData/Roaming/com.entropia.desktop/models/embeddings/bge-m3"
        ));
        assert!(error.contains(LOCAL_EMBEDDING_MODEL_FILE));
        assert!(error.contains(LOCAL_EMBEDDING_ONNX_DATA_FILE));
        assert!(error.contains(LOCAL_EMBEDDING_TOKENIZER_FILE));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn local_embedding_model_info_requires_onnx_external_data_and_tokenizer() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        std::fs::write(temp.path().join(LOCAL_EMBEDDING_MODEL_FILE), b"onnx")
            .expect("model file should be writable");
        std::fs::write(
            temp.path().join(LOCAL_EMBEDDING_TOKENIZER_FILE),
            b"tokenizer",
        )
        .expect("tokenizer should be writable");

        let info = get_local_embedding_model_info(Some(temp.path().to_path_buf()));

        assert!(
            !info.available,
            "external ONNX data file is required by BAAI/bge-m3"
        );
        assert_eq!(info.required_files.len(), 3);
        assert!(info
            .missing_files
            .iter()
            .any(|file| file.filename == LOCAL_EMBEDDING_ONNX_DATA_FILE));
        assert!(info
            .required_files
            .iter()
            .any(|file| file.source_path == "onnx/model.onnx_data"));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn local_embedding_model_info_reports_available_when_all_bge_m3_assets_exist() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        for filename in [
            LOCAL_EMBEDDING_MODEL_FILE,
            LOCAL_EMBEDDING_ONNX_DATA_FILE,
            LOCAL_EMBEDDING_TOKENIZER_FILE,
        ] {
            std::fs::write(temp.path().join(filename), b"asset")
                .expect("asset file should be writable");
        }

        let info = get_local_embedding_model_info(Some(temp.path().to_path_buf()));

        assert!(info.exists);
        assert!(info.available);
        assert!(info.missing_files.is_empty());
        assert_eq!(info.directory, temp.path().to_string_lossy());
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn find_ort_dylib_resolves_hydrated_app_data_runtime_lib_for_local_bge_m3() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let app_data_root = temp.path().join("com.entropia.desktop");
        let model_dir = app_data_root
            .join("models")
            .join("embeddings")
            .join("bge-m3");
        let runtime_lib_dir = app_data_root
            .join("runtime")
            .join("2026.05.0")
            .join("resources")
            .join("lib");
        std::fs::create_dir_all(&model_dir).expect("model dir should be created");
        std::fs::create_dir_all(&runtime_lib_dir).expect("runtime lib dir should be created");
        let expected = runtime_lib_dir.join(runtime_file_names()[0]);
        std::fs::write(&expected, b"runtime").expect("runtime dll should be writable");

        let resolved = find_ort_dylib(&model_dir).expect("runtime dll should resolve");

        assert_eq!(resolved, expected);
    }

    #[cfg(feature = "local-ml")]
    #[cfg(windows)]
    #[test]
    fn find_ort_dylib_resolves_dev_fallback_venv_onnxruntime() {
        // The deps installer drops onnxruntime into the dev-fallback venv
        // (runtime-dev/system-python). find_ort_dylib must reuse it when the
        // managed runtime-pack is not yet hydrated.
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let app_data_root = temp.path().join("com.entropia.desktop");
        let model_dir = app_data_root
            .join("models")
            .join("embeddings")
            .join("bge-m3");
        let capi_dir = app_data_root
            .join("runtime-dev")
            .join("system-python")
            .join("Lib")
            .join("site-packages")
            .join("onnxruntime")
            .join("capi");
        std::fs::create_dir_all(&model_dir).expect("model dir should be created");
        std::fs::create_dir_all(&capi_dir).expect("capi dir should be created");
        let expected = capi_dir.join(runtime_file_names()[0]);
        std::fs::write(&expected, b"runtime").expect("runtime dll should be writable");

        let resolved = find_ort_dylib(&model_dir).expect("dev-fallback runtime dll should resolve");

        assert_eq!(resolved, expected);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn init_local_provider_preserves_ort_error_when_required_assets_exist() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        for filename in [
            LOCAL_EMBEDDING_MODEL_FILE,
            LOCAL_EMBEDDING_ONNX_DATA_FILE,
            LOCAL_EMBEDDING_TOKENIZER_FILE,
        ] {
            std::fs::write(temp.path().join(filename), b"asset")
                .expect("asset file should be writable");
        }

        let error = match EmbeddingEngine::init(EmbeddingConfig::local(
            DEFAULT_OPENROUTER_EMBEDDING_MODEL.to_string(),
            Some(temp.path().to_path_buf()),
        )) {
            Ok(_) => panic!("local provider should fail because ORT runtime is absent"),
            Err(error) => error,
        };

        assert!(
            error.contains("No ONNX Runtime dynamic library found")
                || error.contains("Failed to load local BGE-M3 tokenizer"),
            "expected runtime or tokenizer initialization error, got: {error}"
        );
        assert!(error.contains(&temp.path().to_string_lossy().to_string()));
        assert!(!error.contains("Local BGE-M3 model incomplete"));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn embedding_vector_from_onnx_output_accepts_cls_hidden_state_shape() {
        let values: Vec<f32> = (0..(2 * OPENROUTER_EMBEDDING_DIMENSIONS))
            .map(|index| index as f32)
            .collect();
        let array =
            ndarray::Array3::from_shape_vec((1, 2, OPENROUTER_EMBEDDING_DIMENSIONS), values)
                .expect("array shape should be valid");

        let vector = embedding_vector_from_onnx_output(array.view().into_dyn())
            .expect("CLS hidden state should be accepted");

        assert_eq!(vector.len(), OPENROUTER_EMBEDDING_DIMENSIONS);
        assert_eq!(vector[0], 0.0);
        assert_eq!(vector[OPENROUTER_EMBEDDING_DIMENSIONS - 1], 1023.0);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn l2_normalize_returns_unit_length_vector() {
        let vector = l2_normalize(vec![3.0, 4.0]).expect("vector should normalize");
        assert!((vector[0] - 0.6).abs() < 0.0001);
        assert!((vector[1] - 0.8).abs() < 0.0001);
    }

    #[test]
    fn embed_text_normalizes_successful_openrouter_bge_m3_response() {
        let vector: Vec<f32> = (0..OPENROUTER_EMBEDDING_DIMENSIONS)
            .map(|index| index as f32 / 10.0)
            .collect();
        let expected_last =
            vector[OPENROUTER_EMBEDDING_DIMENSIONS - 1] / vector_norm(&vector) as f32;
        let endpoint = local_openrouter_embedding_server(vector.clone());
        let engine = EmbeddingEngine::init_with_endpoint(
            EmbeddingConfig::openrouter(
                "sk-test".to_string(),
                DEFAULT_OPENROUTER_EMBEDDING_MODEL.to_string(),
            ),
            endpoint,
        )
        .expect("test embedding engine should initialize");

        let result = engine
            .embed_text("texto histórico para embedding")
            .expect("mocked OpenRouter response should embed successfully");

        assert_eq!(result.len(), OPENROUTER_EMBEDDING_DIMENSIONS);
        assert_eq!(result[0], 0.0);
        assert!((result[OPENROUTER_EMBEDDING_DIMENSIONS - 1] - expected_last).abs() < 0.000_001);
        assert!((vector_norm(&result) - 1.0).abs() < 0.000_001);
    }

    fn local_openrouter_embedding_server(vector: Vec<f32>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server should bind");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("local addr should exist")
        );

        thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("mock server should receive request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream
                    .read(&mut buffer)
                    .expect("request should be readable");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request_text = String::from_utf8_lossy(&request);
            let content_length = request_text
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .expect("OpenRouter request should include a JSON body length");
            let header_end = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|index| index + 4)
                .expect("HTTP headers should terminate");
            while request.len() < header_end + content_length {
                let read = stream
                    .read(&mut buffer)
                    .expect("request body should be readable");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            let request_text = String::from_utf8_lossy(&request);
            assert!(request_text.starts_with("POST / HTTP/1.1"));
            assert!(request_text.contains("authorization: Bearer sk-test"));
            assert!(request_text.contains("\"model\":\"baai/bge-m3\""));
            assert!(request_text.contains("texto histórico para embedding"));

            let body = serde_json::json!({
                "data": [
                    { "embedding": vector }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("mock response should write");
        });

        endpoint
    }

    #[test]
    fn upsert_vec_asset_writes_when_vec_assets_table_exists() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute(
            "CREATE TABLE vec_assets(asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL, embedding_model TEXT NOT NULL DEFAULT 'legacy', embedding_contract TEXT NOT NULL DEFAULT 'legacy', dimensions INTEGER NOT NULL DEFAULT 0)",
            [],
        )
        .expect("vec_assets table should be created");

        upsert_vec_asset(&conn, "item-1", "asset-1", &[9, 8, 7, 6]).expect("upsert should succeed");

        let stored: (String, String, i64) = conn
            .query_row(
                "SELECT embedding_model, embedding_contract, dimensions FROM vec_assets WHERE asset_id = 'asset-1' AND item_id = 'item-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("metadata query should succeed");
        assert_eq!(stored.0, CANONICAL_EMBEDDING_MODEL);
        assert_eq!(stored.1, CANONICAL_EMBEDDING_CONTRACT_V1);
        assert_eq!(stored.2, CANONICAL_EMBEDDING_DIMENSIONS as i64);
    }

    #[test]
    fn upsert_vec_asset_twice_emits_update_not_delete_insert() {
        // Regression for #21: re-embedding the same asset must emit ONE UPDATE in
        // sync_oplog, NOT a DELETE+INSERT pair. INSERT OR REPLACE = DELETE+INSERT
        // fires the `_d` tombstone trigger, which can delete the row on the remote.
        use crate::sync::capture::ensure_capture;
        use crate::sync::test_support::{new_synced_test_db, set_session_with_capture};

        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        set_session_with_capture(&conn);

        upsert_vec_asset(&conn, "item-1", "asset-1", &[1u8, 2, 3]).expect("first upsert");
        upsert_vec_asset(&conn, "item-1", "asset-1", &[4u8, 5, 6]).expect("second upsert");

        let deletes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_oplog WHERE table_name = 'vec_assets' AND op = 'D'",
                [],
                |row| row.get(0),
            )
            .expect("count deletes");
        let updates: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_oplog WHERE table_name = 'vec_assets' AND op = 'U'",
                [],
                |row| row.get(0),
            )
            .expect("count updates");

        assert_eq!(
            deletes, 0,
            "re-embedding must not emit a tombstone (op 'D')"
        );
        assert!(updates >= 1, "re-embedding must emit an UPDATE (op 'U')");
    }

    #[test]
    fn list_asset_embedding_candidates_reindexes_missing_legacy_and_incompatible_embeddings() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            r#"
            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE transcriptions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT NOT NULL,
              language TEXT,
              duration_ms INTEGER,
              model TEXT NOT NULL,
              segments TEXT,
              confidence REAL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE vec_assets (
              asset_id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              embedding BLOB NOT NULL,
              embedding_model TEXT NOT NULL DEFAULT 'legacy',
              embedding_contract TEXT NOT NULL DEFAULT 'legacy',
              dimensions INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .expect("schema should be created");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["asset-a", "item-1", "a.txt", "txt", 1_i64],
        )
        .expect("asset a should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["asset-b", "item-1", "b.txt", "txt", 2_i64],
        )
        .expect("asset b should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["asset-c", "item-2", "c.txt", "txt", 3_i64],
        )
        .expect("asset c should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["asset-d", "item-2", "d.txt", "txt", 4_i64],
        )
        .expect("asset d should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["asset-e", "item-3", "e.txt", "txt", 5_i64],
        )
        .expect("asset e should insert");

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params!["ext-a", "asset-a", "texto OCR", 10_i64],
        )
        .expect("extraction should insert");
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model, segments, confidence, created_at) VALUES (?1, ?2, ?3, 'es', 1000, 'base', '[]', 0.9, ?4)",
            params!["tr-b", "asset-b", "audio transcripto", 20_i64],
        )
        .expect("transcription should insert");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params!["ext-c", "asset-c", "   ", 30_i64],
        )
        .expect("blank extraction should insert");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params!["ext-d", "asset-d", "vector legacy", 40_i64],
        )
        .expect("legacy extraction should insert");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params!["ext-e", "asset-e", "dimensiones incorrectas", 50_i64],
        )
        .expect("wrong-dimensions extraction should insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "asset-b",
                "item-1",
                vec![1_u8, 2, 3, 4],
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS as i64
            ],
        )
        .expect("current vec asset should insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES (?1, ?2, ?3)",
            params!["asset-d", "item-2", vec![5_u8, 6, 7, 8]],
        )
        .expect("legacy vec asset should insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "asset-e",
                "item-3",
                vec![9_u8, 10, 11, 12],
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                384_i64
            ],
        )
        .expect("wrong-dimensions vec asset should insert");

        let candidates = list_asset_embedding_candidates(&conn, false, None)
            .expect("candidate query should succeed");

        assert_eq!(
            candidates,
            vec![
                AssetEmbeddingCandidate {
                    asset_id: "asset-a".to_string(),
                    item_id: "item-1".to_string(),
                },
                AssetEmbeddingCandidate {
                    asset_id: "asset-d".to_string(),
                    item_id: "item-2".to_string(),
                },
                AssetEmbeddingCandidate {
                    asset_id: "asset-e".to_string(),
                    item_id: "item-3".to_string(),
                },
            ]
        );
    }

    #[test]
    fn list_asset_embedding_candidates_force_mode_includes_existing_embeddings() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            r#"
            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE transcriptions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT NOT NULL,
              language TEXT,
              duration_ms INTEGER,
              model TEXT NOT NULL,
              segments TEXT,
              confidence REAL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE vec_assets (
              asset_id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              embedding BLOB NOT NULL,
              embedding_model TEXT NOT NULL DEFAULT 'legacy',
              embedding_contract TEXT NOT NULL DEFAULT 'legacy',
              dimensions INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .expect("schema should be created");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES ('asset-z', 'item-z', 'z.txt', 'txt', 1)",
            [],
        )
        .expect("asset should insert");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES ('ext-z', 'asset-z', 'texto', 2)",
            [],
        )
        .expect("extraction should insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES ('asset-z', 'item-z', ?1)",
            params![vec![9_u8, 9, 9, 9]],
        )
        .expect("vec asset should insert");

        let candidates = list_asset_embedding_candidates(&conn, true, Some(10))
            .expect("force query should succeed");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].asset_id, "asset-z");
    }
    fn prepare_rag_embedding_test_db(conn: &Connection) {
        conn.execute_batch(
            r#"
            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE transcriptions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              language TEXT,
              duration_ms INTEGER,
              model TEXT NOT NULL,
              segments TEXT,
              confidence REAL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE vec_assets (
              asset_id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              embedding BLOB NOT NULL,
              embedding_model TEXT NOT NULL DEFAULT 'legacy',
              embedding_contract TEXT NOT NULL DEFAULT 'legacy',
              dimensions INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .expect("RAG embedding schema should be created");
        ensure_rag_embedding_state_schema(conn).expect("RAG state schema should be created");
    }

    fn seed_rag_asset(conn: &Connection, item_id: &str, asset_id: &str, text: &str) {
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![asset_id, item_id, "asset.txt", "txt", 1_i64],
        )
        .expect("asset should insert");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![format!("ext-{asset_id}"), asset_id, text, 2_i64],
        )
        .expect("extraction should insert");
    }

    #[test]
    fn persist_asset_embedding_partial_backfill_failure_leaves_incomplete_candidate() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        prepare_rag_embedding_test_db(&conn);
        seed_rag_asset(&conn, "item-1", "asset-1", "texto para embedding");

        let error = persist_asset_embedding_with_rag_state_at(
            &conn,
            "item-1",
            "asset-1",
            &[1_u8, 2, 3, 4],
            1_000,
            |_db, _item_id, _asset_id| Err("database is locked".to_string()),
        )
        .expect_err("chunk backfill failure should surface");

        assert_eq!(error, "database is locked");

        let persisted_vec: Option<(String, Vec<u8>, String, String, i64)> = conn
            .query_row(
                "SELECT item_id, embedding, embedding_model, embedding_contract, dimensions FROM vec_assets WHERE asset_id = ?1",
                params!["asset-1"],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .expect("compatible vec row lookup should succeed");
        assert_eq!(
            persisted_vec,
            Some((
                "item-1".to_string(),
                vec![1_u8, 2, 3, 4],
                CANONICAL_EMBEDDING_MODEL.to_string(),
                CANONICAL_EMBEDDING_CONTRACT_V1.to_string(),
                CANONICAL_EMBEDDING_DIMENSIONS as i64,
            )),
            "canonical vec row must survive a chunk backfill failure"
        );

        let (rag_incomplete, failure_count, next_retry_at_ms, last_error): (
            i64,
            i64,
            i64,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT rag_incomplete, failure_count, next_retry_at_ms, last_error FROM rag_asset_embedding_state WHERE asset_id = ?1",
                params!["asset-1"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("rag state row should exist");

        assert_eq!(rag_incomplete, 1);
        assert_eq!(failure_count, 0);
        assert_eq!(next_retry_at_ms, 0);
        assert!(last_error.is_none());

        let candidates = list_asset_embedding_candidates_at(&conn, false, None, 1_000)
            .expect("candidate scan should succeed");
        assert_eq!(
            candidates,
            vec![AssetEmbeddingCandidate {
                asset_id: "asset-1".to_string(),
                item_id: "item-1".to_string(),
            }],
            "a compatible vec row with rag_incomplete=1 must remain a candidate"
        );
    }

    #[test]
    fn rag_embedding_backoff_persists_reopens_and_clears_on_success() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp.path().join("rag-embedding-state.sqlite");
        let now_ms = 1_000;

        {
            let conn = Connection::open(&db_path).expect("sqlite file should open");
            prepare_rag_embedding_test_db(&conn);
            seed_rag_asset(&conn, "item-2", "asset-2", "texto persistente");
            conn.execute(
                "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "asset-2",
                    "item-2",
                    vec![9_u8, 9, 9, 9],
                    CANONICAL_EMBEDDING_MODEL,
                    CANONICAL_EMBEDDING_CONTRACT_V1,
                    CANONICAL_EMBEDDING_DIMENSIONS as i64,
                ],
            )
            .expect("compatible vec asset should insert");
            record_rag_embedding_failure_at(
                &conn,
                "item-2",
                "asset-2",
                now_ms,
                "embedding provider is not configured",
            )
            .expect("failure state should record");
        }

        {
            let conn = Connection::open(&db_path).expect("sqlite file should reopen");

            let (rag_incomplete, failure_count, next_retry_at_ms, last_error): (
                i64,
                i64,
                i64,
                Option<String>,
            ) = conn
                .query_row(
                    "SELECT rag_incomplete, failure_count, next_retry_at_ms, last_error FROM rag_asset_embedding_state WHERE asset_id = ?1",
                    params!["asset-2"],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .expect("backoff row should persist across reopen");

            assert_eq!(rag_incomplete, 1);
            assert_eq!(failure_count, 1);
            assert_eq!(next_retry_at_ms, now_ms + RAG_REPAIR_BACKOFF_INITIAL_MS);
            assert_eq!(
                last_error.as_deref(),
                Some("embedding provider is not configured")
            );

            let not_due = list_asset_embedding_candidates_at(
                &conn,
                false,
                None,
                next_retry_at_ms.saturating_sub(1),
            )
            .expect("candidate scan should succeed");
            assert!(
                not_due.is_empty(),
                "candidate must be skipped before next_retry_at_ms"
            );

            let due = list_asset_embedding_candidates_at(&conn, false, None, next_retry_at_ms)
                .expect("candidate scan should succeed");
            assert_eq!(
                due,
                vec![AssetEmbeddingCandidate {
                    asset_id: "asset-2".to_string(),
                    item_id: "item-2".to_string(),
                }],
                "candidate must reappear when backoff becomes due"
            );

            persist_asset_embedding_with_rag_state_at(
                &conn,
                "item-2",
                "asset-2",
                &[5_u8, 6, 7, 8],
                next_retry_at_ms,
                |_db, _item_id, _asset_id| Ok(()),
            )
            .expect("successful full persistence should clear state");

            let cleared: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM rag_asset_embedding_state WHERE asset_id = ?1",
                    params!["asset-2"],
                    |row| row.get(0),
                )
                .optional()
                .expect("state lookup should succeed");
            assert!(
                cleared.is_none(),
                "successful embedding should delete the backoff row"
            );
        }
    }

    #[test]
    fn summarize_asset_embedding_coverage_counts_text_and_missing_rows() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            r#"
            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE transcriptions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT NOT NULL,
              language TEXT,
              duration_ms INTEGER,
              model TEXT NOT NULL,
              segments TEXT,
              confidence REAL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE vec_assets (
              asset_id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              embedding BLOB NOT NULL,
              embedding_model TEXT NOT NULL DEFAULT 'legacy',
              embedding_contract TEXT NOT NULL DEFAULT 'legacy',
              dimensions INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .expect("schema should be created");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES ('asset-1', 'item-1', '1.txt', 'txt', 1)",
            [],
        )
        .expect("asset 1 should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES ('asset-2', 'item-2', '2.txt', 'audio', 2)",
            [],
        )
        .expect("asset 2 should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES ('asset-3', 'item-3', '3.txt', 'txt', 3)",
            [],
        )
        .expect("asset 3 should insert");

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES ('ext-1', 'asset-1', 'texto uno', 10)",
            [],
        )
        .expect("extraction should insert");
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model, segments, confidence, created_at) VALUES ('tr-2', 'asset-2', 'audio dos', 'es', 1000, 'base', '[]', 0.9, 20)",
            [],
        )
        .expect("transcription should insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions) VALUES ('asset-1', 'item-1', ?1, ?2, ?3, ?4)",
            params![
                vec![1_u8, 2, 3, 4],
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS as i64
            ],
        )
        .expect("current vec asset should insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES ('asset-2', 'item-2', ?1)",
            params![vec![5_u8, 6, 7, 8]],
        )
        .expect("legacy vec asset should insert");

        let summary =
            summarize_asset_embedding_coverage(&conn).expect("coverage summary should succeed");

        assert_eq!(summary.total_assets, 3);
        assert_eq!(summary.assets_with_text, 2);
        assert_eq!(summary.assets_with_embedding, 1);
        assert_eq!(summary.assets_missing_embedding, 1);
    }

    #[test]
    fn rolling_hash64_is_stable_for_same_input() {
        let a = rolling_hash64(b"hola");
        let b = rolling_hash64(b"hola");
        let c = rolling_hash64(b"adios");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    fn axis_vector(axis: usize, magnitude: f32) -> Vec<f32> {
        let mut vector = vec![0.0; OPENROUTER_EMBEDDING_DIMENSIONS];
        vector[axis] = magnitude;
        vector
    }

    fn vector_norm(vector: &[f32]) -> f64 {
        vector
            .iter()
            .map(|value| (*value as f64).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    #[test]
    fn embedding_preprocessing_keeps_short_text_in_one_chunk() {
        let chunks = chunk_embedding_text("short historical note")
            .expect("non-empty text should produce chunks");

        assert_eq!(chunks, vec!["short historical note"]);
    }

    #[test]
    fn embedding_preprocessing_obeys_chunk_boundaries() {
        let at_limit = "a".repeat(EMBEDDING_CHUNK_MAX_CHARS);
        let over_limit = "b".repeat(EMBEDDING_CHUNK_MAX_CHARS + 1);

        assert_eq!(
            chunk_embedding_text(&at_limit)
                .expect("text at limit should be accepted")
                .len(),
            1
        );
        let chunks = chunk_embedding_text(&over_limit).expect("text over limit should be chunked");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), EMBEDDING_CHUNK_MAX_CHARS);
        assert_eq!(chunks[1], "b");
    }

    #[test]
    fn embedding_preprocessing_counts_unicode_scalars_and_reconstructs_without_loss() {
        let text = format!("{}\nfin ñ", "🎉".repeat(EMBEDDING_CHUNK_MAX_CHARS + 3));
        let chunks = chunk_embedding_text(&text).expect("Unicode text should be chunked");

        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= EMBEDDING_CHUNK_MAX_CHARS));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn chunk_aggregation_weights_by_non_whitespace_character_count() {
        let chunks = vec!["a  a a", "b"];
        let result =
            aggregate_chunk_embeddings(&chunks, vec![axis_vector(0, 4.0), axis_vector(1, 7.0)])
                .expect("valid chunk vectors should aggregate");

        assert!((result[0] - 3.0 / 10.0_f32.sqrt()).abs() < 0.0001);
        assert!((result[1] - 1.0 / 10.0_f32.sqrt()).abs() < 0.0001);
    }

    #[test]
    fn chunk_aggregation_normalizes_provider_vectors() {
        let result = aggregate_chunk_embeddings(
            &["text"],
            vec![{
                let mut vector = axis_vector(0, 3.0);
                vector[1] = 4.0;
                vector
            }],
        )
        .expect("non-unit provider vector should be normalized");

        assert!((result[0] - 0.6).abs() < 0.0001);
        assert!((result[1] - 0.8).abs() < 0.0001);
    }

    #[test]
    fn chunk_aggregation_rejects_wrong_dimensions() {
        let error = aggregate_chunk_embeddings(&["text"], vec![vec![1.0; 3]])
            .expect_err("wrong dimensions must be rejected");

        assert!(error.contains("3 dimensions"));
        assert!(error.contains("1024"));
    }

    #[test]
    fn chunk_aggregation_rejects_non_finite_values() {
        let mut vector = axis_vector(0, 1.0);
        vector[4] = f32::NAN;

        let error =
            aggregate_chunk_embeddings(&["text"], vec![vector]).expect_err("NaN must be rejected");

        assert!(error.contains("non-finite"));
    }

    #[test]
    fn chunk_aggregation_rejects_zero_vectors() {
        let error =
            aggregate_chunk_embeddings(&["text"], vec![vec![0.0; OPENROUTER_EMBEDDING_DIMENSIONS]])
                .expect_err("zero vectors must be rejected");

        assert!(error.contains("zero"));
    }

    #[test]
    fn chunk_aggregation_returns_a_unit_vector() {
        let result = aggregate_chunk_embeddings(
            &["first", "second"],
            vec![axis_vector(0, 1.0), axis_vector(1, 1.0)],
        )
        .expect("valid vectors should aggregate");

        assert!((vector_norm(&result) - 1.0).abs() < 0.000_001);
    }

    #[test]
    fn canonical_rag_chunks_are_deterministic_and_preserve_source_provenance() {
        let source = RagChunkSource {
            asset_id: "asset-1".to_string(),
            item_id: "item-1".to_string(),
            source_kind: RagChunkSourceKind::Extraction,
            source_id: "ext-asset-1".to_string(),
            text: format!("{} segundo pasaje", "primer pasaje ".repeat(80)),
        };

        let first = plan_rag_chunks(&source).expect("chunk planning should succeed");
        let second = plan_rag_chunks(&source).expect("same source should plan again");

        assert_eq!(
            first, second,
            "canonical chunk identity must be deterministic"
        );
        assert!(first.len() > 1, "worked source must cross a chunk boundary");
        for (ordinal, chunk) in first.iter().enumerate() {
            assert_eq!(chunk.asset_id, source.asset_id);
            assert_eq!(chunk.item_id, source.item_id);
            assert_eq!(chunk.source_kind, source.source_kind);
            assert_eq!(chunk.source_id, source.source_id);
            assert_eq!(chunk.chunk_ordinal, ordinal);
            assert!(chunk.start_char < chunk.end_char);
            assert!(chunk.end_char <= source.text.chars().count());
            assert_eq!(
                chunk.text_content,
                source
                    .text
                    .chars()
                    .skip(chunk.start_char)
                    .take(chunk.end_char - chunk.start_char)
                    .collect::<String>()
            );
        }
        assert_ne!(
            canonical_rag_chunk_id(&source.asset_id, source.source_kind, &source.source_id, 0),
            canonical_rag_chunk_id(
                &source.asset_id,
                RagChunkSourceKind::Transcription,
                &source.source_id,
                0
            ),
            "source kind participates in canonical identity"
        );
    }

    #[test]
    fn rag_chunk_backfill_keeps_current_rows_and_atomically_replaces_stale_sets() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE items (id TEXT PRIMARY KEY);
            CREATE TABLE assets (
                id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE
            );
            CREATE TABLE rag_chunks (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
                source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL,
                chunk_ordinal INTEGER NOT NULL,
                text_content TEXT NOT NULL,
                start_char INTEGER NOT NULL,
                end_char INTEGER NOT NULL,
                source_text_hash TEXT NOT NULL,
                chunking_contract TEXT NOT NULL,
                embedding BLOB NOT NULL,
                embedding_model TEXT NOT NULL,
                embedding_contract TEXT NOT NULL,
                dimensions INTEGER NOT NULL,
                UNIQUE(asset_id, source_kind, source_id, chunk_ordinal)
            );
            INSERT INTO items(id) VALUES ('item-1');
            INSERT INTO assets(id, item_id) VALUES ('asset-1', 'item-1');
            "#,
        )
        .expect("chunk backfill schema should initialize");
        let embedding = RagChunkEmbeddingSpec {
            model: CANONICAL_EMBEDDING_MODEL,
            contract: CANONICAL_EMBEDDING_CONTRACT_V1,
            dimensions: CANONICAL_EMBEDDING_DIMENSIONS,
        };
        let original = RagChunkSource {
            asset_id: "asset-1".to_string(),
            item_id: "item-1".to_string(),
            source_kind: RagChunkSourceKind::Extraction,
            source_id: "ext-asset-1".to_string(),
            text: "texto original ".repeat(100),
        };
        let embed = |_text: &str| Ok(vec![0.25; CANONICAL_EMBEDDING_DIMENSIONS]);

        assert_eq!(
            backfill_rag_chunks(&conn, &original, embedding, embed)
                .expect("first backfill should succeed"),
            RagChunkBackfillOutcome::Replaced
        );
        let before: Vec<(i64, String)> = conn
            .prepare("SELECT rowid, id FROM rag_chunks ORDER BY chunk_ordinal")
            .expect("row query should prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("row query should run")
            .collect::<Result<_, _>>()
            .expect("rows should decode");
        assert_eq!(
            backfill_rag_chunks(&conn, &original, embedding, embed)
                .expect("current backfill should succeed"),
            RagChunkBackfillOutcome::Current
        );
        let unchanged: Vec<(i64, String)> = conn
            .prepare("SELECT rowid, id FROM rag_chunks ORDER BY chunk_ordinal")
            .expect("row query should prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("row query should run")
            .collect::<Result<_, _>>()
            .expect("rows should decode");
        assert_eq!(unchanged, before, "current rows must remain untouched");

        let stale = RagChunkSource {
            text: "contenido revisado ".repeat(20),
            ..original
        };
        assert_eq!(
            backfill_rag_chunks(&conn, &stale, embedding, embed)
                .expect("stale set replacement should succeed"),
            RagChunkBackfillOutcome::Replaced
        );
        let current: Vec<(String, String, String)> = conn
            .prepare(
                "SELECT id, source_text_hash, text_content
                 FROM rag_chunks
                 ORDER BY chunk_ordinal",
            )
            .expect("chunk query should prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("chunk query should run")
            .collect::<Result<_, _>>()
            .expect("chunks should decode");
        assert_eq!(
            current[0].0, before[0].1,
            "canonical identity remains stable for the same source ordinal"
        );
        let stale_hash = sha256_hex(stale.text.as_bytes());
        assert!(
            current.iter().all(|(_, hash, text)| {
                hash == &stale_hash && text.contains("contenido revisado")
            }),
            "the stale set must be fully replaced with one current source hash"
        );

        conn.execute("DELETE FROM assets WHERE id = 'asset-1'", [])
            .expect("asset delete should cascade");
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_chunks", [], |row| row.get(0))
            .expect("remaining chunk count should query");
        assert_eq!(remaining, 0, "asset deletion must cascade to chunks");
    }
}
