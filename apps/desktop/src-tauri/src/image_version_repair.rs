//! Maintenance: images whose `assets.path` shows an older version than the one
//! their layout was computed on.
//!
//! A mass push from another device once rewrote `assets.path` of hundreds of
//! images to an older `_vN` file while the layout kept describing the latest
//! crop. This module finds those assets and, for the ones a person confirms,
//! points the asset back at the version its layout belongs to.
//!
//! Two steps. [`scan`] only reads: it lists what could be repaired and counts
//! everything else by the reason it is left alone. [`apply`] writes, and only
//! what was confirmed: it judges every confirmed asset again against the
//! database and the files as they are now, refuses the whole batch when a
//! single proposal no longer holds, and writes the rest in one transaction.
//!
//! No file is ever written, moved or deleted here. A repair changes
//! `assets.path` and adds one history row; completing the origin of an old
//! layout writes its `source_path` and `source_sha256`. Whether a file
//! "corresponds" to a layout is never decided here: that is
//! [`layout_version`], the rule every reader of a layout uses.

use crate::asset_history::{self, sha256_hex, ProcessEvent};
use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use crate::derived_source::{file_facts, layout_version, LayoutSource, LayoutVersion};
use crate::image_versions::{list_family, ImageVersion};
use crate::path_utils::resolve_asset_path;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use tauri::{AppHandle, Emitter, State};

/// `assets.type` of the only assets that have versions.
const IMAGE_ASSET_TYPE: &str = "image";

/// Sent while a scan runs: `{ done, total }`, in assets.
const PROGRESS_EVENT: &str = "image-version-repairs:progress";
/// What every view that shows an asset listens to (`lib/item-view-image-edit.ts`).
const IMAGE_UPDATED_EVENT: &str = "asset:image-updated";
/// A scan reports its progress once every this many assets, and at the end.
const PROGRESS_STEP: usize = 20;

/// Start of the error [`apply`] answers when a confirmed proposal no longer
/// holds. The frontend recognises it and asks for a new scan.
pub(crate) const STALE_PROPOSAL_ERROR: &str = "stale_proposal";

/// An image asset with a layout: everything the review reads from the database.
#[derive(Debug, Clone)]
pub(crate) struct LayoutRow {
    asset_id: String,
    item_id: String,
    item_title: Option<String>,
    path: String,
    source_sha256: Option<String>,
    image_width: i64,
    image_height: i64,
    /// `layouts.created_at`, milliseconds: every OCR run rewrites it, so it is
    /// when the stored layout was computed.
    created_at: i64,
}

impl LayoutRow {
    fn stamp(&self) -> Option<&str> {
        self.source_sha256
            .as_deref()
            .filter(|stamp| !stamp.is_empty())
    }

    fn source(&self) -> LayoutSource<'_> {
        LayoutSource {
            source_sha256: self.source_sha256.as_deref(),
            image_width: self.image_width,
            image_height: self.image_height,
        }
    }
}

/// What the review concludes about one asset.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Finding {
    /// The file shown is the one the layout belongs to. `completable`: the
    /// layout has no stamp and no other file of the family could be its
    /// source, so the stamp can be filled in from the file shown.
    Matches { completable: bool },
    /// The layout belongs to exactly one other file, a later version.
    Repair {
        shown: Box<ImageVersion>,
        proposed: Box<ImageVersion>,
    },
    /// The image shown is newer than the layout: the layout must be run again.
    NeedsLayoutRerun,
    /// Two or more other files could be the layout's source.
    Ambiguous,
    /// A later version exists, but no file corresponds to the layout.
    NoCandidate,
    /// The only candidate was written after an unstamped layout was computed,
    /// so the layout cannot have come from it.
    CandidateAfterLayout,
    /// The file `assets.path` names is not on disk, or cannot be reached.
    FileMissing,
}

fn classify(row: &LayoutRow, family: &[ImageVersion]) -> Finding {
    let layout = row.source();
    let corresponds = |version: &ImageVersion| {
        layout_version(IMAGE_ASSET_TYPE, &layout, Some(&version.facts)) == LayoutVersion::Matches
    };

    let Some(shown) = family.iter().find(|version| version.current) else {
        return Finding::FileMissing;
    };
    let others: Vec<&ImageVersion> = family
        .iter()
        .filter(|version| !version.current && corresponds(version))
        .collect();

    if corresponds(shown) {
        return Finding::Matches {
            completable: row.stamp().is_none() && others.is_empty(),
        };
    }
    match others.as_slice() {
        [] => {
            if family
                .iter()
                .all(|version| version.version <= shown.version)
            {
                Finding::NeedsLayoutRerun
            } else {
                Finding::NoCandidate
            }
        }
        [candidate] => {
            if candidate.version <= shown.version {
                // The person edited the image after the layout was computed.
                Finding::NeedsLayoutRerun
            } else if row.stamp().is_none() && candidate.modified_ms > row.created_at {
                Finding::CandidateAfterLayout
            } else {
                Finding::Repair {
                    shown: Box::new(shown.clone()),
                    proposed: Box::new((*candidate).clone()),
                }
            }
        }
        _ => Finding::Ambiguous,
    }
}

/// Judges one asset against the files on disk. Hashes the family only when the
/// layout carries a stamp to compare the hashes with.
fn inspect(row: &LayoutRow, data_dir: &Path) -> Finding {
    match list_family(&row.path, data_dir, row.stamp().is_some()) {
        Ok(family) => classify(row, &family),
        Err(_) => Finding::FileMissing,
    }
}

/// One file of a proposal, as the person is shown it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairFile {
    pub file_name: String,
    /// `N` of `<stem>_vN`; `0` for the original.
    pub version: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl RepairFile {
    fn of(version: &ImageVersion) -> Self {
        Self {
            file_name: file_name(&version.path),
            version: version.version,
            width: version.facts.width,
            height: version.facts.height,
        }
    }
}

fn file_name(stored_path: &str) -> String {
    stored_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(stored_path)
        .to_string()
}

/// A proposal to show the version the layout belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathRepair {
    pub asset_id: String,
    pub item_id: String,
    /// The item's title, or the file name when it has none.
    pub name: String,
    pub shown: RepairFile,
    pub proposed: RepairFile,
    /// Names exactly this proposal; [`apply`] takes it back with the asset id.
    pub token: String,
}

/// A proposal to record, on a layout written before the stamps, the file it
/// was computed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCompletion {
    pub asset_id: String,
    pub name: String,
    pub file_name: String,
    pub token: String,
}

/// How many assets the review left in each state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairCounts {
    /// Image assets with a layout: everything that was looked at.
    pub scanned: usize,
    pub matching: usize,
    pub repairable: usize,
    pub needs_layout_rerun: usize,
    pub ambiguous: usize,
    pub no_candidate: usize,
    pub candidate_after_layout: usize,
    pub file_missing: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairScan {
    pub counts: RepairCounts,
    pub repairs: Vec<PathRepair>,
    pub completions: Vec<SourceCompletion>,
}

/// A token names one proposal by everything it rests on: a proposal computed
/// later from another path, another candidate or another layout has another
/// token. The frontend hands it back unread, so it never sends a path.
fn token(kind: &str, row: &LayoutRow, target_path: &str) -> String {
    sha256_hex(
        [
            kind,
            &row.asset_id,
            &row.path,
            target_path,
            row.stamp().unwrap_or(""),
            &row.image_width.to_string(),
            &row.image_height.to_string(),
        ]
        .join("\n")
        .as_bytes(),
    )
}

fn repair_token(row: &LayoutRow, proposed: &ImageVersion) -> String {
    token("repair", row, &proposed.path)
}

fn completion_token(row: &LayoutRow) -> String {
    token("complete", row, &row.path)
}

fn display_name(row: &LayoutRow) -> String {
    row.item_title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map_or_else(|| file_name(&row.path), str::to_string)
}

/// Reviews `rows` against the files under `data_dir`. Reads only.
///
/// One asset at a time: nothing of an asset outlives its turn except a
/// proposal, so memory does not grow with the archive. `progress(done, total)`
/// is called now and then and once at the end.
pub(crate) fn scan(
    rows: &[LayoutRow],
    data_dir: &Path,
    progress: &mut dyn FnMut(usize, usize),
) -> RepairScan {
    let mut result = RepairScan::default();
    let total = rows.len();
    for (index, row) in rows.iter().enumerate() {
        result.counts.scanned += 1;
        match inspect(row, data_dir) {
            Finding::Matches { completable } => {
                result.counts.matching += 1;
                if completable {
                    result.completions.push(SourceCompletion {
                        asset_id: row.asset_id.clone(),
                        name: display_name(row),
                        file_name: file_name(&row.path),
                        token: completion_token(row),
                    });
                }
            }
            Finding::Repair { shown, proposed } => {
                result.counts.repairable += 1;
                result.repairs.push(PathRepair {
                    asset_id: row.asset_id.clone(),
                    item_id: row.item_id.clone(),
                    name: display_name(row),
                    shown: RepairFile::of(&shown),
                    token: repair_token(row, &proposed),
                    proposed: RepairFile::of(&proposed),
                });
            }
            Finding::NeedsLayoutRerun => result.counts.needs_layout_rerun += 1,
            Finding::Ambiguous => result.counts.ambiguous += 1,
            Finding::NoCandidate => result.counts.no_candidate += 1,
            Finding::CandidateAfterLayout => result.counts.candidate_after_layout += 1,
            Finding::FileMissing => result.counts.file_missing += 1,
        }
        let done = index + 1;
        if done % PROGRESS_STEP == 0 || done == total {
            progress(done, total);
        }
    }
    result
}

const LAYOUT_ROWS_SQL: &str = "SELECT a.id, a.item_id, i.title, a.path,
            l.source_sha256, l.image_width, l.image_height, l.created_at
       FROM assets a
       JOIN layouts l ON l.asset_id = a.id
       LEFT JOIN items i ON i.id = a.item_id
      WHERE a.type = 'image' AND a.parent_asset_id IS NULL";

fn layout_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LayoutRow> {
    Ok(LayoutRow {
        asset_id: row.get(0)?,
        item_id: row.get(1)?,
        item_title: row.get(2)?,
        path: row.get(3)?,
        source_sha256: row.get(4)?,
        image_width: row.get(5)?,
        image_height: row.get(6)?,
        created_at: row.get(7)?,
    })
}

/// Every image that is its own document and has a layout. PDFs and the pages
/// of a PDF are never read: they have no version family.
pub(crate) fn all_layout_rows(conn: &Connection) -> Result<Vec<LayoutRow>, String> {
    let mut statement = conn
        .prepare(&format!("{LAYOUT_ROWS_SQL} ORDER BY a.id"))
        .map_err(|e| format!("Failed to read the image layouts: {e}"))?;
    let rows = statement
        .query_map([], layout_row)
        .map_err(|e| format!("Failed to read the image layouts: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read the image layouts: {e}"))?;
    Ok(rows)
}

/// The rows of `asset_ids` that are still images with a layout.
fn layout_rows_of(conn: &Connection, asset_ids: &[String]) -> Result<Vec<LayoutRow>, String> {
    let mut statement = conn
        .prepare(&format!("{LAYOUT_ROWS_SQL} AND a.id = ?1"))
        .map_err(|e| format!("Failed to read the image layouts: {e}"))?;
    let mut rows = Vec::new();
    for asset_id in asset_ids {
        let found = statement
            .query_map([asset_id], layout_row)
            .map_err(|e| format!("Failed to read the image layouts: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read the image layouts: {e}"))?;
        rows.extend(found);
    }
    Ok(rows)
}

/// A proposal the person confirmed: the asset and the token the scan gave it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedProposal {
    pub asset_id: String,
    pub token: String,
}

struct PlannedRepair {
    row: LayoutRow,
    to_path: String,
    event: ProcessEvent,
}

struct PlannedCompletion {
    row: LayoutRow,
    sha256: String,
}

/// Everything [`write_plan`] will write, judged and hashed beforehand so the
/// transaction itself reads no file.
pub(crate) struct RepairPlan {
    repairs: Vec<PlannedRepair>,
    completions: Vec<PlannedCompletion>,
}

/// What a repair leaves in the views: one `asset:image-updated` per asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairedAsset {
    pub item_id: String,
    pub asset_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairApplied {
    pub repaired: Vec<RepairedAsset>,
    pub completed: usize,
}

fn stale(asset_ids: &[&str]) -> String {
    format!("{STALE_PROPOSAL_ERROR}: {}", asset_ids.join(", "))
}

fn confirmed_ids(confirmed: &[ConfirmedProposal]) -> Vec<String> {
    let mut seen = HashSet::new();
    confirmed
        .iter()
        .filter(|proposal| seen.insert(proposal.asset_id.as_str()))
        .map(|proposal| proposal.asset_id.clone())
        .collect()
}

/// The asset ids [`plan`] needs the rows of.
pub(crate) fn planned_asset_ids(
    repairs: &[ConfirmedProposal],
    completions: &[ConfirmedProposal],
) -> Vec<String> {
    let all: Vec<ConfirmedProposal> = repairs.iter().chain(completions).cloned().collect();
    confirmed_ids(&all)
}

fn sha256_of(stored_path: &str, data_dir: &Path) -> Result<String, String> {
    file_facts(&resolve_asset_path(stored_path, data_dir), true)?
        .sha256
        .ok_or_else(|| format!("No hash for {stored_path}"))
}

/// Judges every confirmed proposal again, against `rows` as they were just
/// read and the files as they are now. A proposal whose token no longer comes
/// out — the path changed, the layout was run again, a file appeared or went
/// away — refuses the whole batch.
pub(crate) fn plan(
    rows: &[LayoutRow],
    data_dir: &Path,
    repairs: &[ConfirmedProposal],
    completions: &[ConfirmedProposal],
) -> Result<RepairPlan, String> {
    let row_of = |asset_id: &str| rows.iter().find(|row| row.asset_id == asset_id);
    let mut refused: Vec<&str> = Vec::new();
    let mut planned = RepairPlan {
        repairs: Vec::new(),
        completions: Vec::new(),
    };

    let mut seen = HashSet::new();
    for confirmed in repairs {
        if !seen.insert(confirmed.asset_id.as_str()) {
            continue;
        }
        let holds = row_of(&confirmed.asset_id).and_then(|row| match inspect(row, data_dir) {
            Finding::Repair { shown, proposed }
                if repair_token(row, &proposed) == confirmed.token =>
            {
                Some((row, shown, proposed))
            }
            _ => None,
        });
        let Some((row, shown, proposed)) = holds else {
            refused.push(&confirmed.asset_id);
            continue;
        };
        let event = ProcessEvent::new(
            &row.asset_id,
            asset_history::PROCESS_IMAGE_VERSION_REPAIR,
            asset_history::TOOL_IMAGE_VERSION_REPAIR,
        )
        .params(json!({
            "fromVersion": shown.version,
            "toVersion": proposed.version,
            "fromPath": shown.path,
            "toPath": proposed.path,
        }))
        .input_sha256(Some(sha256_of(&shown.path, data_dir)?))
        .output_sha256(Some(sha256_of(&proposed.path, data_dir)?));
        planned.repairs.push(PlannedRepair {
            row: row.clone(),
            to_path: proposed.path.clone(),
            event,
        });
    }

    let mut seen = HashSet::new();
    for confirmed in completions {
        if !seen.insert(confirmed.asset_id.as_str()) {
            continue;
        }
        let holds = row_of(&confirmed.asset_id).filter(|row| {
            inspect(row, data_dir) == Finding::Matches { completable: true }
                && completion_token(row) == confirmed.token
        });
        let Some(row) = holds else {
            refused.push(&confirmed.asset_id);
            continue;
        };
        planned.completions.push(PlannedCompletion {
            row: row.clone(),
            sha256: sha256_of(&row.path, data_dir)?,
        });
    }

    if refused.is_empty() {
        Ok(planned)
    } else {
        Err(stale(&refused))
    }
}

/// Writes a plan in one transaction: every row or none.
///
/// Each statement names the state the plan was judged on, so a row that
/// changed between the judging and the write refuses the batch instead of
/// being overwritten. `assets` and `layouts` are synced tables and these are
/// edits a person confirmed: they are captured like any other edit.
pub(crate) fn write_plan(conn: &Connection, plan: &RepairPlan) -> Result<RepairApplied, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin the image version repair: {e}"))?;
    let mut applied = RepairApplied::default();

    for repair in &plan.repairs {
        let row = &repair.row;
        let changed = tx
            .execute(
                "UPDATE assets SET path = ?1
                  WHERE id = ?2 AND path = ?3
                    AND type = 'image' AND parent_asset_id IS NULL
                    AND EXISTS (SELECT 1 FROM layouts
                                 WHERE asset_id = ?2
                                   AND image_width = ?4 AND image_height = ?5
                                   AND source_sha256 IS ?6)",
                params![
                    repair.to_path,
                    row.asset_id,
                    row.path,
                    row.image_width,
                    row.image_height,
                    row.source_sha256,
                ],
            )
            .map_err(|e| format!("Failed to repair the image version: {e}"))?;
        if changed != 1 {
            return Err(stale(&[&row.asset_id]));
        }
        asset_history::record(&tx, &repair.event)?;
        applied.repaired.push(RepairedAsset {
            item_id: row.item_id.clone(),
            asset_id: row.asset_id.clone(),
            path: repair.to_path.clone(),
        });
    }

    for completion in &plan.completions {
        let row = &completion.row;
        let changed = tx
            .execute(
                "UPDATE layouts SET source_path = ?1, source_sha256 = ?2
                  WHERE asset_id = ?3
                    AND image_width = ?4 AND image_height = ?5
                    AND source_sha256 IS ?6
                    AND EXISTS (SELECT 1 FROM assets WHERE id = ?3 AND path = ?1)",
                params![
                    row.path,
                    completion.sha256,
                    row.asset_id,
                    row.image_width,
                    row.image_height,
                    row.source_sha256,
                ],
            )
            .map_err(|e| format!("Failed to complete the layout source: {e}"))?;
        if changed != 1 {
            return Err(stale(&[&row.asset_id]));
        }
        applied.completed += 1;
    }

    tx.commit()
        .map_err(|e| format!("Failed to commit the image version repair: {e}"))?;
    Ok(applied)
}

#[derive(Clone, Serialize)]
struct ScanProgress {
    done: usize,
    total: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageUpdated<'a> {
    item_id: &'a str,
    asset_id: &'a str,
    path: &'a str,
}

fn with_ui_conn<T>(
    db: &AppDbState,
    read: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock poisoned: {e}"))?;
    read(&conn)
}

/// Reviews every image that has a layout and lists what could be repaired.
/// Writes nothing: stopping it half way loses nothing.
#[tauri::command]
pub async fn scan_image_version_repairs(
    app: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<RepairScan, String> {
    let db = db.inner().clone();
    let rows = run_blocking_db_task(move || with_ui_conn(&db, all_layout_rows)).await?;
    let data_dir = crate::path_utils::data_dir(&app)?;
    // The database is released by now: reading and hashing files blocks no query.
    tokio::task::spawn_blocking(move || {
        scan(&rows, &data_dir, &mut |done, total| {
            let _ = app.emit(PROGRESS_EVENT, ScanProgress { done, total });
        })
    })
    .await
    .map_err(|e| format!("Image version review task failed: {e}"))
}

/// Applies the proposals a person confirmed, all of them or none.
///
/// Only asset ids and the tokens of the scan come in; the paths are worked out
/// again here from the database and the files.
#[tauri::command]
pub async fn apply_image_version_repairs(
    repairs: Vec<ConfirmedProposal>,
    completions: Vec<ConfirmedProposal>,
    app: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<RepairApplied, String> {
    let db = db.inner().clone();
    let data_dir = crate::path_utils::data_dir(&app)?;
    let asset_ids = planned_asset_ids(&repairs, &completions);

    let read_db = db.clone();
    let rows =
        run_blocking_db_task(move || with_ui_conn(&read_db, |c| layout_rows_of(c, &asset_ids)))
            .await?;
    // Judged and hashed with the database released.
    let planned =
        tokio::task::spawn_blocking(move || plan(&rows, &data_dir, &repairs, &completions))
            .await
            .map_err(|e| format!("Image version repair task failed: {e}"))??;
    let applied =
        run_blocking_db_task(move || with_ui_conn(&db, |c| write_plan(c, &planned))).await?;

    // The thumbnail of an image is named after its path, so the views that
    // reload on this event ask for a new one; nothing is cleared by hand.
    for asset in &applied.repaired {
        let _ = app.emit(
            IMAGE_UPDATED_EVENT,
            ImageUpdated {
                item_id: &asset.item_id,
                asset_id: &asset.asset_id,
                path: &asset.path,
            },
        );
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_orient::test_support::{camera_jpeg, encode_jpeg, upright_quadrants};
    use crate::sync::blobs::hash_file;
    use std::path::PathBuf;

    const DIR: &str = "assets/col-1/item-1";
    /// A layout computed long after every fixture file was written.
    const LATER: i64 = i64::MAX / 2;

    struct Archive {
        data: tempfile::TempDir,
        conn: Connection,
    }

    impl Archive {
        fn new() -> Self {
            let data = tempfile::tempdir().expect("tempdir");
            std::fs::create_dir_all(data.path().join(DIR)).expect("item dir");
            let conn = Connection::open_in_memory().expect("db");
            conn.execute_batch(
                "CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL);
                 CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL,
                    type TEXT NOT NULL, sort_index INTEGER NOT NULL DEFAULT 0, size INTEGER,
                    parent_asset_id TEXT, page_number INTEGER, created_at INTEGER NOT NULL);
                 CREATE TABLE layouts (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL UNIQUE,
                    regions TEXT NOT NULL, blocks TEXT NOT NULL, model TEXT NOT NULL,
                    image_width INTEGER NOT NULL, image_height INTEGER NOT NULL,
                    created_at INTEGER NOT NULL, source_path TEXT, source_sha256 TEXT);
                 CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL,
                    text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL,
                    created_at INTEGER NOT NULL, source_path TEXT, source_sha256 TEXT);
                 CREATE TABLE annotations (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL,
                    page INTEGER NOT NULL, kind TEXT NOT NULL, color TEXT NOT NULL,
                    x REAL NOT NULL, y REAL NOT NULL, width REAL NOT NULL, height REAL NOT NULL,
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
                 INSERT INTO items (id, title) VALUES ('item-1', 'Acta 1921');",
            )
            .expect("schema");
            conn.execute_batch(asset_history::TEST_DDL)
                .expect("history");
            Self { data, conn }
        }

        fn dir(&self) -> PathBuf {
            self.data.path().join(DIR)
        }

        /// A JPEG of `width`×`height`; `salt` makes files of one size differ.
        fn file(&self, name: &str, width: u32, height: u32, salt: &[u8]) -> PathBuf {
            let mut bytes = encode_jpeg(upright_quadrants(width, height));
            bytes.extend_from_slice(salt);
            let path = self.dir().join(name);
            std::fs::write(&path, bytes).expect("write fixture");
            path
        }

        fn asset(&self, id: &str, asset_type: &str, parent: Option<&str>, name: &str) {
            self.conn
                .execute(
                    "INSERT INTO assets (id, item_id, path, type, parent_asset_id, created_at)
                     VALUES (?1, 'item-1', ?2, ?3, ?4, 1)",
                    params![id, format!("{DIR}/{name}"), asset_type, parent],
                )
                .expect("asset");
        }

        fn image(&self, id: &str, name: &str) {
            self.asset(id, "image", None, name);
        }

        fn layout(&self, asset_id: &str, width: i64, height: i64, sha: Option<&str>, at: i64) {
            self.conn
                .execute(
                    "INSERT INTO layouts (id, asset_id, regions, blocks, model, image_width,
                        image_height, created_at, source_path, source_sha256)
                     VALUES (?1, ?2, '[{\"r\":1}]', '[{\"b\":2}]', 'glm-ocr', ?3, ?4, ?5, NULL, ?6)",
                    params![format!("lay-{asset_id}"), asset_id, width, height, at, sha],
                )
                .expect("layout");
        }

        fn scan(&self) -> RepairScan {
            let rows = all_layout_rows(&self.conn).expect("rows");
            scan(&rows, self.data.path(), &mut |_, _| {})
        }

        fn apply(
            &self,
            repairs: &[ConfirmedProposal],
            completions: &[ConfirmedProposal],
        ) -> Result<RepairApplied, String> {
            let ids = planned_asset_ids(repairs, completions);
            let rows = layout_rows_of(&self.conn, &ids)?;
            let planned = plan(&rows, self.data.path(), repairs, completions)?;
            write_plan(&self.conn, &planned)
        }

        fn path_of(&self, asset_id: &str) -> String {
            self.conn
                .query_row("SELECT path FROM assets WHERE id = ?1", [asset_id], |row| {
                    row.get(0)
                })
                .expect("path")
        }

        /// Every row of `table`, every column, as text.
        fn dump(&self, table: &str) -> Vec<String> {
            let mut statement = self
                .conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .expect("prepare");
            let columns = statement.column_count();
            statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| row.get::<_, rusqlite::types::Value>(index))
                        .collect::<Result<Vec<_>, _>>()
                        .map(|values| format!("{values:?}"))
                })
                .expect("query")
                .map(Result::unwrap)
                .collect()
        }

        /// Name, size, modification time and hash of every file of the item.
        fn files(&self) -> Vec<(String, u64, std::time::SystemTime, String)> {
            let mut files: Vec<_> = std::fs::read_dir(self.dir())
                .expect("read dir")
                .map(|entry| {
                    let entry = entry.expect("entry");
                    let meta = entry.metadata().expect("meta");
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        meta.len(),
                        meta.modified().expect("mtime"),
                        hash_file(&entry.path()).expect("hash").0,
                    )
                })
                .collect();
            files.sort();
            files
        }

        fn events(&self) -> Vec<(String, String, Option<String>)> {
            let mut statement = self
                .conn
                .prepare(
                    "SELECT asset_id, process, params_json FROM asset_process_events
                      ORDER BY asset_id",
                )
                .expect("prepare");
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .expect("query")
                .map(Result::unwrap)
                .collect()
        }
    }

    fn confirm(repair: &PathRepair) -> ConfirmedProposal {
        ConfirmedProposal {
            asset_id: repair.asset_id.clone(),
            token: repair.token.clone(),
        }
    }

    fn confirm_all(scan: &RepairScan) -> Vec<ConfirmedProposal> {
        scan.repairs.iter().map(confirm).collect()
    }

    /// The incident: the asset shows `_v4`, the layout is the one of `_v11`.
    fn reverted(archive: &Archive, id: &str, stem: &str) {
        archive.file(&format!("{stem}.jpg"), 96, 128, b"");
        archive.file(&format!("{stem}_v4.jpg"), 96, 128, b"v4");
        archive.file(&format!("{stem}_v11.jpg"), 40, 60, b"");
        archive.image(id, &format!("{stem}_v4.jpg"));
        archive.layout(id, 40, 60, None, LATER);
    }

    #[test]
    fn an_image_showing_an_older_version_than_its_layout_is_proposed_and_repaired() {
        let archive = Archive::new();
        reverted(&archive, "a1", "IMG_3006");
        archive
            .conn
            .execute_batch(
                "INSERT INTO extractions VALUES ('e1', 'a1', 'texto', 'glm-ocr', 0.9, 5, NULL, NULL);
                 INSERT INTO annotations VALUES ('n1', 'a1', 1, 'rectangle', '#f00', 1, 2, 3, 4, 5, 6);",
            )
            .expect("derived rows");

        let found = archive.scan();

        assert_eq!(
            found.counts,
            RepairCounts {
                scanned: 1,
                repairable: 1,
                ..RepairCounts::default()
            }
        );
        let [repair] = found.repairs.as_slice() else {
            panic!("one proposal expected: {:?}", found.repairs);
        };
        assert_eq!(repair.name, "Acta 1921");
        assert_eq!(
            (&repair.shown, &repair.proposed),
            (
                &RepairFile {
                    file_name: "IMG_3006_v4.jpg".to_string(),
                    version: 4,
                    width: Some(96),
                    height: Some(128),
                },
                &RepairFile {
                    file_name: "IMG_3006_v11.jpg".to_string(),
                    version: 11,
                    width: Some(40),
                    height: Some(60),
                },
            )
        );
        assert!(found.completions.is_empty());

        // The scan wrote nothing.
        assert_eq!(archive.path_of("a1"), format!("{DIR}/IMG_3006_v4.jpg"));
        assert!(archive.events().is_empty());

        let files = archive.files();
        let derived = ["layouts", "extractions", "annotations"].map(|table| archive.dump(table));

        let applied = archive.apply(&confirm_all(&found), &[]).expect("apply");

        assert_eq!(
            applied.repaired,
            [RepairedAsset {
                item_id: "item-1".to_string(),
                asset_id: "a1".to_string(),
                path: format!("{DIR}/IMG_3006_v11.jpg"),
            }]
        );
        assert_eq!(archive.path_of("a1"), format!("{DIR}/IMG_3006_v11.jpg"));
        assert_eq!(
            archive.files(),
            files,
            "no file is written, moved or deleted"
        );
        assert_eq!(
            ["layouts", "extractions", "annotations"].map(|table| archive.dump(table)),
            derived,
            "layout, text and annotations stay as they were"
        );

        let events = archive.events();
        let [(asset_id, process, params)] = events.as_slice() else {
            panic!("one history row expected: {events:?}");
        };
        assert_eq!(
            (asset_id.as_str(), process.as_str()),
            ("a1", "image_version_repair")
        );
        let params: serde_json::Value =
            serde_json::from_str(params.as_deref().expect("params")).expect("json");
        assert_eq!(params["fromVersion"], 4);
        assert_eq!(params["toVersion"], 11);
        let (input, output): (String, String) = archive
            .conn
            .query_row(
                "SELECT input_sha256, output_sha256 FROM asset_process_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("hashes");
        assert_eq!(
            input,
            hash_file(&archive.dir().join("IMG_3006_v4.jpg"))
                .expect("hash")
                .0
        );
        assert_eq!(
            output,
            hash_file(&archive.dir().join("IMG_3006_v11.jpg"))
                .expect("hash")
                .0
        );

        // Repaired: the next scan finds the layout on the image shown.
        let after = archive.scan();
        assert_eq!((after.counts.matching, after.counts.repairable), (1, 0));
    }

    #[test]
    fn two_versions_with_the_size_of_an_unstamped_layout_are_ambiguous() {
        let archive = Archive::new();
        archive.file("scan.jpg", 96, 128, b"");
        archive.file("scan_v2.jpg", 40, 60, b"a");
        archive.file("scan_v3.jpg", 40, 60, b"b");
        archive.image("a1", "scan.jpg");
        archive.layout("a1", 40, 60, None, LATER);

        let found = archive.scan();

        assert!(found.repairs.is_empty());
        assert_eq!(found.counts.ambiguous, 1);
        assert_eq!(found.counts.repairable, 0);
    }

    #[test]
    fn the_latest_version_with_a_stale_layout_needs_a_layout_run_not_a_repair() {
        let archive = Archive::new();
        // No file has the layout's size.
        archive.file("a.jpg", 96, 128, b"");
        archive.file("a_v2.jpg", 80, 100, b"");
        archive.image("a1", "a_v2.jpg");
        archive.layout("a1", 50, 50, None, LATER);
        // The layout is the one of an older version: edited after the OCR.
        archive.file("b.jpg", 96, 128, b"");
        archive.file("b_v2.jpg", 40, 60, b"");
        archive.file("b_v5.jpg", 30, 30, b"");
        archive.image("a2", "b_v5.jpg");
        archive.layout("a2", 40, 60, None, LATER);

        let found = archive.scan();

        assert!(found.repairs.is_empty());
        assert_eq!(found.counts.needs_layout_rerun, 2);
    }

    #[test]
    fn a_candidate_older_than_the_version_shown_is_never_proposed() {
        let archive = Archive::new();
        archive.file("scan.jpg", 96, 128, b"");
        archive.file("scan_v2.jpg", 40, 60, b"");
        archive.file("scan_v3.jpg", 80, 100, b"");
        archive.file("scan_v4.jpg", 70, 90, b"");
        // Not the latest, and the only file of the layout's size is older.
        archive.image("a1", "scan_v3.jpg");
        archive.layout("a1", 40, 60, None, LATER);

        let found = archive.scan();

        assert!(found.repairs.is_empty());
        assert_eq!(found.counts.needs_layout_rerun, 1);
    }

    #[test]
    fn a_later_version_that_matches_nothing_leaves_no_candidate() {
        let archive = Archive::new();
        archive.file("scan.jpg", 96, 128, b"");
        archive.file("scan_v2.jpg", 80, 100, b"");
        archive.image("a1", "scan.jpg");
        archive.layout("a1", 40, 60, None, LATER);

        let found = archive.scan();

        assert!(found.repairs.is_empty());
        assert_eq!(found.counts.no_candidate, 1);
    }

    #[test]
    fn a_stamped_layout_picks_its_file_by_hash_among_versions_of_one_size() {
        let archive = Archive::new();
        archive.file("scan.jpg", 40, 60, b"");
        archive.file("scan_v2.jpg", 40, 60, b"two");
        let source = archive.file("scan_v3.jpg", 40, 60, b"three");
        archive.file("scan_v4.jpg", 40, 60, b"four");
        archive.image("a1", "scan.jpg");
        let sha = hash_file(&source).expect("hash").0;
        // A stamp needs no date: the hash names the file.
        archive.layout("a1", 40, 60, Some(&sha), 0);

        let found = archive.scan();

        let [repair] = found.repairs.as_slice() else {
            panic!("one proposal expected: {:?}", found);
        };
        assert_eq!(repair.proposed.file_name, "scan_v3.jpg");
        archive.apply(&confirm_all(&found), &[]).expect("apply");
        assert_eq!(archive.path_of("a1"), format!("{DIR}/scan_v3.jpg"));
    }

    #[test]
    fn a_candidate_written_after_an_unstamped_layout_is_not_its_source() {
        let archive = Archive::new();
        archive.file("scan.jpg", 96, 128, b"");
        archive.file("scan_v2.jpg", 40, 60, b"");
        archive.image("a1", "scan.jpg");
        // Computed in 1970: before any file of today.
        archive.layout("a1", 40, 60, None, 1_000);

        let found = archive.scan();

        assert!(found.repairs.is_empty());
        assert_eq!(found.counts.candidate_after_layout, 1);
    }

    #[test]
    fn a_missing_file_and_a_tagged_photo_are_counted_and_left_alone() {
        let archive = Archive::new();
        archive.file("gone.jpg", 96, 128, b"");
        archive.image("a1", "gone_v2.jpg");
        archive.layout("a1", 96, 128, None, LATER);
        // An unstamped layout on a tagged photo was computed sideways: it is
        // the latest file, so the layout must be run again.
        std::fs::write(
            archive.dir().join("photo.jpg"),
            camera_jpeg(&upright_quadrants(64, 32), 6),
        )
        .expect("write photo");
        archive.image("a2", "photo.jpg");
        archive.layout("a2", 64, 32, None, LATER);

        let found = archive.scan();

        assert!(found.repairs.is_empty() && found.completions.is_empty());
        assert_eq!(
            (found.counts.file_missing, found.counts.needs_layout_rerun),
            (1, 1)
        );
    }

    #[test]
    fn a_stale_proposal_refuses_the_whole_batch() {
        let archive = Archive::new();
        reverted(&archive, "a1", "one");
        reverted(&archive, "a2", "two");
        let found = archive.scan();
        assert_eq!(found.repairs.len(), 2);

        // Between the scan and the confirmation, someone chose another version.
        archive
            .conn
            .execute(
                "UPDATE assets SET path = ?1 WHERE id = 'a2'",
                [format!("{DIR}/two.jpg")],
            )
            .expect("move");
        let assets = archive.dump("assets");

        let error = archive
            .apply(&confirm_all(&found), &[])
            .expect_err("stale proposal");

        assert!(error.starts_with(STALE_PROPOSAL_ERROR), "{error}");
        assert!(error.contains("a2") && !error.contains("a1"), "{error}");
        assert_eq!(
            archive.dump("assets"),
            assets,
            "nothing of the batch is written"
        );
        assert!(archive.events().is_empty());
    }

    #[test]
    fn a_row_that_changes_after_the_judging_rolls_the_batch_back() {
        let archive = Archive::new();
        reverted(&archive, "a1", "one");
        reverted(&archive, "a2", "two");
        let found = archive.scan();
        let confirmed = confirm_all(&found);
        let rows =
            layout_rows_of(&archive.conn, &planned_asset_ids(&confirmed, &[])).expect("rows");
        let planned = plan(&rows, archive.data.path(), &confirmed, &[]).expect("plan");

        // The second asset's layout is run again before the write.
        archive
            .conn
            .execute(
                "UPDATE layouts SET image_width = 41 WHERE asset_id = 'a2'",
                [],
            )
            .expect("rerun");
        let assets = archive.dump("assets");

        let error = write_plan(&archive.conn, &planned).expect_err("stale at write");

        assert!(error.starts_with(STALE_PROPOSAL_ERROR), "{error}");
        assert_eq!(
            archive.dump("assets"),
            assets,
            "the first repair is rolled back"
        );
        assert!(archive.events().is_empty());
    }

    #[test]
    fn only_the_confirmed_assets_are_touched() {
        let archive = Archive::new();
        reverted(&archive, "a1", "one");
        reverted(&archive, "a2", "two");
        let found = archive.scan();
        let only_second: Vec<_> = found
            .repairs
            .iter()
            .filter(|repair| repair.asset_id == "a2")
            .map(confirm)
            .collect();

        let applied = archive.apply(&only_second, &[]).expect("apply");

        assert_eq!(applied.repaired.len(), 1);
        assert_eq!(archive.path_of("a1"), format!("{DIR}/one_v4.jpg"));
        assert_eq!(archive.path_of("a2"), format!("{DIR}/two_v11.jpg"));
        assert_eq!(archive.events().len(), 1);

        // An asset the scan never proposed cannot be slipped in.
        let forged = ConfirmedProposal {
            asset_id: "a1".to_string(),
            token: "not-a-token".to_string(),
        };
        assert!(archive.apply(&[forged], &[]).is_err());
        assert_eq!(archive.path_of("a1"), format!("{DIR}/one_v4.jpg"));
    }

    #[test]
    fn pdfs_and_the_pages_of_a_pdf_are_never_scanned() {
        let archive = Archive::new();
        archive.file("doc.jpg", 96, 128, b"");
        archive.file("doc_v2.jpg", 40, 60, b"");
        std::fs::write(archive.dir().join("doc.pdf"), b"%PDF-1.7\n%%EOF\n").expect("pdf");
        archive.asset("pdf", "pdf", None, "doc.pdf");
        archive.layout("pdf", 40, 60, None, LATER);
        // A page of the PDF, stored as an image with a layout of another size.
        archive.asset("page", "image", Some("pdf"), "doc.jpg");
        archive.layout("page", 40, 60, None, LATER);

        let found = archive.scan();

        assert_eq!(found, RepairScan::default());
        let forged = ConfirmedProposal {
            asset_id: "page".to_string(),
            token: String::new(),
        };
        assert!(archive.apply(&[forged], &[]).is_err());
        assert_eq!(archive.path_of("page"), format!("{DIR}/doc.jpg"));
    }

    #[test]
    fn completing_the_source_of_an_old_layout_writes_exactly_its_two_columns() {
        let archive = Archive::new();
        let shown = archive.file("scan.jpg", 40, 60, b"");
        archive.image("a1", "scan.jpg");
        archive.layout("a1", 40, 60, None, LATER);
        // Two files of the layout's size: which one it came from is unknown.
        archive.file("twin.jpg", 40, 60, b"");
        archive.file("twin_v2.jpg", 40, 60, b"other");
        archive.image("a2", "twin_v2.jpg");
        archive.layout("a2", 40, 60, None, LATER);
        archive
            .conn
            .execute_batch(
                "INSERT INTO extractions VALUES ('e1', 'a1', 'texto', 'glm-ocr', 0.9, 5, NULL, NULL);",
            )
            .expect("extraction");

        let found = archive.scan();

        assert_eq!(found.counts.matching, 2);
        let [completion] = found.completions.as_slice() else {
            panic!("one completion expected: {:?}", found.completions);
        };
        assert_eq!(
            (completion.asset_id.as_str(), completion.file_name.as_str()),
            ("a1", "scan.jpg")
        );

        let files = archive.files();
        let untouched = ["assets", "extractions"].map(|table| archive.dump(table));
        let confirmed = ConfirmedProposal {
            asset_id: completion.asset_id.clone(),
            token: completion.token.clone(),
        };

        let applied = archive
            .apply(&[], std::slice::from_ref(&confirmed))
            .expect("apply");

        assert_eq!((applied.repaired.len(), applied.completed), (0, 1));
        type Layout = (
            String,
            String,
            String,
            i64,
            i64,
            i64,
            Option<String>,
            Option<String>,
        );
        let layout: Layout = archive
            .conn
            .query_row(
                "SELECT regions, blocks, model, image_width, image_height, created_at,
                        source_path, source_sha256 FROM layouts WHERE asset_id = 'a1'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .expect("layout");
        assert_eq!(
            layout,
            (
                "[{\"r\":1}]".to_string(),
                "[{\"b\":2}]".to_string(),
                "glm-ocr".to_string(),
                40,
                60,
                LATER,
                Some(format!("{DIR}/scan.jpg")),
                Some(hash_file(&shown).expect("hash").0),
            )
        );
        assert_eq!(
            ["assets", "extractions"].map(|table| archive.dump(table)),
            untouched,
            "the text is not stamped: nothing says which file it was read from"
        );
        assert_eq!(archive.files(), files);
        assert!(archive.events().is_empty(), "a completion is not a process");

        // Stamped now: not offered again, and the old confirmation is stale.
        assert!(archive.scan().completions.is_empty());
        assert!(archive.apply(&[], &[confirmed]).is_err());
    }

    #[test]
    fn progress_is_reported_in_steps_and_at_the_end() {
        let archive = Archive::new();
        for index in 0..(PROGRESS_STEP + 3) {
            let name = format!("p{index}.jpg");
            archive.file(&name, 8, 8, b"");
            archive.image(&format!("a{index:02}"), &name);
            archive.layout(&format!("a{index:02}"), 8, 8, None, LATER);
        }
        let rows = all_layout_rows(&archive.conn).expect("rows");
        let mut reports = Vec::new();

        scan(&rows, archive.data.path(), &mut |done, total| {
            reports.push((done, total))
        });

        let total = PROGRESS_STEP + 3;
        assert_eq!(reports, [(PROGRESS_STEP, total), (total, total)]);
    }
}
