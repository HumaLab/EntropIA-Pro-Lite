//! B3: explicit reprocess planning over what is already stored
//! (plan-texto-nativo-parte-b 2.3 "El plan, una sola función…" + 2.4).
//!
//! Nothing here spends money. [`plan_reprocess`] is the ONE pure planner the
//! read-only preview (this slice) and the B4 executor share: both feed it the
//! same input — the native pages the part-A reader builds from the current
//! bytes, the stored page rows, and whether the stored extraction matches the
//! source — and get the same `plan_hash` back. The preview shows the pages
//! and the estimated USD; every paid OCR is approved by the owner.
//!
//! 2.4 "Una sola vez": the candidate list excludes an attachment whose
//! current file already went through a successful reprocess at the current
//! detector version; the sync never reads that mark.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::bibliography::attachment::{
    attachment_ref_for, resolve_attachment_file, AttachmentRef, AttachmentResolution,
};
use crate::bibliography::processing::{
    is_pdf_attachment, ocr_candidate_pages, read_native_extraction_basis_with_cancel,
    ExtractPageText, NativeExtractionBasis, BIBLIOGRAPHY_EXTRACT_MAX_BYTES,
    ZOTERO_DATA_DIR_SETTING_KEY,
};
use crate::bibliography::repository::{extraction_matches_source, page_texts_for_attachment};
use crate::ocr::markup::ocr_markup_to_text;
use crate::ocr::pdf::{
    is_garbled_bibliography_text, is_garbled_text, BIBLIOGRAPHY_DETECTOR_VERSION,
};
use crate::processing::repository::{
    attachment_extraction_fingerprint, attachment_extraction_identity_prefix, derived_task_id,
    is_terminal_ocr_attempt, link_batch_task_subject, live_task, now_ms, parse_extract_contract,
    reprocess_contract_hash,
};

// ── Cost estimate (2.3 "Monto estimado") ───────────────────────────────────

/// GLM-OCR's rate: USD 0.03 per million tokens, billed on input AND output
/// alike (docs.z.ai pricing for GLM-OCR, observed 2026-10-08).
pub const GLM_OCR_USD_PER_MILLION_TOKENS: f64 = 0.03;

/// Tokens charged per OCR'd page. ASSUMPTION, not a measured figure: the
/// provider's token count is never read back, so the estimate assumes 3,000
/// tokens per page (input + output together) and the UI must label the
/// amount as estimated (plan 2.3 "Monto estimado", section 5 risks).
pub const ESTIMATED_TOKENS_PER_PAGE: u64 = 3000;

/// The estimated USD cost of sending `ocr_pages` pages to GLM-OCR:
/// pages × tokens-per-page × rate per million tokens.
pub fn estimated_usd(ocr_pages: u64) -> f64 {
    ocr_pages as f64 * ESTIMATED_TOKENS_PER_PAGE as f64 * GLM_OCR_USD_PER_MILLION_TOKENS
        / 1_000_000.0
}

// ── The plan (2.3) ─────────────────────────────────────────────────────────

/// The plan format version, pinned into `plan_hash` so a future change to the
/// selection rules can never silently reuse an old authorization.
pub const REPROCESS_PLAN_VERSION: u32 = 1;

/// One stored `bibliographic_page_texts` row, reduced to what the planner
/// decides on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPageInput {
    pub page_number: i64,
    /// `native` or `ocr` — only stored `ocr` rows are ever reused.
    pub method: String,
    pub text_hash: String,
    pub text_content: String,
}

/// [`plan_reprocess`]'s whole input. Exactly what the B4 executor feeds it
/// after re-reading the file, and what the preview feeds it now.
#[derive(Debug, Clone)]
pub struct ReprocessPlanInput {
    pub attachment_id: String,
    /// SHA-256 (hex) of the FILE bytes — a PDF replaced by another of the
    /// same size still changes the plan hash (JD7-A-003).
    pub source_sha256: String,
    /// The native page rows exactly as the bibliography extractor builds
    /// them before OCR (part-A reader output), one per document page.
    pub native_pages: Vec<ExtractPageText>,
    /// The extractor's pre-part-A `native_blank` basis on those bytes.
    pub native_blank: bool,
    /// The stored `bibliographic_page_texts` rows of the attachment.
    pub stored_pages: Vec<StoredPageInput>,
    /// `extraction_matches_source`: does the stored extraction reflect this
    /// exact file identity? Only then may stored OCR rows be reused.
    pub extraction_matches_source: bool,
}

/// A stored OCR page the plan reuses instead of re-sending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReusedOcrPage {
    pub page: i64,
    pub text_hash: String,
}

/// What one reprocess would do to one attachment, plus the hash that ties
/// the owner's approval to exactly this plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessPlan {
    pub page_count: i64,
    /// Pages to send to GLM-OCR, ascending. The executor sends exactly
    /// these and nothing else (2.3 "El ejecutor no vuelve a elegir páginas").
    pub ocr_pages: Vec<i64>,
    /// Stored `method = 'ocr'` rows reused as-is (empty ones included),
    /// ascending by page.
    pub reused_ocr_pages: Vec<ReusedOcrPage>,
    /// Pages whose STORED text the detectors flag and that neither go to
    /// OCR nor are reused: re-extraction alone (PDFium's part-A read) is
    /// expected to repair them. Ascending.
    pub fixed_without_ocr: Vec<i64>,
    /// SHA-256 (hex) of the canonical plan payload. The contract string is
    /// NOT part of it (JD7-B "hash circular"): the contract embeds the hash,
    /// never the reverse.
    pub plan_hash: String,
}

/// The pure planner: preview and executor compute the SAME plan from the
/// SAME input. Selection (2.3):
///
/// 1. When the stored extraction matches the source, every stored
///    `method = 'ocr'` row is reused — never sent again, even when empty.
/// 2. Of the rest, [`ocr_candidate_pages`] picks the pages that go to OCR
///    (sparse/empty/unreadable-per-`native_blank` plus the B2 detector).
/// 3. `fixed_without_ocr`: pages whose STORED text is flagged by
///    [`is_garbled_bibliography_text`] or [`is_garbled_text`] after markup
///    conversion, that are neither sent nor reused.
pub fn plan_reprocess(input: &ReprocessPlanInput) -> ReprocessPlan {
    // 1. Stored OCR rows of the matching source are settled: reused as-is,
    //    empty answers included — never sent to a provider again.
    let mut reused: BTreeMap<i64, String> = BTreeMap::new();
    if input.extraction_matches_source {
        for row in &input.stored_pages {
            if row.method == "ocr" {
                reused.insert(row.page_number, row.text_hash.clone());
            }
        }
    }
    // 2. Of the rest, the executor's own candidacy rule picks the pages that
    //    go to OCR (it already carries the B2 detector).
    let ocr_pages: BTreeSet<i64> = ocr_candidate_pages(&input.native_pages, input.native_blank)
        .into_iter()
        .filter(|page_number| !reused.contains_key(page_number))
        .collect();
    // 3. Pages whose STORED text both detectors flag and the plan neither
    //    sends nor reuses: the fresh native read alone is expected to repair
    //    them without a provider.
    let mut fixed_without_ocr: Vec<i64> = input
        .stored_pages
        .iter()
        .filter(|row| {
            !reused.contains_key(&row.page_number) && !ocr_pages.contains(&row.page_number)
        })
        .filter(|row| {
            let converted = ocr_markup_to_text(&row.text_content);
            is_garbled_bibliography_text(&converted) || is_garbled_text(&converted)
        })
        .map(|row| row.page_number)
        .collect();
    fixed_without_ocr.sort_unstable();
    fixed_without_ocr.dedup();
    let page_count = input.native_pages.len() as i64;
    let ocr_pages: Vec<i64> = ocr_pages.into_iter().collect();
    let reused_ocr_pages: Vec<ReusedOcrPage> = reused
        .into_iter()
        .map(|(page, text_hash)| ReusedOcrPage { page, text_hash })
        .collect();
    let plan_hash = plan_hash(
        &input.attachment_id,
        &input.source_sha256,
        page_count,
        &ocr_pages,
        &reused_ocr_pages,
        BIBLIOGRAPHY_DETECTOR_VERSION,
    );
    ReprocessPlan {
        page_count,
        ocr_pages,
        reused_ocr_pages,
        fixed_without_ocr,
        plan_hash,
    }
}

/// The canonical JSON payload behind `plan_hash`: exactly
/// `{attachmentId, sourceSha256, pageCount, ocrPages, reusedOcr [{page,
/// textHash}], detectorVersion, planVersion}` with sorted keys and sorted
/// page entries — serde_json is built without `preserve_order` and the maps
/// are assembled through `BTreeMap`, so the bytes are deterministic. The
/// contract is deliberately absent (see [`ReprocessPlan::plan_hash`]).
fn plan_hash_payload(
    attachment_id: &str,
    source_sha256: &str,
    page_count: i64,
    ocr_pages: &[i64],
    reused_ocr_pages: &[ReusedOcrPage],
    detector_version: u32,
) -> Vec<u8> {
    let reused: Vec<serde_json::Value> = reused_ocr_pages
        .iter()
        .map(|entry| {
            let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
            map.insert("page".to_string(), serde_json::json!(entry.page));
            map.insert("textHash".to_string(), serde_json::json!(entry.text_hash));
            serde_json::Value::Object(map.into_iter().collect())
        })
        .collect();
    let mut payload: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    payload.insert("attachmentId".to_string(), serde_json::json!(attachment_id));
    payload.insert("sourceSha256".to_string(), serde_json::json!(source_sha256));
    payload.insert("pageCount".to_string(), serde_json::json!(page_count));
    payload.insert("ocrPages".to_string(), serde_json::json!(ocr_pages));
    payload.insert("reusedOcr".to_string(), serde_json::Value::Array(reused));
    payload.insert(
        "detectorVersion".to_string(),
        serde_json::json!(detector_version),
    );
    payload.insert(
        "planVersion".to_string(),
        serde_json::json!(REPROCESS_PLAN_VERSION),
    );
    serde_json::to_vec(&serde_json::Value::Object(payload.into_iter().collect()))
        .expect("canonical plan JSON serializes")
}

/// SHA-256 (hex) of the canonical plan payload. `detector_version` is a
/// parameter so the version's participation in the hash is testable; the
/// planner passes [`BIBLIOGRAPHY_DETECTOR_VERSION`].
fn plan_hash(
    attachment_id: &str,
    source_sha256: &str,
    page_count: i64,
    ocr_pages: &[i64],
    reused_ocr_pages: &[ReusedOcrPage],
    detector_version: u32,
) -> String {
    format!(
        "{:x}",
        Sha256::digest(plan_hash_payload(
            attachment_id,
            source_sha256,
            page_count,
            ocr_pages,
            reused_ocr_pages,
            detector_version,
        ))
    )
}

/// A planning failure. `Cancelled` is the preview's own stop flag, not
/// damage: the attachment was abandoned mid-read and reports no plan.
#[derive(Debug)]
pub enum PlanError {
    Cancelled,
    Failed(String),
}

/// Everything one planning read produced (2.3 "El plan, una sola función
/// para la vista previa y el ejecutor"): the pre-OCR basis the B4 executor
/// publishes from, the plan itself, and the stored rows the plan decided
/// over. Preview and executor go through THIS function, so the pages the
/// hash covers are exactly the pages the executor publishes.
pub struct ReprocessPlanning {
    /// The native page rows of the part-A reader on the current bytes.
    pub basis: NativeExtractionBasis,
    pub plan: ReprocessPlan,
    /// The stored `bibliographic_page_texts` rows the plan decided over —
    /// the text the executor reuses for `reused_ocr_pages`.
    pub stored_pages: Vec<StoredPageInput>,
    /// SHA-256 (hex) of the file bytes as read (the plan's `sourceSha256`).
    pub source_sha256: String,
}

/// Computes the plan of one attachment from its current file bytes and the
/// stored rows — the entry point the B3 preview uses and the B4 executor
/// reuses on the bytes it read. `path` names the file for error
/// messages only; the bytes are the source of truth.
pub fn plan_reprocess_for_attachment(
    conn: &Connection,
    attachment: &AttachmentRef,
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<ReprocessPlan, String> {
    match plan_reprocess_for_attachment_cancellable(conn, attachment, path, bytes, None) {
        Ok(planning) => Ok(planning.plan),
        Err(PlanError::Failed(message)) => Err(message),
        Err(PlanError::Cancelled) => Err("reprocess planning cancelled mid-file".to_string()),
    }
}

/// [`plan_reprocess_for_attachment`] with the preview's cancellation flag:
/// checked between page batches inside the shared basis reader
/// ([`crate::bibliography::processing::read_native_extraction_basis_with_cancel`]).
/// The reader is called with NO app handle on purpose: the preview and the
/// executor must resolve the SAME decoder or their plan hashes would drift
/// (the resolver caches process-wide, so both sides see one answer).
pub fn plan_reprocess_for_attachment_cancellable(
    conn: &Connection,
    attachment: &AttachmentRef,
    path: &std::path::Path,
    bytes: &[u8],
    cancel: Option<&AtomicBool>,
) -> Result<ReprocessPlanning, PlanError> {
    let source_sha256 = format!("{:x}", Sha256::digest(bytes));
    let basis = read_native_extraction_basis_with_cancel(bytes, None, cancel).map_err(
        |output| match output {
            crate::processing::scheduler::ExecOutput::Stopped => PlanError::Cancelled,
            other => PlanError::Failed(format!(
                "{}: {}",
                path.display(),
                exec_output_message(&other)
            )),
        },
    )?;
    let stored_pages = page_texts_for_attachment(conn, &attachment.attachment_id)
        .map_err(|error| PlanError::Failed(format!("{}: {}", error.code, error.message)))?
        .into_iter()
        .map(|row| StoredPageInput {
            page_number: row.page_number,
            method: row.method,
            text_hash: row.text_hash,
            text_content: row.text_content,
        })
        .collect::<Vec<_>>();
    let matches_source = extraction_matches_source(
        conn,
        &attachment.attachment_id,
        attachment.mtime,
        bytes.len() as i64,
    )
    .map_err(|error| PlanError::Failed(format!("{}: {}", error.code, error.message)))?;
    let plan = plan_reprocess(&ReprocessPlanInput {
        attachment_id: attachment.attachment_id.clone(),
        source_sha256: source_sha256.clone(),
        native_pages: basis.pages.clone(),
        native_blank: basis.native_blank,
        stored_pages: stored_pages.clone(),
        extraction_matches_source: matches_source,
    });
    Ok(ReprocessPlanning {
        basis,
        plan,
        stored_pages,
        source_sha256,
    })
}

fn exec_output_message(output: &crate::processing::scheduler::ExecOutput) -> String {
    use crate::processing::scheduler::ExecOutput;
    match output {
        ExecOutput::Success { outcome, .. } => outcome.clone(),
        ExecOutput::Retryable { code, message }
        | ExecOutput::Fatal { code, message }
        | ExecOutput::Blocked { code, message } => format!("{code}: {message}"),
        ExecOutput::Stopped => "stopped".to_string(),
    }
}

// ── Candidate list (2.3 "Comandos nuevos" 1, 2.4) ──────────────────────────

/// Reason codes (stable machine strings; the UI renders them).
/// (a) At least one stored page flagged by the detectors after conversion.
pub const REASON_GARBLED_STORED_PAGES: &str = "garbled_stored_pages";
/// (b) An `empty` extraction or `empty` page rows and no succeeded OCR pass
/// ever answered for the current file.
pub const REASON_EMPTY_WITHOUT_OCR: &str = "empty_without_ocr";
/// (c) A terminal failed/cancelled OCR attempt already spent on this file
/// (the B1 rule): the owner recovers it here.
pub const REASON_FAILED_OCR_ATTEMPT: &str = "failed_ocr_attempt";

/// One attachment the reprocess action should consider.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessCandidate {
    pub attachment_id: String,
    pub item_id: String,
    pub title: String,
    pub filename: Option<String>,
    /// Machine reason codes in fixed order, at least one.
    pub reasons: Vec<String>,
    /// How many stored page rows the detectors flag (the size of rule (a)).
    pub flagged_pages: i64,
}

/// The read-only, cheap candidate scan (2.3 "Comandos nuevos" 1): PDF
/// attachments whose stored text needs the owner's repair action, ordered by
/// work title. No file contents are read; the file length behind the
/// "already reprocessed" exclusion is the stored extraction identity (see
/// [`current_file_identity`]).
pub fn reprocess_candidates(conn: &Connection) -> Result<Vec<ReprocessCandidate>, String> {
    // ONE pass over the extraction tasks: the real archive carries tens of
    // thousands of task rows and no terminal-state index, so per-attachment
    // lookups would full-scan the table once per attachment (measured on the
    // prueba-sync copy: 28 s for ~200 PDF attachments).
    let tasks = ExtractTaskIndex::load(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.item_id, a.content_type, a.filename, a.mtime, COALESCE(i.title, ''),
                    e.quality
             FROM zotero_attachments a
             JOIN bibliographic_items i ON i.id = a.item_id
             LEFT JOIN bibliographic_extractions e ON e.attachment_id = a.id
             ORDER BY COALESCE(i.title, ''), a.id",
        )
        .map_err(|error| format!("Failed to list PDF attachments: {error}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|error| format!("Failed to list PDF attachments: {error}"))?;
    let mut out = Vec::new();
    for row in rows {
        let (attachment_id, item_id, content_type, filename, mtime, title, extraction_quality) =
            row.map_err(|error| format!("Failed to list PDF attachments: {error}"))?;
        if !is_pdf_attachment(content_type.as_deref(), filename.as_deref()) {
            continue;
        }
        // (a): scan the STORED page texts — no file is read — for text the
        // detectors flag after markup conversion, and note `empty` rows for
        // (b). The scan streams one attachment at a time.
        let mut flagged_pages: i64 = 0;
        let mut any_empty_page = false;
        {
            let mut pages = conn
                .prepare(
                    "SELECT quality, text_content FROM bibliographic_page_texts
                     WHERE attachment_id = ?1 ORDER BY page_number",
                )
                .map_err(|error| format!("Failed to read stored pages: {error}"))?;
            let pages = pages
                .query_map([&attachment_id], |page| {
                    Ok((page.get::<_, String>(0)?, page.get::<_, String>(1)?))
                })
                .map_err(|error| format!("Failed to read stored pages: {error}"))?;
            for page in pages {
                let (quality, text_content) =
                    page.map_err(|error| format!("Failed to read stored pages: {error}"))?;
                if quality == "empty" {
                    any_empty_page = true;
                }
                let converted = ocr_markup_to_text(&text_content);
                if is_garbled_bibliography_text(&converted) || is_garbled_text(&converted) {
                    flagged_pages += 1;
                }
            }
        }
        let mut reasons: Vec<&str> = Vec::new();
        if flagged_pages > 0 {
            reasons.push(REASON_GARBLED_STORED_PAGES);
        }
        if (extraction_quality.as_deref() == Some("empty") || any_empty_page)
            && !tasks.succeeded_ocr_attempted(&attachment_id, mtime)
        {
            reasons.push(REASON_EMPTY_WITHOUT_OCR);
        }
        if tasks.terminal_ocr_attempt(&attachment_id, mtime) {
            reasons.push(REASON_FAILED_OCR_ATTEMPT);
        }
        if reasons.is_empty() {
            continue;
        }
        // 2.4: a successful reprocess of this exact file at the current
        // detector version has already repaired it — once is enough.
        let identity = current_file_identity(conn, &attachment_id, mtime)?;
        if tasks.reprocessed_current_file(&attachment_id, identity) {
            continue;
        }
        out.push(ReprocessCandidate {
            attachment_id,
            item_id,
            title,
            filename,
            reasons: reasons.into_iter().map(String::from).collect(),
            flagged_pages,
        });
    }
    Ok(out)
}

/// One `bibliography_extract` task row as the candidate scan reads it.
struct ExtractTaskRow {
    state: String,
    last_error_code: Option<String>,
    input_fingerprint: String,
    receipt: Option<String>,
}

/// Every `bibliography_extract` task row of the catalog, grouped by
/// attachment, loaded once per candidate scan.
struct ExtractTaskIndex {
    rows: BTreeMap<String, Vec<ExtractTaskRow>>,
}

impl ExtractTaskIndex {
    fn load(conn: &Connection) -> Result<Self, String> {
        let mut stmt = conn
            .prepare(
                "SELECT subject_id, state, last_error_code, input_fingerprint, result_receipt_json
                 FROM processing_tasks
                 WHERE kind = 'bibliography_extract'
                   AND domain = 'bibliography' AND subject_kind = 'attachment'
                   AND state IN ('succeeded', 'failed', 'cancelled')",
            )
            .map_err(|error| format!("Failed to list extraction tasks: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    ExtractTaskRow {
                        state: row.get(1)?,
                        last_error_code: row.get(2)?,
                        input_fingerprint: row.get(3)?,
                        receipt: row.get(4)?,
                    },
                ))
            })
            .map_err(|error| format!("Failed to list extraction tasks: {error}"))?;
        let mut index: BTreeMap<String, Vec<ExtractTaskRow>> = BTreeMap::new();
        for row in rows {
            let (attachment_id, task) =
                row.map_err(|error| format!("Failed to list extraction tasks: {error}"))?;
            index.entry(attachment_id).or_default().push(task);
        }
        Ok(ExtractTaskIndex { rows: index })
    }

    fn for_attachment(&self, attachment_id: &str) -> &[ExtractTaskRow] {
        self.rows
            .get(attachment_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The B1 rule (2.1), ONE predicate over the row:
    /// [`is_terminal_ocr_attempt`] in `processing::repository` — a terminal
    /// `bibliography_extract` task whose `input_fingerprint` carries the file
    /// identity prefix and whose verdict was a spent OCR attempt. The index
    /// keeps the one-pass scan; the rule itself lives in one place.
    fn terminal_ocr_attempt(&self, attachment_id: &str, mtime: Option<i64>) -> bool {
        let prefix = attachment_extraction_identity_prefix(attachment_id, mtime);
        self.for_attachment(attachment_id).iter().any(|task| {
            task.input_fingerprint.starts_with(&prefix)
                && is_terminal_ocr_attempt(&task.state, task.last_error_code.as_deref())
        })
    }

    /// Rule (b)'s escape: a succeeded `bibliography_extract` whose receipt
    /// says `"ocrAttempted": true` and whose task fingerprint pins the
    /// current file (`mtime` prefix). Then the `empty` verdict is what the
    /// OCR really saw — a blank scan — and nothing is wrong.
    fn succeeded_ocr_attempted(&self, attachment_id: &str, mtime: Option<i64>) -> bool {
        let prefix = attachment_extraction_identity_prefix(attachment_id, mtime);
        self.for_attachment(attachment_id).iter().any(|task| {
            task.state == "succeeded"
                && task.input_fingerprint.starts_with(&prefix)
                && task
                    .receipt()
                    .and_then(|value| value.get("ocrAttempted").and_then(|flag| flag.as_bool()))
                    == Some(true)
        })
    }

    /// 2.4 "Una sola vez": true when a succeeded task already carries a
    /// reprocess receipt for THIS file identity at the CURRENT detector
    /// version. The receipt is written by the B4 confirm/executor path;
    /// compared fields are `reprocess.detectorVersion` plus
    /// `sourceMtime`/`sourceBytes`.
    fn reprocessed_current_file(
        &self,
        attachment_id: &str,
        identity: Option<(Option<i64>, i64)>,
    ) -> bool {
        let Some((mtime, source_bytes)) = identity else {
            return false;
        };
        self.for_attachment(attachment_id).iter().any(|task| {
            if task.state != "succeeded" {
                return false;
            }
            let Some(value) = task.receipt() else {
                return false;
            };
            let Some(reprocess) = value.get("reprocess") else {
                return false;
            };
            if reprocess.get("detectorVersion").and_then(|v| v.as_u64())
                != Some(u64::from(BIBLIOGRAPHY_DETECTOR_VERSION))
            {
                return false;
            }
            let source_mtime = match value.get("sourceMtime") {
                None | Some(serde_json::Value::Null) => None,
                Some(other) => other.as_i64(),
            };
            source_mtime == mtime
                && value.get("sourceBytes").and_then(|v| v.as_i64()) == Some(source_bytes)
        })
    }
}

impl ExtractTaskRow {
    fn receipt(&self) -> Option<serde_json::Value> {
        self.receipt
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
    }
}

/// The current file identity as the candidate list can know it without
/// reading the file (2.4: "Se compara con sourceMtime y sourceBytes, sin
/// leer el archivo en la lista; el SHA lo verifica la vista previa"): the
/// catalog `mtime` plus the byte length the stored extraction recorded for
/// it — trustworthy exactly while that extraction still matches the source.
/// `None` when no extraction pins the current file: then nothing may be
/// excluded, and the preview verifies with the real SHA-256.
fn current_file_identity(
    conn: &Connection,
    attachment_id: &str,
    mtime: Option<i64>,
) -> Result<Option<(Option<i64>, i64)>, String> {
    let stored: Option<(Option<i64>, i64)> = conn
        .query_row(
            "SELECT source_mtime, source_bytes FROM bibliographic_extractions
             WHERE attachment_id = ?1",
            [attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Failed to read extraction identity: {error}"))?;
    Ok(stored.and_then(|(source_mtime, source_bytes)| {
        (source_mtime == mtime).then_some((mtime, source_bytes))
    }))
}

/// Whether a live `bibliography_extract` task exists for the attachment —
/// the preview's `busy` flag and the confirm's own admission gate. ONE
/// lookup: `processing::repository::live_task` on the full subject identity.
fn live_extract_task_exists(conn: &Connection, attachment_id: &str) -> Result<bool, String> {
    Ok(live_task(
        conn,
        "bibliography",
        "attachment",
        attachment_id,
        "bibliography_extract",
    )?
    .is_some())
}

// ── Preview (2.3 "Comandos nuevos" 2) ──────────────────────────────────────

/// Unreadable reasons (stable machine strings; the UI renders them).
/// The attachment row vanished between listing and preview.
pub const UNREADABLE_ATTACHMENT_MISSING: &str = "attachment_missing";
/// No readable local file (missing moved/linked file, missing stored copy…).
pub const UNREADABLE_FILE_MISSING: &str = "file_missing";
/// The file is over [`BIBLIOGRAPHY_EXTRACT_MAX_BYTES`]: a storage problem,
/// never a text job (same gate as the executor).
pub const UNREADABLE_FILE_TOO_LARGE: &str = "file_too_large";
/// The attachment is not a PDF; reprocess covers PDF attachments.
pub const UNREADABLE_NOT_A_PDF: &str = "not_a_pdf";
/// The file exists but cannot be read or planned (I/O error, corrupt or
/// genuinely locked PDF).
pub const UNREADABLE_READ_FAILED: &str = "read_failed";

/// One previewed attachment. Unreadable ones carry `unreadable` and no
/// `planHash`; `busy` attachments have a plan but a live task, so the
/// confirm path will answer `busy` for them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessPreviewAttachment {
    pub attachment_id: String,
    pub item_id: String,
    pub title: String,
    pub filename: Option<String>,
    pub page_count: i64,
    pub ocr_pages: i64,
    pub reused_ocr_pages: i64,
    pub fixed_without_ocr: i64,
    pub plan_hash: Option<String>,
    pub busy: bool,
    pub unreadable: Option<String>,
}

/// The preview totals the dialog shows, including the estimated USD
/// (labeled "estimated" by the UI; see [`estimated_usd`]).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessPreviewTotals {
    pub attachments: i64,
    pub pages: i64,
    pub ocr_pages: i64,
    pub reused_ocr_pages: i64,
    pub fixed_without_ocr: i64,
    pub estimated_usd: f64,
}

/// The whole preview answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessPreview {
    pub attachments: Vec<ReprocessPreviewAttachment>,
    pub totals: ReprocessPreviewTotals,
    /// True when the cancel flag stopped the run early: `attachments` then
    /// holds only what was processed before the stop.
    pub cancelled: bool,
}

/// Per-attachment preview step: one entry, or the whole run stopping.
enum PreviewStep {
    Entry(ReprocessPreviewAttachment),
    Cancelled,
}

/// The preview's cancellation flag, process-wide: `bibliography_reprocess_preview_cancel`
/// sets it, every new preview call resets it (`reset_reprocess_preview_cancel`).
static PREVIEW_CANCEL: AtomicBool = AtomicBool::new(false);

/// The flag the preview command checks and the cancel command sets.
pub fn preview_cancel_flag() -> &'static AtomicBool {
    &PREVIEW_CANCEL
}

/// A new preview call resets the flag (2.3: "se cancela con
/// `bibliography_reprocess_preview_cancel` … una nueva vista previa lo
/// reinicia").
pub fn reset_reprocess_preview_cancel() {
    PREVIEW_CANCEL.store(false, Ordering::SeqCst);
}

/// Stops the running preview between attachments or page batches.
pub fn cancel_reprocess_preview() {
    PREVIEW_CANCEL.store(true, Ordering::SeqCst);
}

/// Runs the read-only preview over `attachment_ids`, in order: per
/// attachment it reads the file (size-gated like the executor), hashes the
/// bytes and plans through [`plan_reprocess`]. `on_progress` fires after
/// each finished attachment; `cancel` is checked between attachments and,
/// inside the shared reader, between page batches.
pub fn run_reprocess_preview(
    conn: &Connection,
    attachment_ids: &[String],
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(i64, i64),
) -> Result<ReprocessPreview, String> {
    let total = attachment_ids.len() as i64;
    let mut attachments: Vec<ReprocessPreviewAttachment> = Vec::new();
    let mut cancelled = false;
    for attachment_id in attachment_ids {
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        match preview_attachment(conn, attachment_id, cancel)? {
            PreviewStep::Entry(entry) => {
                attachments.push(entry);
                on_progress(attachments.len() as i64, total);
            }
            PreviewStep::Cancelled => {
                cancelled = true;
                break;
            }
        }
    }
    let mut totals = ReprocessPreviewTotals {
        attachments: attachments.len() as i64,
        pages: 0,
        ocr_pages: 0,
        reused_ocr_pages: 0,
        fixed_without_ocr: 0,
        estimated_usd: 0.0,
    };
    for entry in &attachments {
        totals.pages += entry.page_count;
        totals.ocr_pages += entry.ocr_pages;
        totals.reused_ocr_pages += entry.reused_ocr_pages;
        totals.fixed_without_ocr += entry.fixed_without_ocr;
    }
    // The estimate follows the OCR page count of the whole preview (busy
    // entries included: their plan is real even when the confirm path will
    // refuse to queue them).
    totals.estimated_usd = estimated_usd(totals.ocr_pages.max(0) as u64);
    Ok(ReprocessPreview {
        attachments,
        totals,
        cancelled,
    })
}

/// One attachment's preview entry. `Cancelled` stops the caller's loop.
fn preview_attachment(
    conn: &Connection,
    attachment_id: &str,
    cancel: &AtomicBool,
) -> Result<PreviewStep, String> {
    let Some(attachment) = attachment_ref_for(conn, attachment_id)? else {
        return Ok(PreviewStep::Entry(ReprocessPreviewAttachment {
            attachment_id: attachment_id.to_string(),
            item_id: String::new(),
            title: String::new(),
            filename: None,
            page_count: 0,
            ocr_pages: 0,
            reused_ocr_pages: 0,
            fixed_without_ocr: 0,
            plan_hash: None,
            busy: false,
            unreadable: Some(UNREADABLE_ATTACHMENT_MISSING.to_string()),
        }));
    };
    let (item_id, title): (String, String) = conn
        .query_row(
            "SELECT COALESCE(a.item_id, ''), COALESCE(i.title, '')
             FROM zotero_attachments a
             LEFT JOIN bibliographic_items i ON i.id = a.item_id
             WHERE a.id = ?1",
            [attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("Failed to read attachment work: {error}"))?;
    let busy = live_extract_task_exists(conn, attachment_id)?;
    let filename = attachment.filename.clone();
    let entry =
        |unreadable: Option<&str>, plan: Option<&ReprocessPlan>| ReprocessPreviewAttachment {
            attachment_id: attachment_id.to_string(),
            item_id: item_id.clone(),
            title: title.clone(),
            filename: filename.clone(),
            page_count: plan.map(|plan| plan.page_count).unwrap_or(0),
            ocr_pages: plan.map(|plan| plan.ocr_pages.len() as i64).unwrap_or(0),
            reused_ocr_pages: plan
                .map(|plan| plan.reused_ocr_pages.len() as i64)
                .unwrap_or(0),
            fixed_without_ocr: plan
                .map(|plan| plan.fixed_without_ocr.len() as i64)
                .unwrap_or(0),
            plan_hash: plan.map(|plan| plan.plan_hash.clone()),
            busy,
            unreadable: unreadable.map(String::from),
        };
    let data_dir = crate::settings::get_setting(conn, ZOTERO_DATA_DIR_SETTING_KEY);
    let path = match resolve_attachment_file(&attachment, data_dir.as_deref()) {
        AttachmentResolution::File(path) => path,
        AttachmentResolution::Unavailable { .. } => {
            return Ok(PreviewStep::Entry(entry(
                Some(UNREADABLE_FILE_MISSING),
                None,
            )));
        }
    };
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return Ok(PreviewStep::Entry(entry(
                Some(UNREADABLE_FILE_MISSING),
                None,
            )));
        }
    };
    if metadata.len() > BIBLIOGRAPHY_EXTRACT_MAX_BYTES {
        return Ok(PreviewStep::Entry(entry(
            Some(UNREADABLE_FILE_TOO_LARGE),
            None,
        )));
    }
    if !is_pdf_attachment(
        attachment.content_type.as_deref(),
        attachment.filename.as_deref(),
    ) {
        return Ok(PreviewStep::Entry(entry(Some(UNREADABLE_NOT_A_PDF), None)));
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(PreviewStep::Entry(entry(
                Some(UNREADABLE_READ_FAILED),
                None,
            )));
        }
    };
    match plan_reprocess_for_attachment_cancellable(conn, &attachment, &path, &bytes, Some(cancel))
    {
        Ok(planning) => Ok(PreviewStep::Entry(entry(None, Some(&planning.plan)))),
        Err(PlanError::Cancelled) => Ok(PreviewStep::Cancelled),
        Err(PlanError::Failed(_)) => Ok(PreviewStep::Entry(entry(
            Some(UNREADABLE_READ_FAILED),
            None,
        ))),
    }
}

// ── Confirm (2.3 "Comandos nuevos" 3: admisión propia) ─────────────────

/// One owner-approved entry of the confirm call: the attachment and the
/// `plan_hash` the preview showed (B5 sends exactly `{attachmentId,
/// planHash}`).
#[derive(Debug, Clone)]
pub struct ReprocessConfirmEntry {
    pub attachment_id: String,
    pub plan_hash: String,
}

/// What happened to one confirm entry. Stable machine strings; the UI
/// renders them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReprocessEntryStatus {
    /// The entry owns a fresh reprocess task in the new batch.
    Queued,
    /// A live `bibliography_extract` task already owns the attachment: the
    /// confirm NEVER attaches to it and never charges outside its plan.
    Busy,
    /// No PDF attachment row for the id (vanished, or never a PDF).
    UnknownAttachment,
}

/// One per-entry answer of the confirm.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessConfirmResult {
    pub attachment_id: String,
    pub status: ReprocessEntryStatus,
}

/// The confirm answer: the new user batch when anything was queued, and one
/// status per entry in request order.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessConfirm {
    pub batch_id: Option<String>,
    pub results: Vec<ReprocessConfirmResult>,
}

/// What one plain task INSERT did (2.3 "Si no la hay, hace un INSERT sin
/// `OR IGNORE`. Un choque con el índice único también da `busy`"). The
/// seam the race test drives directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted(String),
    Busy,
}

/// The confirm's own INSERT: a plain `INSERT` (never `OR IGNORE`, never
/// `admit_subject_or_attach`) of one `bibliography_extract` task pinning the
/// unchanged [`attachment_extraction_fingerprint`] and the reprocess
/// contract `prefix + <plan hash>`. The partial unique
/// `idx_processing_tasks_subject_active_unique` is the single-flight
/// authority: a live row inserted between the caller's check and this INSERT
/// collides and answers [`InsertOutcome::Busy`], and the caller must never
/// link such a task to the batch.
pub fn insert_reprocess_extract_task(
    conn: &Connection,
    attachment: &AttachmentRef,
    plan_hash: &str,
) -> Result<InsertOutcome, String> {
    let contract_hash = reprocess_contract_hash(plan_hash);
    if parse_extract_contract(&contract_hash).is_none() {
        return Err(format!(
            "invalid_selection: {plan_hash:?} is not an approved reprocess plan hash"
        ));
    }
    let task_id = derived_task_id(now_ms());
    let now = now_ms();
    let inserted = conn.execute(
        "INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
            input_revision, input_fingerprint, contract_hash, state, created_at, updated_at)
         VALUES (?1, 'bibliography_extract', ?2, 'bibliography', 'attachment', ?2,
            0, ?3, ?4, 'pending', ?5, ?5)",
        rusqlite::params![
            task_id,
            attachment.attachment_id,
            attachment_extraction_fingerprint(attachment),
            contract_hash,
            now
        ],
    );
    match inserted {
        Ok(_) => Ok(InsertOutcome::Inserted(task_id)),
        Err(error) if is_unique_constraint(&error) => {
            // A live task landed between the check and this INSERT (JD7-B-001):
            // one `busy` answer, never an attach and never a second writer.
            Ok(InsertOutcome::Busy)
        }
        Err(error) => Err(format!(
            "Failed to queue the reprocess of {}: {error}",
            attachment.attachment_id
        )),
    }
}

/// A UNIQUE violation of `processing_tasks`: the partial unique index is the
/// race authority the plain INSERT collides against.
fn is_unique_constraint(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation
    ) && error.to_string().contains("UNIQUE constraint")
}

/// `bibliography_reprocess_confirm`'s transactional core (2.3): ONE
/// `BEGIN IMMEDIATE` transaction creates ONE user batch (visible and
/// cancelable like any user batch) and admits each entry on its own — a live
/// task is `busy`, a UNIQUE collision is `busy`, everything else is a plain
/// INSERT linked to the batch. The confirm NEVER attaches to an existing
/// task and NEVER reuses [`crate::processing::repository::admit_subject_or_attach`].
/// A batch with nothing queued is deleted inside the same transaction and
/// answers `batchId: null`.
pub fn confirm_reprocess(
    conn: &Connection,
    entries: &[ReprocessConfirmEntry],
) -> Result<ReprocessConfirm, String> {
    if entries.is_empty() {
        return Err(
            "invalid_selection: confirm at least one attachment of the preview".to_string(),
        );
    }
    // The whole call fails before the queue is touched when any approval is
    // not the shape the preview produces.
    for entry in entries {
        if parse_extract_contract(&reprocess_contract_hash(&entry.plan_hash)).is_none() {
            return Err(format!(
                "invalid_selection: plan hash {:?} of attachment {} is not 64 lowercase hex characters",
                entry.plan_hash, entry.attachment_id
            ));
        }
    }
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| format!("Failed to begin the reprocess confirm: {error}"))?;
    let applied = (|| -> Result<ReprocessConfirm, String> {
        let now = now_ms();
        let batch_id = format!("batch-{}", uuid::Uuid::new_v4());
        let request_id = format!("bibliography-reprocess-{}", uuid::Uuid::new_v4());
        // The operations list is what the batch tab renders: `"ocr"` shows
        // the batch in its OCR column, exactly what these tasks spend.
        conn.execute(
            "INSERT INTO processing_batches
               (id, request_id, origin, state, desired_state, operations, planning_done, priority, created_at, updated_at)
             VALUES (?1, ?2, 'user', 'running', 'run', '[\"ocr\"]', 1, 2, ?3, ?3)",
            rusqlite::params![batch_id, request_id, now],
        )
        .map_err(|error| format!("Failed to create the reprocess batch: {error}"))?;
        let mut results = Vec::with_capacity(entries.len());
        let mut queued = 0usize;
        for entry in entries {
            let Some(attachment) = attachment_ref_for(conn, &entry.attachment_id)
                .map_err(|error| format!("Failed to check bibliography attachment: {error}"))?
            else {
                results.push(ReprocessConfirmResult {
                    attachment_id: entry.attachment_id.clone(),
                    status: ReprocessEntryStatus::UnknownAttachment,
                });
                continue;
            };
            // Reprocess covers PDF attachments exactly like the preview
            // (which reports `not_a_pdf` and no plan for anything else).
            if !is_pdf_attachment(
                attachment.content_type.as_deref(),
                attachment.filename.as_deref(),
            ) {
                results.push(ReprocessConfirmResult {
                    attachment_id: entry.attachment_id.clone(),
                    status: ReprocessEntryStatus::UnknownAttachment,
                });
                continue;
            }
            // The busy check is the fast path, never the authority: the plain
            // INSERT below is the race-proof admission.
            if live_extract_task_exists(conn, &entry.attachment_id)? {
                results.push(ReprocessConfirmResult {
                    attachment_id: entry.attachment_id.clone(),
                    status: ReprocessEntryStatus::Busy,
                });
                continue;
            }
            match insert_reprocess_extract_task(conn, &attachment, &entry.plan_hash)? {
                InsertOutcome::Inserted(task_id) => {
                    link_batch_task_subject(
                        conn,
                        &batch_id,
                        &task_id,
                        "bibliography_extract",
                        &entry.attachment_id,
                        "bibliography",
                        "attachment",
                        &entry.attachment_id,
                        None,
                    )?;
                    queued += 1;
                    results.push(ReprocessConfirmResult {
                        attachment_id: entry.attachment_id.clone(),
                        status: ReprocessEntryStatus::Queued,
                    });
                }
                InsertOutcome::Busy => {
                    results.push(ReprocessConfirmResult {
                        attachment_id: entry.attachment_id.clone(),
                        status: ReprocessEntryStatus::Busy,
                    });
                }
            }
        }
        let batch_id = if queued == 0 {
            // Nothing was queued: the empty batch never reaches the world.
            conn.execute("DELETE FROM processing_batches WHERE id = ?1", [&batch_id])
                .map_err(|error| format!("Failed to drop the empty reprocess batch: {error}"))?;
            None
        } else {
            Some(batch_id)
        };
        Ok(ReprocessConfirm { batch_id, results })
    })();
    match applied {
        Ok(response) => {
            conn.execute_batch("COMMIT")
                .map_err(|error| format!("Failed to commit the reprocess confirm: {error}"))?;
            Ok(response)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Clean prose a detector must never flag.
    const CLEAN: &str =
        "The quiet reading room holds every book the city ever loved, and the light falls softly.";

    /// Glue the B2 detector's rule 1 flags: 90 latin letters, all inside
    /// tokens longer than 24 letters.
    const GARBLED: &str =
        "abcdefghijklmnopqrstuvwxyzabcd fghijklmnopqrstuvwxyzabcd klmnopqrstuvwxyzabcdefghij";

    fn page(number: i64, quality: &str, text: &str) -> ExtractPageText {
        ExtractPageText {
            page_number: number,
            method: "native".to_string(),
            text_content: text.to_string(),
            text_hash: format!("native-hash-{number}"),
            text_chars: text.chars().count() as i64,
            quality: quality.to_string(),
        }
    }

    fn stored(number: i64, method: &str, text: &str) -> StoredPageInput {
        StoredPageInput {
            page_number: number,
            method: method.to_string(),
            text_hash: format!("stored-hash-{number}"),
            text_content: text.to_string(),
        }
    }

    fn input(
        native_pages: Vec<ExtractPageText>,
        stored_pages: Vec<StoredPageInput>,
        extraction_matches_source: bool,
    ) -> ReprocessPlanInput {
        ReprocessPlanInput {
            attachment_id: "att-1".to_string(),
            source_sha256: "source-sha".to_string(),
            native_pages,
            native_blank: false,
            stored_pages,
            extraction_matches_source,
        }
    }

    #[test]
    fn stored_ocr_rows_are_reused_even_when_empty_and_never_sent() {
        let plan = plan_reprocess(&input(
            vec![
                page(1, "rich", CLEAN),
                page(2, "sparse", "hi"),
                page(3, "empty", ""),
            ],
            vec![stored(2, "ocr", ""), stored(3, "ocr", "read by OCR before")],
            true,
        ));
        assert_eq!(plan.page_count, 3);
        assert!(
            plan.ocr_pages.is_empty(),
            "reused pages are never sent again: {:?}",
            plan.ocr_pages
        );
        assert_eq!(
            plan.reused_ocr_pages,
            vec![
                ReusedOcrPage {
                    page: 2,
                    text_hash: "stored-hash-2".to_string(),
                },
                ReusedOcrPage {
                    page: 3,
                    text_hash: "stored-hash-3".to_string(),
                },
            ],
            "empty OCR answers are settled pages and are reused as-is"
        );
    }

    #[test]
    fn nothing_is_reused_when_the_stored_extraction_does_not_match_the_source() {
        let plan = plan_reprocess(&input(
            vec![
                page(1, "rich", CLEAN),
                page(2, "sparse", "hi"),
                page(3, "empty", ""),
            ],
            vec![stored(2, "ocr", ""), stored(3, "ocr", "read by OCR before")],
            false,
        ));
        assert!(
            plan.reused_ocr_pages.is_empty(),
            "a replaced file invalidates every stored OCR row"
        );
        assert_eq!(plan.ocr_pages, vec![2, 3], "the candidates go out again");
    }

    #[test]
    fn a_rich_page_the_detector_flags_reaches_ocr() {
        let plan = plan_reprocess(&input(
            vec![page(1, "rich", CLEAN), page(2, "rich", GARBLED)],
            vec![],
            false,
        ));
        assert_eq!(
            plan.ocr_pages,
            vec![2],
            "the B2 detector makes it a candidate"
        );
    }

    #[test]
    fn fixed_without_ocr_counts_flagged_stored_pages_the_plan_leaves_alone() {
        let plan = plan_reprocess(&input(
            vec![
                page(1, "rich", CLEAN),
                page(2, "sparse", "hi"),
                page(3, "rich", CLEAN),
            ],
            vec![
                // Flagged stored text the clean native read repairs alone.
                stored(1, "native", GARBLED),
                // Flagged stored text, but the page goes to OCR: not "fixed
                // without OCR".
                stored(2, "native", GARBLED),
                // Flagged stored text on a reused OCR page: reused, not fixed.
                stored(3, "ocr", GARBLED),
            ],
            true,
        ));
        assert_eq!(plan.ocr_pages, vec![2]);
        assert_eq!(
            plan.reused_ocr_pages,
            vec![ReusedOcrPage {
                page: 3,
                text_hash: "stored-hash-3".to_string(),
            }]
        );
        assert_eq!(
            plan.fixed_without_ocr,
            vec![1],
            "only the flagged stored page neither sent nor reused"
        );
    }

    #[test]
    fn the_plan_hash_is_stable_under_input_reordering() {
        let forward = plan_reprocess(&input(
            vec![
                page(1, "rich", CLEAN),
                page(2, "sparse", "hi"),
                page(3, "sparse", "yo"),
            ],
            vec![
                stored(1, "native", CLEAN),
                stored(2, "ocr", "two"),
                stored(3, "native", "three"),
            ],
            true,
        ));
        let mut native = vec![
            page(3, "sparse", "yo"),
            page(1, "rich", CLEAN),
            page(2, "sparse", "hi"),
        ];
        native.reverse();
        let mut stored_rows = vec![
            stored(3, "native", "three"),
            stored(1, "native", CLEAN),
            stored(2, "ocr", "two"),
        ];
        stored_rows.reverse();
        let backward = plan_reprocess(&input(native, stored_rows, true));
        assert_eq!(forward.plan_hash, backward.plan_hash);
    }

    #[test]
    fn the_plan_hash_changes_with_the_source_the_ocr_set_and_the_reused_hash() {
        let base = plan_reprocess(&input(
            vec![page(1, "rich", CLEAN), page(2, "sparse", "hi")],
            vec![stored(1, "ocr", "one")],
            true,
        ));
        let mut changed_source = input(
            vec![page(1, "rich", CLEAN), page(2, "sparse", "hi")],
            vec![stored(1, "ocr", "one")],
            true,
        );
        changed_source.source_sha256 = "another-sha".to_string();
        assert_ne!(
            base.plan_hash,
            plan_reprocess(&changed_source).plan_hash,
            "a replaced PDF of the same size must move the hash (JD7-A-003)"
        );
        let changed_ocr_set = plan_reprocess(&input(
            vec![page(1, "rich", CLEAN), page(2, "rich", CLEAN)],
            vec![stored(1, "ocr", "one")],
            true,
        ));
        assert_ne!(
            base.plan_hash, changed_ocr_set.plan_hash,
            "page 2 leaving the OCR set must move the hash"
        );
        let mut changed_reused = input(
            vec![page(1, "rich", CLEAN), page(2, "sparse", "hi")],
            vec![stored(1, "ocr", "one")],
            true,
        );
        changed_reused.stored_pages[0].text_hash = "another-hash".to_string();
        assert_ne!(
            base.plan_hash,
            plan_reprocess(&changed_reused).plan_hash,
            "the reused text hash is what the executor will publish"
        );
    }

    #[test]
    fn the_plan_hash_changes_with_the_detector_version() {
        let reused = vec![ReusedOcrPage {
            page: 1,
            text_hash: "stored-hash-1".to_string(),
        }];
        let current = plan_hash("att-1", "source-sha", 2, &[2], &reused, 1);
        assert_ne!(
            current,
            plan_hash("att-1", "source-sha", 2, &[2], &reused, 2),
            "a new detector version must invalidate old approvals"
        );
    }

    #[test]
    fn the_plan_hash_payload_carries_no_contract() {
        let payload = plan_hash_payload("att-1", "source-sha", 1, &[], &[], 1);
        let text = String::from_utf8(payload).expect("payload is UTF-8");
        let value: serde_json::Value = serde_json::from_str(&text).expect("payload is JSON");
        let keys: BTreeSet<String> = value
            .as_object()
            .expect("payload is an object")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(
                [
                    "attachmentId",
                    "sourceSha256",
                    "pageCount",
                    "ocrPages",
                    "reusedOcr",
                    "detectorVersion",
                    "planVersion",
                ]
                .map(String::from)
            ),
            "exactly the contract-free payload of plan 2.3"
        );
        assert!(
            !text.contains("contract"),
            "no contract field, no circular hash"
        );
        assert!(
            !text.contains("reprocess"),
            "no contract string, no circular hash"
        );
        assert_eq!(
            plan_hash("att-1", "source-sha", 1, &[], &[], 1),
            format!("{:x}", Sha256::digest(text.as_bytes())),
            "plan_hash is the payload's SHA-256"
        );
    }

    #[test]
    fn the_estimate_is_pages_times_tokens_times_rate() {
        assert_eq!(estimated_usd(0), 0.0, "no OCR pages, no cost");
        assert!(
            (estimated_usd(1000) - 0.09).abs() < 1e-12,
            "1000 pages × 3000 tokens × USD 0.03/M = USD 0.09, got {}",
            estimated_usd(1000)
        );
    }
}
