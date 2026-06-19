pub mod commands;
#[cfg(feature = "local-ml")]
pub mod download;
#[cfg(feature = "local-ml")]
pub mod engine;
pub mod openrouter;
pub mod prompt;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(feature = "local-ml")]
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::nlp::text_provider;
use crate::settings;

#[cfg(feature = "local-ml")]
use self::engine::{LlmConfig, LlmEngine};
use self::openrouter::OpenRouterClient;

const LLM_LOCAL_PREFIX: &str = "[llm-local]";
const LLM_CLOUD_PREFIX: &str = "[llm-cloud]";
const LLM_TARGET_ASSET: &str = "asset";
const LLM_TARGET_ITEM: &str = "item";
const LLM_TARGET_COLLECTION: &str = "collection";

#[cfg(feature = "local-ml")]
pub(crate) type SharedLocalLlmEngine = Arc<Mutex<LlmEngine>>;

#[cfg(feature = "local-ml")]
static LOCAL_GEMMA_ENGINE: Lazy<Mutex<Option<SharedLocalLlmEngine>>> =
    Lazy::new(|| Mutex::new(None));

/// Settings key for the local model filename.
pub const LOCAL_MODEL_FILENAME_KEY: &str = "local_model_filename";
/// Settings key for the local model download source URL.
pub const LOCAL_MODEL_SOURCE_URL_KEY: &str = "local_model_source_url";
/// Default public source for the local model (unsloth/gemma-4-E2B-it-GGUF).
pub const DEFAULT_MODEL_SOURCE_URL: &str = "https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf?download=true";
/// Expected filename for the local Gemma GGUF model.
pub const MODEL_FILENAME: &str = "gemma-4-E2B-it-Q4_K_M.gguf";
const LEGACY_MODEL_FILENAMES: &[&str] = &[
    "google_gemma-3-4b-it-Q4_K_M.gguf",
    "gemma-3-4b-it-Q4_K_M.gguf",
];
const LEGACY_MODEL_SOURCE_URLS: &[&str] = &[
    "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/main/google_gemma-3-4b-it-Q4_K_M.gguf",
    "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-Q4_K_M.gguf",
];

pub fn resolve_local_model_filename(conn: Option<&rusqlite::Connection>) -> String {
    let configured = conn
        .and_then(|db| crate::settings::get_setting(db, LOCAL_MODEL_FILENAME_KEY))
        .filter(|value| !value.trim().is_empty());

    match configured.as_deref() {
        Some(value) if LEGACY_MODEL_FILENAMES.contains(&value) => MODEL_FILENAME.to_string(),
        Some(other) => other.to_string(),
        None => MODEL_FILENAME.to_string(),
    }
}

pub fn resolve_local_model_source_url(conn: Option<&rusqlite::Connection>) -> String {
    let configured = conn
        .and_then(|db| crate::settings::get_setting(db, LOCAL_MODEL_SOURCE_URL_KEY))
        .filter(|value| !value.trim().is_empty());

    match configured.as_deref() {
        Some(value) if is_legacy_default_model_source(value) => {
            DEFAULT_MODEL_SOURCE_URL.to_string()
        }
        Some(other) => other.to_string(),
        None => DEFAULT_MODEL_SOURCE_URL.to_string(),
    }
}

fn is_legacy_default_model_source(value: &str) -> bool {
    let trimmed = value.trim();
    LEGACY_MODEL_SOURCE_URLS.contains(&trimmed)
        || trimmed.contains("bartowski/google_gemma-3-4b-it-GGUF")
        || trimmed.contains("gemma-3-4b-it-Q4_K_M.gguf")
        || trimmed.contains("google_gemma-3-4b-it-Q4_K_M.gguf")
}

#[cfg(feature = "local-ml")]
fn persist_default_model_setting_migrations(conn: &rusqlite::Connection) -> Result<(), String> {
    if let Some(filename) = settings::get_setting(conn, LOCAL_MODEL_FILENAME_KEY) {
        if LEGACY_MODEL_FILENAMES.contains(&filename.as_str()) {
            settings::set_setting(conn, LOCAL_MODEL_FILENAME_KEY, MODEL_FILENAME)
                .map_err(|e| format!("Failed to migrate local model filename setting: {e}"))?;
        }
    }
    if let Some(source_url) = settings::get_setting(conn, LOCAL_MODEL_SOURCE_URL_KEY) {
        if is_legacy_default_model_source(&source_url) {
            settings::set_setting(conn, LOCAL_MODEL_SOURCE_URL_KEY, DEFAULT_MODEL_SOURCE_URL)
                .map_err(|e| format!("Failed to migrate local model source URL setting: {e}"))?;
        }
    }
    Ok(())
}

fn should_auto_download_default_model(
    conn: &rusqlite::Connection,
    model_path: &std::path::Path,
) -> bool {
    !model_path.exists()
        && resolve_local_model_filename(Some(conn)) == MODEL_FILENAME
        && resolve_local_model_source_url(Some(conn)) == DEFAULT_MODEL_SOURCE_URL
}

#[cfg(feature = "local-ml")]
pub(crate) fn ensure_default_model_downloaded_if_missing(
    conn: &rusqlite::Connection,
    db_path: &std::path::Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    persist_default_model_setting_migrations(conn)?;
    let model_path = resolve_model_path(db_path);
    if !should_auto_download_default_model(conn, &model_path) {
        return Ok(());
    }
    if let Some(parent) = model_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create model directory {}: {e}", parent.display()))?;
    }
    eprintln!(
        "{LLM_LOCAL_PREFIX} Default Gemma model missing; starting controlled auto-download from default source"
    );
    self::download::download_model_file(DEFAULT_MODEL_SOURCE_URL, &model_path, app_handle)
}

#[cfg(feature = "local-ml")]
pub(crate) fn get_or_init_local_gemma_engine(
    conn: &rusqlite::Connection,
    db_path: &std::path::Path,
    app_handle: &AppHandle,
) -> Result<SharedLocalLlmEngine, String> {
    let mut cached = LOCAL_GEMMA_ENGINE
        .lock()
        .map_err(|error| format!("Local Gemma engine cache lock poisoned: {error}"))?;

    if let Some(engine) = cached.as_ref() {
        return Ok(Arc::clone(engine));
    }

    ensure_default_model_downloaded_if_missing(conn, db_path, app_handle)?;
    let model_path = resolve_model_path(db_path);
    eprintln!("{LLM_LOCAL_PREFIX} OCRC configured as text-only (multimodal disabled)");
    let engine = LlmEngine::init(LlmConfig {
        model_path,
        n_ctx: 4096,
        n_threads: None,
        seed: 1234,
    })?;
    let engine = Arc::new(Mutex::new(engine));
    *cached = Some(Arc::clone(&engine));
    Ok(engine)
}

/// Resolved status of the local LLM model on disk.
#[derive(Clone, serde::Serialize)]
pub struct LocalModelInfo {
    pub exists: bool,
    pub available: bool,
    pub can_auto_download: bool,
    pub disabled_reason: Option<String>,
    pub path: String,
    pub size_bytes: Option<u64>,
    pub filename: String,
    pub source_url: String,
}

fn local_model_can_initialize_from_conn(
    conn: &rusqlite::Connection,
    db_path: &std::path::Path,
) -> bool {
    let model_path = resolve_model_path(db_path);
    model_path.exists() || should_auto_download_default_model(conn, &model_path)
}

/// Resolve the expected model path and report whether the file is present.
/// The models directory is created idempotently so callers can open it.
/// Reads `local_model_filename` from settings; falls back to `MODEL_FILENAME`.
pub fn resolve_model_path(db_path: &std::path::Path) -> std::path::PathBuf {
    let app_models_dir = db_path
        .parent()
        .expect("db_path should have a parent")
        .join("models");
    std::fs::create_dir_all(&app_models_dir).ok();

    let filename = if let Ok(conn) = rusqlite::Connection::open(db_path) {
        resolve_local_model_filename(Some(&conn))
    } else {
        MODEL_FILENAME.to_string()
    };

    let current_dir = std::env::current_dir().unwrap_or_default();
    let mut search_paths = vec![
        app_models_dir.join(&filename),
        current_dir.join(&filename),
        current_dir.join(format!("google_{filename}")),
    ];

    if filename == MODEL_FILENAME {
        search_paths.extend(
            LEGACY_MODEL_FILENAMES
                .iter()
                .flat_map(|legacy| [app_models_dir.join(legacy), current_dir.join(legacy)]),
        );
    }

    search_paths
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| app_models_dir.join(&filename))
}

/// Build a `LocalModelInfo` snapshot from the current filesystem state.
pub fn get_local_model_info(db_path: &std::path::Path) -> LocalModelInfo {
    let path = resolve_model_path(db_path);
    let exists = path.exists();
    let size_bytes = exists
        .then(|| std::fs::metadata(&path).ok().map(|m| m.len()))
        .flatten();
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| MODEL_FILENAME.to_string());
    let (source_url, can_auto_download) = if let Ok(conn) = rusqlite::Connection::open(db_path) {
        (
            resolve_local_model_source_url(Some(&conn)),
            should_auto_download_default_model(&conn, &path),
        )
    } else {
        (DEFAULT_MODEL_SOURCE_URL.to_string(), false)
    };
    LocalModelInfo {
        exists,
        available: exists || can_auto_download,
        can_auto_download,
        disabled_reason: None,
        path: path.to_string_lossy().to_string(),
        size_bytes,
        filename,
        source_url,
    }
}

fn llm_prefix(use_cloud: bool) -> &'static str {
    if use_cloud {
        LLM_CLOUD_PREFIX
    } else {
        LLM_LOCAL_PREFIX
    }
}

fn llm_job_suffix(job: &LlmJob) -> Option<&'static str> {
    match job {
        LlmJob::CorrectOcr { .. } | LlmJob::CorrectOcrAsset { .. } => Some("correction"),
        LlmJob::Summarize { .. } | LlmJob::SummarizeAsset { .. } => Some("summary"),
        LlmJob::ExtractTriples { .. } | LlmJob::ExtractTriplesAsset { .. } => Some("triples"),
        _ => None,
    }
}

fn llm_job_prefix(use_cloud: bool, job: &LlmJob) -> String {
    match llm_job_suffix(job) {
        Some(suffix) => format!("{}[{}]", llm_prefix(use_cloud), suffix),
        None => llm_prefix(use_cloud).to_string(),
    }
}

// ---------------------------------------------------------------------------
// Job definition
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum LlmJob {
    CorrectOcr {
        item_id: String,
    },
    ExtractEntities {
        item_id: String,
    },
    #[allow(dead_code)] // Future: entity consolidation via LLM review (not yet wired)
    ConsolidateEntities {
        item_id: String,
        candidate_entities_json: String,
    },
    ExtractTriples {
        item_id: String,
    },
    Summarize {
        item_id: String,
    },
    Classify {
        item_id: String,
        categories: Vec<String>,
    },
    Ask {
        collection_id: String,
        question: String,
    },
    // Asset-level variants — operate on a single asset/page instead of the whole item.
    // These use get_asset_text() which only fetches text for the specified asset,
    // avoiding context-window overflow on multi-page documents.
    CorrectOcrAsset {
        asset_id: String,
    },
    ExtractEntitiesAsset {
        asset_id: String,
    },
    #[allow(dead_code)] // Future: entity consolidation via LLM review (not yet wired)
    ConsolidateEntitiesAsset {
        asset_id: String,
        candidate_entities_json: String,
    },
    ExtractTriplesAsset {
        asset_id: String,
    },
    SummarizeAsset {
        asset_id: String,
    },
}

impl LlmJob {
    fn job_name(&self) -> &'static str {
        match self {
            LlmJob::CorrectOcr { .. } => "correct_ocr",
            LlmJob::ExtractEntities { .. } => "extract_entities",
            LlmJob::ConsolidateEntities { .. } => "consolidate_entities",
            LlmJob::ExtractTriples { .. } => "extract_triples",
            LlmJob::Summarize { .. } => "summarize",
            LlmJob::Classify { .. } => "classify",
            LlmJob::Ask { .. } => "ask",
            LlmJob::CorrectOcrAsset { .. } => "correct_ocr",
            LlmJob::ExtractEntitiesAsset { .. } => "extract_entities",
            LlmJob::ConsolidateEntitiesAsset { .. } => "consolidate_entities",
            LlmJob::ExtractTriplesAsset { .. } => "extract_triples",
            LlmJob::SummarizeAsset { .. } => "summarize",
        }
    }

    /// Returns the ID used as the event/persistence target.
    /// For asset-level jobs, this is the asset_id; for item-level, the item_id.
    fn target_id(&self) -> &str {
        match self {
            LlmJob::CorrectOcr { item_id }
            | LlmJob::ExtractEntities { item_id }
            | LlmJob::ConsolidateEntities { item_id, .. }
            | LlmJob::ExtractTriples { item_id }
            | LlmJob::Summarize { item_id }
            | LlmJob::Classify { item_id, .. } => item_id,
            LlmJob::Ask { collection_id, .. } => collection_id,
            LlmJob::CorrectOcrAsset { asset_id }
            | LlmJob::ExtractEntitiesAsset { asset_id }
            | LlmJob::ConsolidateEntitiesAsset { asset_id, .. }
            | LlmJob::ExtractTriplesAsset { asset_id }
            | LlmJob::SummarizeAsset { asset_id } => asset_id,
        }
    }

    fn target_type(&self) -> &'static str {
        match self {
            LlmJob::CorrectOcr { .. }
            | LlmJob::ExtractEntities { .. }
            | LlmJob::ConsolidateEntities { .. }
            | LlmJob::ExtractTriples { .. }
            | LlmJob::Summarize { .. }
            | LlmJob::Classify { .. } => LLM_TARGET_ITEM,
            LlmJob::Ask { .. } => LLM_TARGET_COLLECTION,
            LlmJob::CorrectOcrAsset { .. }
            | LlmJob::ExtractEntitiesAsset { .. }
            | LlmJob::ConsolidateEntitiesAsset { .. }
            | LlmJob::ExtractTriplesAsset { .. }
            | LlmJob::SummarizeAsset { .. } => LLM_TARGET_ASSET,
        }
    }
}

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
pub struct LlmProgressPayload {
    pub id: String,
    pub job: String,
    pub pct: u8,
}

#[derive(Clone, Serialize)]
pub struct LlmCompletePayload {
    pub id: String,
    pub job: String,
    pub result: String,
}

#[derive(Clone, Serialize)]
pub struct LlmErrorPayload {
    pub id: String,
    pub job: String,
    pub error: String,
}

#[derive(Clone, Serialize)]
pub struct LlmDownloadProgressPayload {
    pub pct: u8,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
pub struct LlmDownloadCompletePayload {
    pub path: String,
}

#[derive(Clone, Serialize)]
pub struct LlmDownloadErrorPayload {
    pub error: String,
}

fn emit_progress(app_handle: &AppHandle, id: &str, job: &str, pct: u8) {
    if pct == 0 || pct == 10 || pct == 100 {
        crate::app_logs::info(app_handle, "llm", format!("{job} id={id} progreso={pct}%"));
    }
    let _ = app_handle.emit(
        "llm:progress",
        LlmProgressPayload {
            id: id.to_string(),
            job: job.to_string(),
            pct,
        },
    );
}

fn emit_complete(app_handle: &AppHandle, id: &str, job: &str, result: &str) {
    crate::app_logs::info(
        app_handle,
        "llm",
        format!("{job} completado para id={id}, caracteres={}", result.len()),
    );
    let _ = app_handle.emit(
        "llm:complete",
        LlmCompletePayload {
            id: id.to_string(),
            job: job.to_string(),
            result: result.to_string(),
        },
    );
}

fn emit_error(app_handle: &AppHandle, id: &str, job: &str, error: &str) {
    crate::app_logs::error(
        app_handle,
        "llm",
        format!("{job} falló para id={id}: {error}"),
    );
    let _ = app_handle.emit(
        "llm:error",
        LlmErrorPayload {
            id: id.to_string(),
            job: job.to_string(),
            error: error.to_string(),
        },
    );
}

// ---------------------------------------------------------------------------
// Result retrieval (for UI hydration after page reload)
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
pub struct LlmResultEntry {
    pub target_id: String,
    pub target_type: String,
    pub job_type: String,
    pub result: String,
    pub created_at: i64,
}

/// Fetch the latest LLM result for a given target (item or collection) and
/// optional job type. Returns `None` if no result is found.
pub fn get_latest_result(
    conn: &rusqlite::Connection,
    target_type: &str,
    target_id: &str,
    job_type: Option<&str>,
) -> Result<Option<LlmResultEntry>, String> {
    let row = if let Some(jt) = job_type {
        conn.query_row(
            "SELECT target_id, target_type, job_type, result, created_at
             FROM llm_results
             WHERE target_type = ?1 AND target_id = ?2 AND job_type = ?3
             ORDER BY created_at DESC LIMIT 1",
            params![target_type, target_id, jt],
            |row| {
                Ok(LlmResultEntry {
                    target_id: row.get(0)?,
                    target_type: row.get(1)?,
                    job_type: row.get(2)?,
                    result: row.get(3)?,
                    created_at: row.get(4)?,
                })
            },
        )
    } else {
        conn.query_row(
            "SELECT target_id, target_type, job_type, result, created_at
             FROM llm_results
             WHERE target_type = ?1 AND target_id = ?2
             ORDER BY created_at DESC LIMIT 1",
            params![target_type, target_id],
            |row| {
                Ok(LlmResultEntry {
                    target_id: row.get(0)?,
                    target_type: row.get(1)?,
                    job_type: row.get(2)?,
                    result: row.get(3)?,
                    created_at: row.get(4)?,
                })
            },
        )
    };

    match row {
        Ok(entry) => Ok(Some(entry)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to query llm_results: {e}")),
    }
}

/// Fetch all latest LLM results for a given target (one per job_type).
pub fn get_all_results_for_target(
    conn: &rusqlite::Connection,
    target_type: &str,
    target_id: &str,
) -> Result<Vec<LlmResultEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT target_id, target_type, job_type, result, created_at
             FROM llm_results
             WHERE target_type = ?1 AND target_id = ?2
             ORDER BY created_at DESC",
        )
        .map_err(|e| format!("Failed to prepare llm_results query: {e}"))?;

    let rows = stmt
        .query_map(params![target_type, target_id], |row| {
            Ok(LlmResultEntry {
                target_id: row.get(0)?,
                target_type: row.get(1)?,
                job_type: row.get(2)?,
                result: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| format!("Failed to query llm_results: {e}"))?;

    let mut results = Vec::new();
    let mut seen_job_types = std::collections::HashSet::new();
    for row in rows {
        if let Ok(entry) = row {
            // Keep only the latest result per job_type (DESC order means first is latest)
            if seen_job_types.insert(entry.job_type.clone()) {
                results.push(entry);
            }
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// Persist an LLM result to the database. Uses INSERT OR REPLACE so the
/// latest result per (target, job_type) pair is always kept.
fn persist_result(
    conn: &rusqlite::Connection,
    target_type: &str,
    target_id: &str,
    job_type: &str,
    result: &str,
) -> Result<(), String> {
    let id = format!("llr-{target_type}-{target_id}-{job_type}");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    conn.execute(
        "INSERT INTO llm_results (id, target_id, target_type, job_type, result, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET target_id=excluded.target_id, target_type=excluded.target_type, job_type=excluded.job_type, result=excluded.result, created_at=excluded.created_at",
        params![id, target_id, target_type, job_type, result, now],
    )
    .map_err(|e| format!("Failed to persist LLM result: {e}"))?;

    Ok(())
}

pub fn ensure_llm_results_schema(conn: &rusqlite::Connection) -> Result<(), String> {
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='llm_results' LIMIT 1",
            [],
            |_row| Ok(true),
        )
        .unwrap_or(false);

    if !table_exists {
        conn.execute_batch(
            "CREATE TABLE llm_results (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL,
                target_type TEXT NOT NULL CHECK(target_type IN ('asset', 'item', 'collection', 'unknown')),
                job_type TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_llm_results_target ON llm_results(target_id);
             CREATE INDEX IF NOT EXISTS idx_llm_results_target_typed ON llm_results(target_type, target_id, job_type);",
        )
        .map_err(|e| format!("Failed to create llm_results table: {e}"))?;
        return Ok(());
    }

    let has_target_type: bool = conn
        .prepare("SELECT target_type FROM llm_results LIMIT 0")
        .and_then(|mut stmt| {
            let _ = stmt.query_map([], |_| Ok(()));
            Ok(true)
        })
        .unwrap_or(false);

    if !has_target_type {
        conn.execute_batch(
            "BEGIN TRANSACTION;
             CREATE TABLE llm_results_v2 (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL,
                target_type TEXT NOT NULL CHECK(target_type IN ('asset', 'item', 'collection', 'unknown')),
                job_type TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );
             INSERT INTO llm_results_v2 (id, target_id, target_type, job_type, result, created_at)
             SELECT
                id,
                target_id,
                CASE
                    WHEN EXISTS (SELECT 1 FROM assets a WHERE a.id = llm_results.target_id) THEN 'asset'
                    WHEN EXISTS (SELECT 1 FROM items i WHERE i.id = llm_results.target_id) THEN 'item'
                    WHEN EXISTS (SELECT 1 FROM collections c WHERE c.id = llm_results.target_id) THEN 'collection'
                    ELSE 'unknown'
                END,
                job_type,
                result,
                CASE
                    WHEN created_at > 0 AND created_at < 1000000000000 THEN created_at * 1000
                    ELSE created_at
                END
             FROM llm_results;
             DROP TABLE llm_results;
             ALTER TABLE llm_results_v2 RENAME TO llm_results;
             CREATE INDEX IF NOT EXISTS idx_llm_results_target ON llm_results(target_id);
             CREATE INDEX IF NOT EXISTS idx_llm_results_target_typed ON llm_results(target_type, target_id, job_type);
             COMMIT;",
        )
        .map_err(|e| format!("Failed to migrate llm_results table: {e}"))?;
        return Ok(());
    }

    conn.execute_batch(
        "UPDATE llm_results
         SET target_type = 'unknown'
         WHERE target_type NOT IN ('asset', 'item', 'collection', 'unknown');
         UPDATE llm_results
         SET created_at = created_at * 1000
         WHERE created_at > 0 AND created_at < 1000000000000;
         CREATE INDEX IF NOT EXISTS idx_llm_results_target ON llm_results(target_id);
         CREATE INDEX IF NOT EXISTS idx_llm_results_target_typed ON llm_results(target_type, target_id, job_type);",
    )
    .map_err(|e| format!("Failed to normalize llm_results table: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Parse LLM triples JSON and store in the `triples` table
// ---------------------------------------------------------------------------

/// A single triple parsed from the LLM JSON response.
/// Fields use `#[serde(default)]` so incomplete triples (missing object, etc.)
/// deserialize with empty strings instead of failing the entire array.
/// Incomplete triples are filtered out after parsing.
#[derive(Clone, serde::Deserialize)]
struct LlmTriple {
    #[serde(default, alias = "sujeto")]
    subject: String,
    #[serde(default, alias = "predicado")]
    predicate: String,
    #[serde(default, alias = "objeto")]
    object: String,
}

static TRAILING_COMMA_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r",\s*([}\]])").expect("valid trailing comma regex"));
static MISSING_OBJECT_COMMA_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"}\s*\{").expect("valid missing object comma regex"));

impl LlmTriple {
    fn cleaned(mut self) -> Option<Self> {
        self.subject = self.subject.trim().to_string();
        self.predicate = self.predicate.trim().to_string();
        self.object = self.object.trim().to_string();

        if self.subject.is_empty() || self.predicate.is_empty() || self.object.is_empty() {
            return None;
        }

        Some(self)
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
        .trim_start_matches("javascript")
        .trim_start_matches("js")
        .trim();

    without_opening
        .strip_suffix("```")
        .unwrap_or(without_opening)
        .trim()
        .to_string()
}

fn normalize_jsonish(text: &str) -> String {
    let normalized_quotes = text
        .replace(['“', '”', '„', '‟'], "\"")
        .replace(['’', '‘', '‚', '‛'], "'");

    let without_trailing_commas = TRAILING_COMMA_RE
        .replace_all(normalized_quotes.trim(), "$1")
        .into_owned();

    MISSING_OBJECT_COMMA_RE
        .replace_all(&without_trailing_commas, "},{")
        .into_owned()
}

fn preview_for_log(text: &str, max_chars: usize) -> String {
    let sanitized = text.replace('\r', "\\r").replace('\n', "\\n");
    let mut chars = sanitized.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn extract_json_objects(text: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut escape = false;

    for (i, ch) in text.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }

            match ch {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(obj_start) = start.take() {
                        objects.push(text[obj_start..=i].to_string());
                    }
                }
            }
            _ => {}
        }
    }

    objects
}

fn parse_single_triple(raw: &str) -> Option<LlmTriple> {
    let normalized = normalize_jsonish(raw);
    serde_json::from_str::<LlmTriple>(&normalized)
        .ok()
        .and_then(LlmTriple::cleaned)
}

fn dedupe_triples(triples: Vec<LlmTriple>) -> Vec<LlmTriple> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for triple in triples {
        let key = (
            triple.subject.to_lowercase(),
            triple.predicate.to_lowercase(),
            triple.object.to_lowercase(),
        );
        if seen.insert(key) {
            deduped.push(triple);
        }
    }

    deduped
}

/// Parse the JSON array of triples returned by Gemma 4.
///
/// The LLM is prompted to return `[{"subject": ..., "predicate": ..., "object": ...}]`.
/// This function is tolerant: it strips markdown fences and trailing text,
/// and parses each triple individually so one bad entry doesn't spoil the rest.
fn parse_triples_json(raw: &str, log_prefix: &str) -> Vec<LlmTriple> {
    let content = strip_markdown_fences(raw);
    let normalized_content = normalize_jsonish(&content);

    let json_candidate = if let Some(start) = normalized_content.find('[') {
        if let Some(end) = normalized_content[start..].rfind(']') {
            normalized_content[start..=start + end].to_string()
        } else {
            normalized_content.clone()
        }
    } else if let (Some(start), Some(end)) =
        (normalized_content.find('{'), normalized_content.rfind('}'))
    {
        format!("[{}]", &normalized_content[start..=end])
    } else {
        normalized_content.clone()
    };

    // Try parsing the whole array first (fast path).
    // With #[serde(default)] on LlmTriple, incomplete triples become empty-string fields
    // instead of causing a parse error.
    match serde_json::from_str::<Vec<LlmTriple>>(&json_candidate) {
        Ok(triples) => dedupe_triples(triples.into_iter().filter_map(LlmTriple::cleaned).collect()),
        Err(_) => {
            // Fast path failed — parse each object individually so one bad triple
            // doesn't spoil the rest. Gemma sometimes omits fields or produces
            // malformed entries in the middle of an otherwise valid array.
            let valid_triples = dedupe_triples(
                extract_json_objects(&normalized_content)
                    .into_iter()
                    .filter_map(|obj| parse_single_triple(&obj))
                    .collect(),
            );

            if valid_triples.is_empty() {
                eprintln!("{log_prefix}[triples] failed to parse any triples");
                eprintln!(
                    "{log_prefix}[triples] normalized_preview=\"{}\", candidate_preview=\"{}\"",
                    preview_for_log(&normalized_content, 220),
                    preview_for_log(&json_candidate, 220),
                );
            } else {
                eprintln!(
                    "{log_prefix}[triples] parse fallback ok: parsed={}, object_candidates={}, candidate_preview=\"{}\"",
                    valid_triples.len(),
                    normalized_content.matches('{').count(),
                    preview_for_log(&json_candidate, 220),
                );
            }

            valid_triples
        }
    }
}

fn fn_uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn fn_now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Store parsed LLM triples into the `triples` table for an item-level job.
/// Deletes existing triples for the item before inserting new ones.
fn store_triples_for_item(
    conn: &rusqlite::Connection,
    item_id: &str,
    raw_json: &str,
    log_prefix: &str,
) -> Result<usize, String> {
    let triples = parse_triples_json(raw_json, log_prefix);

    // Delete old triples for this item (no asset_id filter => item-level)
    conn.execute("DELETE FROM triples WHERE item_id = ?1", params![item_id])
        .map_err(|e| format!("Failed to delete old triples for item: {e}"))?;

    let mut count = 0;
    for triple in &triples {
        conn.execute(
            "INSERT INTO triples (id, item_id, subject, predicate, object, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                fn_uuid_v4(),
                item_id,
                triple.subject,
                triple.predicate,
                triple.object,
                fn_now_millis(),
            ],
        )
        .map_err(|e| format!("Failed to insert triple: {e}"))?;
        count += 1;
    }
    Ok(count)
}

/// Store parsed LLM triples into the `triples` table for an asset-level job.
/// Deletes existing triples for the specific asset before inserting new ones.
fn store_triples_for_asset(
    conn: &rusqlite::Connection,
    item_id: &str,
    asset_id: &str,
    raw_json: &str,
    log_prefix: &str,
) -> Result<usize, String> {
    let triples = parse_triples_json(raw_json, log_prefix);

    // Delete old triples for this specific asset only
    conn.execute(
        "DELETE FROM triples WHERE item_id = ?1 AND asset_id = ?2",
        params![item_id, asset_id],
    )
    .map_err(|e| format!("Failed to delete old triples for asset: {e}"))?;

    let mut count = 0;
    for triple in &triples {
        conn.execute(
            "INSERT INTO triples (id, item_id, asset_id, subject, predicate, object, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                fn_uuid_v4(),
                item_id,
                asset_id,
                triple.subject,
                triple.predicate,
                triple.object,
                fn_now_millis(),
            ],
        )
        .map_err(|e| format!("Failed to insert triple: {e}"))?;
        count += 1;
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LlmQueue {
    sender: mpsc::Sender<LlmJob>,
    /// Shared flag set to `true` after the LLM engine initializes successfully.
    available: Arc<AtomicBool>,
    /// Path to the database, used for checking settings.
    db_path: PathBuf,
}

impl LlmQueue {
    pub fn new(db_path: PathBuf) -> (Self, mpsc::Receiver<LlmJob>) {
        let (sender, receiver) = mpsc::channel::<LlmJob>(64);
        let available = Arc::new(AtomicBool::new(false));
        (
            Self {
                sender,
                available: available.clone(),
                db_path,
            },
            receiver,
        )
    }

    pub fn submit(&self, job: LlmJob) -> Result<(), String> {
        self.sender
            .try_send(job)
            .map_err(|e| format!("LLM queue full or closed: {e}"))
    }

    /// Returns `true` if any LLM backend is available (local engine OR OpenRouter configured).
    pub fn is_available(&self) -> bool {
        if self.available.load(Ordering::Relaxed) {
            return true;
        }
        if self.local_model_can_initialize() {
            return true;
        }
        // Check if OpenRouter is configured
        self.is_openrouter_configured()
    }

    /// Report local availability without loading Gemma. Existing models are
    /// usable, and the default model can still auto-download on first local job.
    fn local_model_can_initialize(&self) -> bool {
        let conn = match rusqlite::Connection::open(&self.db_path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        local_model_can_initialize_from_conn(&conn, &self.db_path)
    }

    /// Check if OpenRouter is configured with an API key and mode is not `local`.
    fn is_openrouter_configured(&self) -> bool {
        let conn = match rusqlite::Connection::open(&self.db_path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let mode = settings::get_setting(&conn, "llm_mode").unwrap_or_default();
        if mode != "openrouter" && mode != "auto" {
            return false;
        }
        let key = settings::get_setting(&conn, "openrouter_api_key").unwrap_or_default();
        !key.is_empty()
    }

    /// Returns a clone of the availability flag for sharing with the worker.
    /// Used to signal engine readiness from the worker back to the main state.
    pub fn available_flag(&self) -> Arc<AtomicBool> {
        self.available.clone()
    }

    // `available` is only stored into by the local-ml lazy-init path; without the
    // local engine the remote worker never flips it, so allow it to be unused in
    // lean rather than renaming the parameter (keeps the signature/callers stable).
    #[cfg_attr(not(feature = "local-ml"), allow(unused_variables))]
    pub fn start_worker(
        db_path: PathBuf,
        mut receiver: mpsc::Receiver<LlmJob>,
        app_handle: AppHandle,
        available: Arc<AtomicBool>,
    ) {
        tauri::async_runtime::spawn(async move {
            // Open dedicated DB connection for the worker FIRST so we can read model settings
            let conn = match rusqlite::Connection::open(&db_path) {
                Ok(c) => {
                    let _ = c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;");
                    c
                }
                Err(e) => {
                    eprintln!("{LLM_LOCAL_PREFIX} Failed to open worker DB connection: {e}");
                    return;
                }
            };

            // Ensure app_settings table exists (idempotent) for reading LLM mode and model config
            if let Err(e) = conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS app_settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                 );",
            ) {
                eprintln!("{LLM_LOCAL_PREFIX} Warning: could not create app_settings table: {e}");
            }

            eprintln!(
                "{LLM_LOCAL_PREFIX} Local Gemma engine will initialize lazily on first local job"
            );
            #[cfg(feature = "local-ml")]
            let mut engine: Option<SharedLocalLlmEngine> = None;
            #[cfg(feature = "local-ml")]
            let mut init_error: Option<String> = None;

            // Ensure llm_results table exists and legacy rows are normalized.
            if let Err(e) = ensure_llm_results_schema(&conn) {
                eprintln!("{LLM_LOCAL_PREFIX} Warning: could not create llm_results table: {e}");
            }

            // Main worker loop
            while let Some(job) = receiver.recv().await {
                let job_name = job.job_name();
                let id = job.target_id().to_string();

                // Read LLM mode from settings on each job (< 1ms, allows hot-reload)
                let llm_mode =
                    settings::get_setting(&conn, "llm_mode").unwrap_or_else(|| "local".to_string());
                // Without the local engine `use_openrouter` is forced true and the
                // mode is never inspected; keep the read (it documents intent and
                // touches the DB consistently) but silence the unused-binding warning.
                #[cfg(not(feature = "local-ml"))]
                let _ = &llm_mode;

                let api_key =
                    settings::get_setting(&conn, "openrouter_api_key").unwrap_or_default();
                let remote_model = settings::get_setting(&conn, "openrouter_model")
                    .unwrap_or_else(|| "google/gemma-3-4b-it".to_string());
                #[cfg(feature = "local-ml")]
                let local_can_initialize = local_model_can_initialize_from_conn(&conn, &db_path);
                #[cfg(feature = "local-ml")]
                let use_openrouter = match llm_mode.as_str() {
                    "openrouter" => true,
                    "auto" => engine.is_none() && !local_can_initialize && !api_key.is_empty(),
                    _ => false, // "local" or unknown
                };
                // Without the local engine there is no local path; every job must
                // go through OpenRouter, so force the remote selector on.
                #[cfg(not(feature = "local-ml"))]
                let use_openrouter = true;
                let job_log_prefix = llm_job_prefix(use_openrouter, &job);

                emit_progress(&app_handle, &id, job_name, 10);

                let result = if use_openrouter {
                    // OpenRouter remote path

                    if api_key.is_empty() {
                        emit_error(
                            &app_handle,
                            &id,
                            job_name,
                            "OpenRouter API key no configurada. Andá a Configuración para agregarla.",
                        );
                        continue;
                    }

                    eprintln!("{job_log_prefix} Running job '{job_name}' for {id} via remote API");
                    let client = OpenRouterClient::new(api_key, remote_model);
                    match prepare_remote_job_request(&conn, &job, client.n_ctx()) {
                        Ok(request) => {
                            match client.generate(&request.prompt, request.max_tokens).await {
                                Ok(output) if request.truncate_to_sentence_boundary => {
                                    Ok(truncate_to_sentence_boundary(&output))
                                }
                                Ok(output) => Ok(output),
                                Err(e) => Err(e),
                            }
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    // Local engine path. Compiled only with the local engine; under
                    // `not(local-ml)` `use_openrouter` is always true so this arm is
                    // never taken (the block below is the unreachable tail).
                    #[cfg(not(feature = "local-ml"))]
                    {
                        unreachable!("local-ml disabled: use_openrouter is always true")
                    }
                    #[cfg(feature = "local-ml")]
                    {
                    if engine.is_none() {
                        let init_db_path = db_path.clone();
                        let init_app_handle = app_handle.clone();
                        eprintln!("{LLM_LOCAL_PREFIX} Initializing local Gemma engine on demand for job '{job_name}'");
                        match tokio::task::spawn_blocking(move || {
                            let init_conn =
                                rusqlite::Connection::open(&init_db_path).map_err(|e| {
                                    format!("Failed to open DB for lazy local LLM init: {e}")
                                })?;
                            get_or_init_local_gemma_engine(
                                &init_conn,
                                &init_db_path,
                                &init_app_handle,
                            )
                        })
                        .await
                        {
                            Ok(Ok(resolved_engine)) => {
                                eprintln!("{LLM_LOCAL_PREFIX} Engine ready (lazy local init)");
                                available.store(true, Ordering::Relaxed);
                                init_error = None;
                                engine = Some(resolved_engine);
                            }
                            Ok(Err(error)) => {
                                eprintln!("{LLM_LOCAL_PREFIX} Engine unavailable after lazy local init: {error}");
                                init_error = Some(format!(
                                    "Engine unavailable after lazy local init: {error}"
                                ));
                            }
                            Err(error) => {
                                eprintln!(
                                    "{LLM_LOCAL_PREFIX} Engine lazy local init panicked: {error}"
                                );
                                init_error =
                                    Some(format!("Engine lazy local init panicked: {error}"));
                            }
                        }
                    }

                    match &engine {
                        Some(e) => {
                            eprintln!(
                                "{job_log_prefix} Running job '{job_name}' for {id} via local engine"
                            );
                            tokio::task::block_in_place(|| {
                                let engine = e.lock().map_err(|error| {
                                    format!("Local Gemma engine lock poisoned: {error}")
                                })?;
                                process_job(&engine, &conn, &job)
                            })
                        }
                        None => {
                            if llm_mode == "auto" && !api_key.is_empty() {
                                let fallback_log_prefix = llm_job_prefix(true, &job);
                                eprintln!(
                                    "{fallback_log_prefix} Local Gemma unavailable; falling back to OpenRouter for job '{job_name}' on {id}"
                                );
                                let client = OpenRouterClient::new(api_key, remote_model);
                                match prepare_remote_job_request(&conn, &job, client.n_ctx()) {
                                    Ok(request) => {
                                        match client
                                            .generate(&request.prompt, request.max_tokens)
                                            .await
                                        {
                                            Ok(output) if request.truncate_to_sentence_boundary => {
                                                Ok(truncate_to_sentence_boundary(&output))
                                            }
                                            Ok(output) => Ok(output),
                                            Err(e) => Err(e),
                                        }
                                    }
                                    Err(e) => Err(e),
                                }
                            } else {
                                emit_error(
                                    &app_handle,
                                    &id,
                                    job_name,
                                    init_error.as_deref().unwrap_or(
                                        "LLM no disponible. Colocá un modelo GGUF en models/ o configurá OpenRouter.",
                                    ),
                                );
                                continue;
                            }
                        }
                    }
                    }
                };

                match result {
                    Ok(output) => {
                        // Persist result to database (non-fatal if it fails)
                        if let Err(e) =
                            persist_result(&conn, job.target_type(), &id, job_name, &output)
                        {
                            eprintln!("{job_log_prefix} Warning: failed to persist result for {id}/{job_name}: {e}");
                        }

                        // Parse triples from LLM response and store in `triples` table
                        // so the Semantic Triples section UI shows LLM-extracted triples.
                        match &job {
                            LlmJob::ExtractTriples { item_id } => {
                                match store_triples_for_item(&conn, item_id, &output, &job_log_prefix) {
                                    Ok(count) => eprintln!("{job_log_prefix} Stored {count} triples for item {item_id}"),
                                    Err(e) => eprintln!("{job_log_prefix} Warning: failed to store triples for item {item_id}: {e}"),
                                }
                            }
                            LlmJob::ExtractTriplesAsset { asset_id } => {
                                // Resolve item_id from asset_id for the triples table
                                match crate::nlp::lookup_item_id_for_asset(&conn, asset_id) {
                                    Ok(Some(item_id)) => {
                                        match store_triples_for_asset(&conn, &item_id, asset_id, &output, &job_log_prefix) {
                                            Ok(count) => eprintln!("{job_log_prefix} Stored {count} triples for asset {asset_id}"),
                                            Err(e) => eprintln!("{job_log_prefix} Warning: failed to store triples for asset {asset_id}: {e}"),
                                        }
                                    }
                                    Ok(None) => eprintln!("{job_log_prefix} Warning: no item_id found for asset {asset_id}, skipping triples storage"),
                                    Err(e) => eprintln!("{job_log_prefix} Warning: failed to lookup item_id for asset {asset_id}: {e}"),
                                }
                            }
                            _ => {} // Other job types don't produce triples
                        }

                        emit_progress(&app_handle, &id, job_name, 100);
                        emit_complete(&app_handle, &id, job_name, &output);
                    }
                    Err(e) => {
                        emit_error(&app_handle, &id, job_name, &e);
                    }
                }
            }

            eprintln!("{LLM_LOCAL_PREFIX} Worker loop ended — channel closed.");
        });
    }
}

// ---------------------------------------------------------------------------
// Job processing
// ---------------------------------------------------------------------------

/// Max tokens for generation per job type.
fn max_tokens_for(job: &LlmJob) -> i32 {
    match job {
        LlmJob::CorrectOcr { .. } | LlmJob::CorrectOcrAsset { .. } => 2048,
        LlmJob::ExtractEntities { .. }
        | LlmJob::ExtractEntitiesAsset { .. }
        | LlmJob::ConsolidateEntities { .. }
        | LlmJob::ConsolidateEntitiesAsset { .. } => 1024,
        LlmJob::ExtractTriples { .. } | LlmJob::ExtractTriplesAsset { .. } => 1024,
        LlmJob::Summarize { .. } | LlmJob::SummarizeAsset { .. } => 512,
        LlmJob::Classify { .. } => 256,
        LlmJob::Ask { .. } => 512,
    }
}

/// Truncate text to the last sentence boundary (period, exclamation, question mark)
/// so it doesn't cut mid-sentence. Used for summaries that get truncated by token limits.
fn truncate_to_sentence_boundary(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // If the text already ends with a sentence-ending punctuation, it's fine.
    if trimmed.ends_with('.')
        || trimmed.ends_with('!')
        || trimmed.ends_with('?')
        || trimmed.ends_with('。')
        || trimmed.ends_with('！')
    {
        return trimmed.to_string();
    }

    // Find the last sentence-ending punctuation and truncate there.
    // Search backwards for . ! ? 。 ！
    let sentence_enders = ['.', '!', '?', '。', '！'];
    if let Some(pos) = trimmed.rfind(sentence_enders) {
        // Include the punctuation character
        trimmed[..=pos].to_string()
    } else {
        // No sentence boundary found at all — return as-is (better than nothing)
        trimmed.to_string()
    }
}

// ---------------------------------------------------------------------------
// Text truncation for context safety
// ---------------------------------------------------------------------------

/// Conservative characters-per-token estimate for Latin-script text.
/// Gemma tokenizer averages ~3.5 chars/token for English/Spanish; using 3.0
/// provides a safety margin for multi-byte characters and template overhead.
const CHARS_PER_TOKEN_ESTIMATE: usize = 3;

/// Tokens reserved for prompt template instructions and formatting.
/// Each prompt wraps the text in instruction text (~50-150 tokens for Gemma
/// chat format markers + task instructions).
const TEMPLATE_OVERHEAD_TOKENS: i32 = 128;

/// Truncate text so that the resulting prompt + max_tokens fits within n_ctx.
/// Uses a conservative heuristic and cuts at sentence boundaries when possible.
pub(crate) fn truncate_text_for_context(n_ctx: u32, max_tokens: i32, text: &str) -> String {
    let budget_tokens = (n_ctx as i32) - max_tokens - TEMPLATE_OVERHEAD_TOKENS;
    if budget_tokens <= 0 {
        // Extremely small context — return first ~500 chars as a last resort
        return text.chars().take(500).collect();
    }
    let budget_chars = budget_tokens as usize * CHARS_PER_TOKEN_ESTIMATE;
    let text_chars = text.chars().count();
    if text_chars <= budget_chars {
        return text.to_string();
    }

    // Collect chars up to budget, then try to cut at the last sentence boundary
    let truncated: String = text.chars().take(budget_chars).collect();
    if let Some(pos) = truncated.rfind(|c: char| c == '.' || c == '\n' || c == '！' || c == '。')
    {
        // Keep up to and including the sentence boundary char
        truncated[..=pos].to_string()
    } else {
        truncated
    }
}

// ---------------------------------------------------------------------------
// Context gathering for Ask (FTS-based)
// ---------------------------------------------------------------------------

/// Maximum context size in characters for Ask queries (~2000 tokens budget).
const MAX_ASK_CONTEXT_CHARS: usize = 6000;

/// Maximum characters per individual document snippet (~400 tokens).
const MAX_SNIPPET_CHARS: usize = 1200;

#[cfg(feature = "local-ml")]
fn process_job(
    engine: &LlmEngine,
    conn: &rusqlite::Connection,
    job: &LlmJob,
) -> Result<String, String> {
    let n_ctx = engine.n_ctx();
    let log_prefix = llm_job_prefix(false, job);

    match job {
        LlmJob::CorrectOcr { item_id } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for OCR correction".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::ocr_correction(&truncated);
            engine.generate_ocr_correction(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::ExtractEntities { item_id } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for entity extraction".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::extract_entities(&truncated);
            engine.generate(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::ConsolidateEntities {
            item_id,
            candidate_entities_json,
        } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for entity consolidation".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::consolidate_entities(&truncated, candidate_entities_json);
            engine.generate(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::ExtractTriples { item_id } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for triple extraction".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::extract_triples(&truncated);
            engine.generate_triples(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::Summarize { item_id } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for summarization".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::summarize(&truncated);
            let result = engine.generate(&p, max_tokens_for(job), &log_prefix)?;
            Ok(truncate_to_sentence_boundary(&result))
        }

        LlmJob::Classify {
            item_id,
            categories,
        } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for classification".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::classify(&truncated, categories);
            engine.generate(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::Ask {
            collection_id,
            question,
        } => {
            // Gather context from FTS search over the collection
            let context = gather_collection_context(conn, collection_id, question)?;
            if context.is_empty() {
                return Err("No relevant documents found for this question".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &context);
            let p = prompt::question_answer(question, &truncated);
            engine.generate(&p, max_tokens_for(job), &log_prefix)
        }

        // ── Asset-level variants (single page/asset, avoids context overflow) ──
        LlmJob::CorrectOcrAsset { asset_id } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for OCR correction on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::ocr_correction(&truncated);
            engine.generate_ocr_correction(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::ExtractEntitiesAsset { asset_id } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for entity extraction on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::extract_entities(&truncated);
            engine.generate(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::ConsolidateEntitiesAsset {
            asset_id,
            candidate_entities_json,
        } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for entity consolidation on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::consolidate_entities(&truncated, candidate_entities_json);
            engine.generate(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::ExtractTriplesAsset { asset_id } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for triple extraction on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::extract_triples(&truncated);
            engine.generate_triples(&p, max_tokens_for(job), &log_prefix)
        }

        LlmJob::SummarizeAsset { asset_id } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for summarization on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens_for(job), &text);
            let p = prompt::summarize(&truncated);
            let result = engine.generate(&p, max_tokens_for(job), &log_prefix)?;
            Ok(truncate_to_sentence_boundary(&result))
        }
    }
}

/// Gathers relevant text snippets from a collection using FTS search.
///
/// Uses the existing `sanitize_fts5_query` to safely handle natural-language
/// questions, and retrieves full text via `text_provider::get_item_text`
/// instead of a broken LEFT JOIN on extrations.
fn gather_collection_context(
    conn: &rusqlite::Connection,
    collection_id: &str,
    question: &str,
) -> Result<String, String> {
    // Sanitize the question for FTS5 — natural-language queries contain
    // operators and noise that break FTS MATCH.
    let fts_query = crate::nlp::fts::sanitize_fts5_query(question);
    if fts_query.is_empty() {
        return Ok(String::new());
    }

    // Find matching item IDs via FTS (top 5 by relevance)
    let item_ids: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT i.id
                 FROM fts_items f
                 JOIN items i ON i.rowid = f.rowid
                 WHERE fts_items MATCH ?1 AND i.collection_id = ?2
                 ORDER BY rank
                 LIMIT 5",
            )
            .map_err(|e| format!("FTS query prepare failed: {e}"))?;

        let rows = stmt
            .query_map(params![fts_query, collection_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("FTS query failed: {e}"))?;

        rows.filter_map(|r| r.ok()).collect()
    };

    if item_ids.is_empty() {
        return Ok(String::new());
    }

    // For each matching item, retrieve full text via text_provider
    let mut context = String::new();
    for item_id in &item_ids {
        let title: String = conn
            .query_row(
                "SELECT title FROM items WHERE id = ?1",
                params![item_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "Unknown".to_string());

        let text = text_provider::get_item_text(conn, item_id).unwrap_or_default();
        if !text.is_empty() {
            // Truncate each snippet to stay within budget
            let display_text: String = if text.chars().count() > MAX_SNIPPET_CHARS {
                text.chars().take(MAX_SNIPPET_CHARS).collect()
            } else {
                text.clone()
            };

            let snippet = format!("--- {} ---\n{}\n\n", title, display_text);
            if context.len() + snippet.len() > MAX_ASK_CONTEXT_CHARS {
                // Budget exceeded — add what fits and stop
                let remaining = MAX_ASK_CONTEXT_CHARS.saturating_sub(context.len());
                if remaining > 0 {
                    context.push_str(&snippet[..remaining.min(snippet.len())]);
                }
                break;
            }
            context.push_str(&snippet);
        }
    }

    Ok(context)
}
struct RemoteJobRequest {
    prompt: String,
    max_tokens: i32,
    truncate_to_sentence_boundary: bool,
}

// ---------------------------------------------------------------------------
// Remote job preparation (OpenRouter)
// ---------------------------------------------------------------------------

/// Prepare a remote OpenRouter request without holding a DB connection across `.await`.
fn prepare_remote_job_request(
    conn: &rusqlite::Connection,
    job: &LlmJob,
    n_ctx: u32,
) -> Result<RemoteJobRequest, String> {
    let max_tokens = max_tokens_for(job);

    match job {
        LlmJob::CorrectOcr { item_id } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for OCR correction".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_ocr_correction(&truncated),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::ExtractEntities { item_id } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for entity extraction".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_extract_entities(&truncated),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::ConsolidateEntities {
            item_id,
            candidate_entities_json,
        } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for entity consolidation".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_consolidate_entities(&truncated, candidate_entities_json),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::ExtractTriples { item_id } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for triple extraction".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_extract_triples(&truncated),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::Summarize { item_id } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for summarization".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_summarize(&truncated),
                max_tokens,
                truncate_to_sentence_boundary: true,
            })
        }

        LlmJob::Classify {
            item_id,
            categories,
        } => {
            let text = text_provider::get_item_text(conn, item_id)?;
            if text.is_empty() {
                return Err("No text available for classification".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_classify(&truncated, categories),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::Ask {
            collection_id,
            question,
        } => {
            let context = gather_collection_context(conn, collection_id, question)?;
            if context.is_empty() {
                return Err("No relevant documents found for this question".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &context);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_question_answer(question, &truncated),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        // Asset-level variants
        LlmJob::CorrectOcrAsset { asset_id } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for OCR correction on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_ocr_correction(&truncated),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::ExtractEntitiesAsset { asset_id } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for entity extraction on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_extract_entities(&truncated),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::ConsolidateEntitiesAsset {
            asset_id,
            candidate_entities_json,
        } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for entity consolidation on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_consolidate_entities(&truncated, candidate_entities_json),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::ExtractTriplesAsset { asset_id } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for triple extraction on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_extract_triples(&truncated),
                max_tokens,
                truncate_to_sentence_boundary: false,
            })
        }

        LlmJob::SummarizeAsset { asset_id } => {
            let text = text_provider::get_asset_text(conn, asset_id)?;
            if text.is_empty() {
                return Err("No text available for summarization on this asset".to_string());
            }
            let truncated = truncate_text_for_context(n_ctx, max_tokens, &text);
            Ok(RemoteJobRequest {
                prompt: prompt::raw_summarize(&truncated),
                max_tokens,
                truncate_to_sentence_boundary: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_llm_schema_fixture(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, created_at INTEGER NOT NULL);",
        )
        .unwrap();
    }

    #[test]
    fn ensure_llm_results_schema_migrates_legacy_rows_with_target_type_and_ms_timestamps() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        setup_llm_schema_fixture(&conn);

        conn.execute(
            "INSERT INTO collections (id, name, description, created_at, updated_at) VALUES (?1, ?2, NULL, ?3, ?3)",
            params!["collection-1", "Collection", 1_710_000_000_000_i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?1, ?2, ?3, NULL, ?4, ?4)",
            params!["shared-id", "Item", "collection-1", 1_710_000_000_000_i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "asset-1",
                "shared-id",
                "/tmp/a.pdf",
                "pdf",
                1_710_000_000_000_i64
            ],
        )
        .unwrap();

        conn.execute_batch(
            "CREATE TABLE llm_results (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL,
                job_type TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            INSERT INTO llm_results (id, target_id, job_type, result, created_at) VALUES
                ('legacy-item', 'shared-id', 'summarize', 'item summary', 1710000000),
                ('legacy-asset', 'asset-1', 'summarize', 'asset summary', 1710000000123),
                ('legacy-unknown', 'ghost-1', 'ask', 'ghost answer', 1710000001);",
        )
        .unwrap();

        ensure_llm_results_schema(&conn).unwrap();

        let rows: Vec<(String, String, i64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT target_id, target_type, created_at FROM llm_results ORDER BY id ASC",
                )
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };

        assert_eq!(rows.len(), 3);
        assert!(rows.contains(&(
            "shared-id".to_string(),
            "item".to_string(),
            1_710_000_000_000_i64
        )));
        assert!(rows.contains(&(
            "asset-1".to_string(),
            "asset".to_string(),
            1_710_000_000_123_i64
        )));
        assert!(rows.contains(&(
            "ghost-1".to_string(),
            "unknown".to_string(),
            1_710_000_001_000_i64
        )));
    }

    #[test]
    fn persist_and_query_results_are_scoped_by_target_type() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        ensure_llm_results_schema(&conn).unwrap();

        persist_result(&conn, "item", "shared-id", "summarize", "item summary").unwrap();
        persist_result(&conn, "asset", "shared-id", "summarize", "asset summary").unwrap();

        let item_result = get_latest_result(&conn, "item", "shared-id", Some("summarize"))
            .unwrap()
            .unwrap();
        let asset_result = get_latest_result(&conn, "asset", "shared-id", Some("summarize"))
            .unwrap()
            .unwrap();

        assert_eq!(item_result.target_type, "item");
        assert_eq!(item_result.result, "item summary");
        assert_eq!(asset_result.target_type, "asset");
        assert_eq!(asset_result.result, "asset summary");
        assert!(item_result.created_at >= 1_000_000_000_000_i64);
        assert!(asset_result.created_at >= 1_000_000_000_000_i64);
    }

    #[test]
    fn resolve_model_path_returns_app_models_dir_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("app.db");
        let resolved = resolve_model_path(&db_path);
        // Should point to {tmp}/models/gemma-4-E2B-it-Q4_K_M.gguf
        assert_eq!(resolved, tmp.path().join("models").join(MODEL_FILENAME));
        assert!(!resolved.exists());
    }

    #[test]
    fn resolve_model_path_prefers_existing_file_in_models_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("app.db");
        let models_dir = tmp.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let model_file = models_dir.join(MODEL_FILENAME);
        std::fs::write(&model_file, b"fake-gguf").unwrap();

        let resolved = resolve_model_path(&db_path);
        assert_eq!(resolved, model_file);
        assert!(resolved.exists());
    }

    #[test]
    fn get_local_model_info_reports_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("app.db");
        let info = get_local_model_info(&db_path);
        assert!(!info.exists);
        assert!(info.available);
        assert!(info.can_auto_download);
        assert!(info.disabled_reason.is_none());
        assert!(info.size_bytes.is_none());
        assert_eq!(info.filename, MODEL_FILENAME);
        assert!(info.path.ends_with(MODEL_FILENAME));
        assert_eq!(info.source_url, DEFAULT_MODEL_SOURCE_URL);
    }

    #[test]
    fn get_local_model_info_reports_found_file_with_size() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("app.db");
        let models_dir = tmp.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let model_file = models_dir.join(MODEL_FILENAME);
        let content = b"fake gguf content for testing";
        std::fs::write(&model_file, content).unwrap();

        let info = get_local_model_info(&db_path);
        assert!(info.exists);
        assert!(info.available);
        assert!(!info.can_auto_download);
        assert!(info.disabled_reason.is_none());
        assert_eq!(info.size_bytes, Some(content.len() as u64));
        assert_eq!(info.filename, MODEL_FILENAME);
        assert_eq!(info.path, model_file.to_string_lossy().to_string());
        assert_eq!(info.source_url, DEFAULT_MODEL_SOURCE_URL);
    }

    #[test]
    fn get_local_model_info_reports_custom_missing_as_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("app.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        crate::settings::set_setting(&conn, LOCAL_MODEL_FILENAME_KEY, "custom-model.gguf").unwrap();
        crate::settings::set_setting(
            &conn,
            LOCAL_MODEL_SOURCE_URL_KEY,
            "https://example.invalid/custom-model.gguf",
        )
        .unwrap();

        let info = get_local_model_info(&db_path);

        assert!(!info.exists);
        assert!(!info.available);
        assert!(!info.can_auto_download);
        assert!(info.disabled_reason.is_none());
        assert_eq!(info.filename, "custom-model.gguf");
    }

    #[test]
    fn resolve_model_path_finds_installed_legacy_default_file() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("app.db");
        let models_dir = tmp.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let legacy_file = models_dir.join("google_gemma-3-4b-it-Q4_K_M.gguf");
        std::fs::write(&legacy_file, b"legacy gguf").unwrap();

        let resolved = resolve_model_path(&db_path);
        assert_eq!(resolved, legacy_file);
        assert!(resolved.exists());
    }

    #[test]
    fn resolve_model_path_uses_custom_filename_from_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("app.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        crate::settings::set_setting(&conn, LOCAL_MODEL_FILENAME_KEY, "custom-model.gguf").unwrap();

        let resolved = resolve_model_path(&db_path);
        assert_eq!(
            resolved,
            tmp.path().join("models").join("custom-model.gguf")
        );
        assert!(!resolved.exists());
    }

    #[test]
    fn resolve_model_path_prefers_custom_filename_when_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("app.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        crate::settings::set_setting(&conn, LOCAL_MODEL_FILENAME_KEY, "custom-model.gguf").unwrap();

        let models_dir = tmp.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let custom_file = models_dir.join("custom-model.gguf");
        std::fs::write(&custom_file, b"fake").unwrap();

        let resolved = resolve_model_path(&db_path);
        assert_eq!(resolved, custom_file);
        assert!(resolved.exists());
    }

    #[test]
    fn resolve_local_model_filename_migrates_previous_default_name() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        crate::settings::set_setting(
            &conn,
            LOCAL_MODEL_FILENAME_KEY,
            "google_gemma-3-4b-it-Q4_K_M.gguf",
        )
        .unwrap();

        assert_eq!(resolve_local_model_filename(Some(&conn)), MODEL_FILENAME);
    }

    #[test]
    fn resolve_local_model_source_url_migrates_previous_default_url() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        crate::settings::set_setting(
            &conn,
            LOCAL_MODEL_SOURCE_URL_KEY,
            "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/main/google_gemma-3-4b-it-Q4_K_M.gguf",
        )
        .unwrap();

        assert_eq!(
            resolve_local_model_source_url(Some(&conn)),
            DEFAULT_MODEL_SOURCE_URL
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn persist_default_model_setting_migrations_updates_legacy_defaults_only() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        crate::settings::set_setting(&conn, LOCAL_MODEL_FILENAME_KEY, "gemma-3-4b-it-Q4_K_M.gguf")
            .unwrap();
        crate::settings::set_setting(
            &conn,
            LOCAL_MODEL_SOURCE_URL_KEY,
            "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-Q4_K_M.gguf?download=true",
        )
        .unwrap();

        persist_default_model_setting_migrations(&conn).unwrap();

        assert_eq!(
            crate::settings::get_setting(&conn, LOCAL_MODEL_FILENAME_KEY).as_deref(),
            Some(MODEL_FILENAME)
        );
        assert_eq!(
            crate::settings::get_setting(&conn, LOCAL_MODEL_SOURCE_URL_KEY).as_deref(),
            Some(DEFAULT_MODEL_SOURCE_URL)
        );
    }
}
