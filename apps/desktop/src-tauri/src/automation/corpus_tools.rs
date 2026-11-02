//! The corpus edit tools' adapter: the services of [`crate::corpus`] under the
//! agent tool contract.
//!
//! Like [`super::jobs`] for processing, no corpus rule lives here: validation,
//! revision fencing and the reserved metadata keys belong to the services the
//! app's own views use. This module adds what only the agent path needs: a
//! required revision on every update, idempotent creation keyed by the
//! client's request id, stable error codes, and the announcement that lets the
//! app's views refresh after a write they did not make (S6).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::approvals::{Approval, ApprovalAction, ApprovalRequest, ApprovalStore};
use super::jobs::JobsError;
use crate::corpus::{self, CollectionDto, CollectionPatch, CorpusError, ItemPatch};
use crate::processing::repository;

/// Longest request id an assistant may send.
const REQUEST_ID_MAX_CHARS: usize = 128;

/// What changed, for the app's views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusChange {
    /// `collection` or `item`.
    pub kind: &'static str,
    pub id: String,
    /// The collection the changed record belongs to (itself for a collection).
    pub collection_id: String,
}

/// Called after every successful write; installed by the app (an event the
/// views listen to). Never called while a connection is open in a transaction.
pub type CorpusNotifier = Arc<dyn Fn(CorpusChange) + Send + Sync>;

/// The endpoint's announcer of corpus writes.
#[derive(Default)]
pub struct CorpusEvents {
    notifier: Mutex<Option<CorpusNotifier>>,
}

impl CorpusEvents {
    pub fn set_notifier(&self, notifier: CorpusNotifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    pub fn notify(&self, change: CorpusChange) {
        let notifier = self.notifier.lock().ok().and_then(|slot| slot.clone());
        if let Some(notifier) = notifier {
            notifier(change);
        }
    }
}

/// `collections_create`'s payload.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedCollection {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: i64,
    /// The revision `collections_update` names.
    pub updated_at: i64,
    /// False when this request id already created this collection.
    pub created: bool,
}

/// An item after `items_update`, in the shape `items_get` uses.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdatedItem {
    pub id: String,
    pub title: String,
    pub collection_id: String,
    pub metadata: serde_json::Value,
    pub created_at: i64,
    /// The revision the next update names.
    pub updated_at: i64,
}

fn refusal(code: &'static str, message: impl Into<String>) -> JobsError {
    JobsError {
        code,
        message: message.into(),
    }
}

/// The stable code and Spanish sentence for a refused corpus write. The
/// service's detail names only ids and limits; an unavailable archive's
/// detail may name a path, so it stays in the log.
fn from_corpus(error: CorpusError) -> JobsError {
    match error {
        CorpusError::InvalidArgument(detail) => refusal(
            "invalid_argument",
            format!("La petición no es válida: {detail}."),
        ),
        CorpusError::NotFound(detail) => {
            refusal("not_found", format!("No existe en este archivo: {detail}."))
        }
        CorpusError::RevisionConflict(detail) => refusal(
            super::jobs::ERROR_REVISION_CONFLICT,
            format!("Cambió desde la última lectura ({detail}). Volvé a leerlo y reintentá con su `updatedAt` como `revision`."),
        ),
        CorpusError::Unavailable(detail) => {
            log::warn!(target: "automation", "a corpus write failed: {detail}");
            JobsError::unavailable()
        }
    }
}

fn check_request_id(request_id: &str) -> Result<(), JobsError> {
    if request_id.trim().is_empty()
        || request_id.chars().count() > REQUEST_ID_MAX_CHARS
        || request_id.chars().any(char::is_control)
    {
        return Err(refusal(
            "invalid_argument",
            format!(
                "`requestId` tiene que tener entre 1 y {REQUEST_ID_MAX_CHARS} caracteres visibles."
            ),
        ));
    }
    Ok(())
}

fn payload_hash(parts: &[&str]) -> String {
    hex::encode(Sha256::digest(parts.join("\u{0}").as_bytes()))
}

/// What an assistant proposes for one page.
pub struct TextReplacement {
    pub item_id: String,
    pub asset_id: String,
    /// The page's `textHash` as the assistant read it.
    pub text_hash: String,
    pub text: String,
}

/// Most files one import request may list.
pub const MAX_IMPORT_FILES: usize = 200;

/// Most files `import_sources_list` shows per folder.
pub const MAX_LISTED_FILES: usize = 500;

/// One importable file, as the assistant sees it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub name: String,
    pub kind: crate::corpus::ingest::FileKind,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
}

/// One enabled folder and its importable files.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceFolder {
    pub path: String,
    pub files: Vec<SourceSummary>,
    pub truncated: bool,
}

/// `import_sources_list`'s payload.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSources {
    pub folders: Vec<SourceFolder>,
}

/// What an assistant asks to import.
pub struct ImportRequest {
    pub collection_id: String,
    pub folder: String,
    pub files: Vec<String>,
}

/// The data directory of the archive at `db_path`: it holds the database and
/// `assets/`.
fn data_dir_of(db_path: &std::path::Path) -> Result<PathBuf, JobsError> {
    db_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(JobsError::unavailable)
}

/// Characters of a proposed text the owner's dialog shows.
const PREVIEW_CHARS: usize = 400;

/// `documents_index`'s payload.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexedItem {
    pub item_id: String,
    pub indexed: bool,
}

fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// The corpus of one archive, as the edit tools see it.
#[derive(Clone)]
pub struct CorpusArchive {
    db_path: PathBuf,
    events: Arc<CorpusEvents>,
}

impl CorpusArchive {
    pub fn new(db_path: impl Into<PathBuf>, events: Arc<CorpusEvents>) -> Self {
        Self {
            db_path: db_path.into(),
            events,
        }
    }

    fn connection(&self) -> Result<Connection, JobsError> {
        crate::db::open::open_archive_connection(&self.db_path).map_err(|error| {
            log::warn!(target: "automation", "could not open the archive for a corpus write: {error}");
            JobsError::unavailable()
        })
    }

    /// Create a collection once per client request id: a retry with the same
    /// arguments returns the same collection, other arguments are refused.
    pub fn create_collection(
        &self,
        client_id: &str,
        request_id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<CreatedCollection, JobsError> {
        check_request_id(request_id)?;
        let key = format!("agent:{client_id}:{request_id}");
        let hash = payload_hash(&["collection_create", name, description.unwrap_or("")]);
        let conn = self.connection()?;
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| JobsError::unavailable())?;
        let result = (|| {
            if let Some(previous) = repository::find_request(&conn, &key)
                .map_err(|error| from_corpus(CorpusError::Unavailable(error)))?
            {
                if previous.payload_hash != hash {
                    return Err(refusal(
                        "invalid_argument",
                        format!("El pedido {request_id} ya se usó con otros datos: usá otro `requestId`."),
                    ));
                }
                let stored: CreatedCollection = previous
                    .response_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str(json).ok())
                    .ok_or_else(JobsError::unavailable)?;
                return Ok(CreatedCollection {
                    created: false,
                    ..stored
                });
            }
            let created: CollectionDto =
                corpus::create_collection(&conn, name, description, None).map_err(from_corpus)?;
            let answer = CreatedCollection {
                id: created.id,
                name: created.name,
                description: created.description,
                created_at: created.created_at,
                updated_at: created.updated_at,
                created: true,
            };
            let json = serde_json::to_string(&answer).map_err(|_| JobsError::unavailable())?;
            repository::record_request(
                &conn,
                &key,
                "collection_create",
                None,
                &hash,
                &json,
                repository::now_ms(),
            )
            .map_err(|error| from_corpus(CorpusError::Unavailable(error)))?;
            Ok(answer)
        })();
        match result {
            Ok(answer) => {
                conn.execute_batch("COMMIT")
                    .map_err(|_| JobsError::unavailable())?;
                if answer.created {
                    self.events.notify(CorpusChange {
                        kind: "collection",
                        id: answer.id.clone(),
                        collection_id: answer.id.clone(),
                    });
                }
                Ok(answer)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    /// Ask the owner to replace one page's recognized text. Everything that
    /// can be refused is refused here, before a dialog opens: an unknown or
    /// foreign file, a page without recognized text (audio), a text that
    /// changed since it was read, an empty or unchanged proposal.
    pub fn request_text_replacement(
        &self,
        approvals: &ApprovalStore,
        client_id: &str,
        client_name: &str,
        proposal: &TextReplacement,
    ) -> Result<Approval, JobsError> {
        let TextReplacement {
            item_id,
            asset_id,
            text_hash,
            text,
        } = proposal;
        let (item_id, asset_id, text_hash, text) = (
            item_id.as_str(),
            asset_id.as_str(),
            text_hash.as_str(),
            text.as_str(),
        );
        let max = corpus::MAX_PAGE_TEXT_CHARS;
        if text.trim().is_empty() || text.chars().count() > max {
            return Err(refusal(
                "invalid_argument",
                format!("El texto nuevo tiene que tener entre 1 y {max} caracteres."),
            ));
        }
        let conn = self.connection()?;
        let collection: Option<String> = conn
            .query_row(
                "SELECT c.name FROM assets a
                 JOIN items i ON i.id = a.item_id
                 JOIN collections c ON c.id = i.collection_id
                 WHERE a.id = ?1 AND a.item_id = ?2",
                params![asset_id, item_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| JobsError::unavailable())?;
        let collection = collection.ok_or_else(|| {
            refusal(
                "not_found",
                format!("El ítem «{item_id}» no tiene un archivo con el id «{asset_id}»."),
            )
        })?;
        let current: Option<String> = conn
            .query_row(
                "SELECT text_content FROM extractions WHERE asset_id = ?1
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                params![asset_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| JobsError::unavailable())?;
        let Some(current) = current.filter(|text| !text.trim().is_empty()) else {
            return Err(refusal(
                "invalid_argument",
                "Esta página no tiene texto reconocido que reemplazar (una transcripción de audio no se puede reemplazar así).",
            ));
        };
        if sha256_hex(&current) != text_hash {
            return Err(refusal(
                super::jobs::ERROR_REVISION_CONFLICT,
                "El texto de la página cambió desde que lo leíste: volvé a leerlo con documents_get_text y usá su `textHash`.",
            ));
        }
        if current == text {
            return Err(refusal(
                "invalid_argument",
                "El texto nuevo es igual al actual: no hay nada que reemplazar.",
            ));
        }
        Ok(approvals.request(ApprovalRequest {
            client_id: client_id.to_string(),
            client_name: client_name.to_string(),
            action: ApprovalAction::ReplaceText {
                item_id: item_id.to_string(),
                asset_id: asset_id.to_string(),
                text_hash: text_hash.to_string(),
                new_text_hash: sha256_hex(text),
            },
            job_id: String::new(),
            revision: 0,
            operations: vec!["replace_text".to_string()],
            collections: vec![collection],
            files: 1,
            preview: Some(text.chars().take(PREVIEW_CHARS).collect()),
            text: Some(text.to_string()),
            import_plan: None,
        }))
    }

    /// The supported files directly inside each enabled folder.
    pub fn import_sources(&self, folders: &[PathBuf]) -> Result<ImportSources, JobsError> {
        let data_dir = data_dir_of(&self.db_path)?;
        let mut listed = Vec::with_capacity(folders.len());
        for folder in folders {
            match crate::corpus::ingest::list_folder(folder, folders, &data_dir, MAX_LISTED_FILES) {
                Ok(listing) => listed.push(SourceFolder {
                    path: folder.to_string_lossy().to_string(),
                    files: listing
                        .files
                        .into_iter()
                        .map(|file| SourceSummary {
                            name: file.name,
                            kind: file.kind,
                            size_bytes: file.size_bytes,
                            modified_at: file.modified_at,
                        })
                        .collect(),
                    truncated: listing.truncated,
                }),
                // A folder that vanished or cannot be read is left out.
                Err(error) => {
                    log::warn!(target: "automation", "an import folder could not be listed: {error}")
                }
            }
        }
        Ok(ImportSources { folders: listed })
    }

    /// Ask the owner to import files from one enabled folder into a
    /// collection. Every file is checked first; one that cannot be imported
    /// refuses the whole request, so the dialog only ever lists real files.
    pub fn request_import(
        &self,
        approvals: &ApprovalStore,
        client_id: &str,
        client_name: &str,
        folders: &[PathBuf],
        request: &ImportRequest,
    ) -> Result<Approval, JobsError> {
        use crate::corpus::ingest::{check_source, SourceLimits};
        if request.files.is_empty() || request.files.len() > MAX_IMPORT_FILES {
            return Err(refusal(
                "invalid_argument",
                format!("`files` tiene que nombrar entre 1 y {MAX_IMPORT_FILES} archivos."),
            ));
        }
        let conn = self.connection()?;
        let collection: Option<String> = conn
            .query_row(
                "SELECT name FROM collections WHERE id = ?1",
                params![request.collection_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| JobsError::unavailable())?;
        let collection = collection.ok_or_else(|| {
            refusal(
                "not_found",
                format!(
                    "No existe una colección con el id «{}».",
                    request.collection_id
                ),
            )
        })?;
        let folder = std::fs::canonicalize(&request.folder)
            .ok()
            .filter(|real| folders.contains(real))
            .ok_or_else(|| {
                refusal(
                    "invalid_argument",
                    "Esa carpeta no está habilitada para importar: usá una de las que lista import_sources_list.",
                )
            })?;
        let data_dir = data_dir_of(&self.db_path)?;
        let mut names = request.files.clone();
        names.sort();
        names.dedup();
        let mut plan = Vec::with_capacity(names.len());
        for name in &names {
            let plain = !name.is_empty()
                && name != "."
                && name != ".."
                && !name.contains('/')
                && !name.contains('\\');
            if !plain {
                return Err(refusal(
                    "invalid_argument",
                    format!("«{name}» no es un nombre de archivo de la carpeta (sin subcarpetas ni rutas)."),
                ));
            }
            let source = check_source(
                &folder.join(name).to_string_lossy(),
                folders,
                &data_dir,
                &SourceLimits::default(),
            )
            .map_err(|reason| {
                let code = reason.split(':').next().unwrap_or("invalid");
                refusal(
                    "invalid_argument",
                    format!("«{name}» no se puede importar ({code}): revisá la lista de import_sources_list."),
                )
            })?;
            plan.push(source);
        }
        let mut digest = Sha256::new();
        digest.update(request.collection_id.as_bytes());
        for file in &plan {
            digest.update(b"\0");
            digest.update(file.path.to_string_lossy().as_bytes());
            digest.update(format!("|{}|{:?}", file.size_bytes, file.modified_at).as_bytes());
        }
        let plan_hash = hex::encode(digest.finalize());
        Ok(approvals.request(ApprovalRequest {
            client_id: client_id.to_string(),
            client_name: client_name.to_string(),
            action: ApprovalAction::ImportFiles {
                collection_id: request.collection_id.clone(),
                files: plan.len(),
                plan_hash,
            },
            job_id: String::new(),
            revision: 0,
            operations: vec!["import_files".to_string()],
            collections: vec![collection],
            files: plan.len() as i64,
            preview: None,
            text: None,
            import_plan: Some(plan),
        }))
    }

    /// Refresh one item's search entry.
    pub fn reindex(&self, item_id: &str) -> Result<IndexedItem, JobsError> {
        let conn = self.connection()?;
        corpus::reindex_item(&conn, item_id).map_err(from_corpus)?;
        Ok(IndexedItem {
            item_id: item_id.to_string(),
            indexed: true,
        })
    }

    /// Rename or re-describe a collection at the revision the assistant read.
    pub fn update_collection(
        &self,
        collection_id: &str,
        revision: i64,
        patch: &CollectionPatch,
    ) -> Result<CollectionDto, JobsError> {
        let conn = self.connection()?;
        let updated = corpus::update_collection(&conn, collection_id, patch, Some(revision))
            .map_err(from_corpus)?;
        self.events.notify(CorpusChange {
            kind: "collection",
            id: updated.id.clone(),
            collection_id: updated.id.clone(),
        });
        Ok(updated)
    }

    /// Retitle an item or replace its metadata fields at the revision the
    /// assistant read; the app's reserved keys are kept and cannot be written.
    pub fn update_item(
        &self,
        item_id: &str,
        revision: i64,
        title: Option<String>,
        metadata: Option<BTreeMap<String, String>>,
    ) -> Result<UpdatedItem, JobsError> {
        let conn = self.connection()?;
        let patch = ItemPatch {
            title,
            metadata,
            metadata_json: None,
        };
        let updated =
            corpus::update_item(&conn, item_id, &patch, Some(revision)).map_err(from_corpus)?;
        self.events.notify(CorpusChange {
            kind: "item",
            id: updated.id.clone(),
            collection_id: updated.collection_id.clone(),
        });
        let metadata = updated
            .metadata
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or(serde_json::Value::Null);
        Ok(UpdatedItem {
            id: updated.id,
            title: updated.title,
            collection_id: updated.collection_id,
            metadata: visible_metadata(metadata),
            created_at: updated.created_at,
            updated_at: updated.updated_at,
        })
    }
}

/// What an assistant sees of an item's metadata after an edit: its own
/// fields. The reserved keys are the app's bookkeeping and stay stored.
fn visible_metadata(metadata: serde_json::Value) -> serde_json::Value {
    match metadata {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(key, _)| !key.starts_with(corpus::commands::RESERVED_METADATA_PREFIX))
                .collect(),
        ),
        other => other,
    }
}
