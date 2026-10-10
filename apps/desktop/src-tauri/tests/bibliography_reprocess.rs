//! B3 integration tests (plan-texto-nativo-parte-b 2.3 "Comandos nuevos" 1
//! and 2, plus 2.4 "Una sola vez"): the reprocess candidate list and the
//! read-only preview over synthetic PDF attachments, plus the env-driven
//! measurement on a READ-ONLY copy of a real archive database.
//!
//! The measurement tests are `#[ignore]` and never write to the database they
//! measure — they open a COPY made with the SQLite online backup API:
//!
//! ```text
//! ENTROPIA_MEASURE_DB=G:/EntropIA-Stack/agent-scratch/prueba-sync-copy.sqlite \
//!   cargo test --test bibliography_reprocess -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicBool, Ordering};

use entropia_desktop_lib::bibliography::processing::{
    read_native_extraction_basis_with_cancel, BIBLIOGRAPHY_DETECTOR_VERSION,
};
use entropia_desktop_lib::bibliography::repository::{
    upsert_attachment, upsert_connection, upsert_item, upsert_library, AttachmentInput,
    BibliographicItemInput, ExtractionRow, LibraryType, PageTextRow, SourceOrigin,
    UpsertConnection, UpsertLibrary,
};
use entropia_desktop_lib::bibliography::reprocess::{
    estimated_usd, reprocess_candidates, reprocess_candidates_with, run_reprocess_preview,
    ReprocessCandidate, CANDIDATES_CANCELLED, REASON_EMPTY_WITHOUT_OCR, REASON_FAILED_OCR_ATTEMPT,
    REASON_GARBLED_STORED_PAGES, UNREADABLE_FILE_MISSING,
};
use entropia_desktop_lib::processing::recovery::run_checkpoint_cleanup;
use entropia_desktop_lib::processing::repository::{self as processing_repository, TaskSubject};
use lopdf::{dictionary, Document, Object, Stream};

const MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0032_batch_processing.sql");
const MIGRATION_0033_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0033_processing_source_invalidation.sql"
);
const MIGRATION_0040_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0040_bibliography_catalog.sql");
const MIGRATION_0041_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0041_bibliography_relations.sql");
const MIGRATION_0042_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0042_bibliography_reconciliation.sql");
const MIGRATION_0043_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0043_processing_task_subject_identity.sql"
);
const MIGRATION_0044_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0044_processing_task_subject_cutover.sql"
);
const MIGRATION_0045_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0045_bibliography_sync_tasks.sql");
const MIGRATION_0046_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0046_processing_priority.sql");
const MIGRATION_0047_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0047_bibliographic_semantic_profiles.sql"
);
const MIGRATION_0048_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0048_bibliography_profile_tasks.sql");
const MIGRATION_0049_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0049_bibliographic_index_generations.sql"
);
const MIGRATION_0050_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0050_bibliographic_embedding_generations.sql"
);
const MIGRATION_0051_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0051_bibliographic_profile_fts.sql");
const MIGRATION_0052_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0052_bibliographic_extraction_tasks.sql"
);
const MIGRATION_0053_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0053_bibliographic_page_texts.sql");
const MIGRATION_0054_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0054_bibliographic_chunks.sql");
const MIGRATION_0055_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0055_bibliographic_chunk_embeddings.sql"
);
const MIGRATION_0056_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0056_bibliographic_ingest_operations.sql"
);

/// Clean prose a detector must never flag (and `is_quality_text` grades rich).
const CLEAN: &str =
    "The quiet reading room holds every book the city ever loved, and the light falls softly.";

/// Glue both detectors flag: 90 latin letters, all inside tokens longer
/// than 24 letters (B2 rule 1).
const GARBLED: &str =
    "abcdefghijklmnopqrstuvwxyzabcd fghijklmnopqrstuvwxyzabcd klmnopqrstuvwxyzabcdefghij";

fn migrated_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("entropia.sqlite");
    let conn = entropia_desktop_lib::db_open_for_tests(&path);
    conn.execute_batch(
        "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, size INTEGER, parent_asset_id TEXT, page_number INTEGER, created_at INTEGER NOT NULL);
         CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);
         CREATE TABLE layouts (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, regions TEXT NOT NULL, blocks TEXT NOT NULL, model TEXT NOT NULL, image_width INTEGER NOT NULL, image_height INTEGER NOT NULL, created_at INTEGER NOT NULL);
         CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, model TEXT NOT NULL, created_at INTEGER NOT NULL);
         CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);
         CREATE UNIQUE INDEX idx_extractions_asset_id_unique ON extractions(asset_id);
         CREATE UNIQUE INDEX idx_layouts_asset_id_unique ON layouts(asset_id);
         CREATE TABLE llm_results (id TEXT PRIMARY KEY, target_id TEXT NOT NULL, target_type TEXT NOT NULL DEFAULT 'unknown', job_type TEXT NOT NULL, result TEXT NOT NULL, created_at INTEGER NOT NULL);
         CREATE TABLE ocr_correction_backups (asset_id TEXT PRIMARY KEY, text_content TEXT NOT NULL);",
    )
    .expect("base tables");
    for (sql, name) in [
        (MIGRATION_SQL, "0032_batch_processing"),
        (MIGRATION_0033_SQL, "0033_processing_source_invalidation"),
        (MIGRATION_0040_SQL, "0040_bibliography_catalog"),
        (MIGRATION_0041_SQL, "0041_bibliography_relations"),
        (MIGRATION_0042_SQL, "0042_bibliography_reconciliation"),
        (MIGRATION_0043_SQL, "0043_processing_task_subject_identity"),
        (MIGRATION_0044_SQL, "0044_processing_task_subject_cutover"),
        (MIGRATION_0045_SQL, "0045_bibliography_sync_tasks"),
        (MIGRATION_0046_SQL, "0046_processing_priority"),
        (MIGRATION_0047_SQL, "0047_bibliographic_semantic_profiles"),
        (MIGRATION_0048_SQL, "0048_bibliography_profile_tasks"),
        (MIGRATION_0049_SQL, "0049_bibliographic_index_generations"),
        (
            MIGRATION_0050_SQL,
            "0050_bibliographic_embedding_generations",
        ),
        (MIGRATION_0051_SQL, "0051_bibliographic_profile_fts"),
        (MIGRATION_0052_SQL, "0052_bibliographic_extraction_tasks"),
        (MIGRATION_0053_SQL, "0053_bibliographic_page_texts"),
        (MIGRATION_0054_SQL, "0054_bibliographic_chunks"),
        (MIGRATION_0055_SQL, "0055_bibliographic_chunk_embeddings"),
        (MIGRATION_0056_SQL, "0056_bibliographic_ingest_operations"),
    ] {
        conn.execute_batch(sql).expect("apply migration");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
            [name],
        )
        .expect("track migration");
    }
    (dir, conn)
}

/// Seeds one catalog work and returns its item row id.
fn seed_item(conn: &mut rusqlite::Connection, item_key: &str, title: &str) -> String {
    let source = upsert_connection(
        conn,
        UpsertConnection {
            id: "conn-1".to_string(),
            source_origin: SourceOrigin::Local,
            source_instance_id: None,
            endpoint: Some("http://synthetic.invalid".to_string()),
            capabilities_json: r#"{"read":true}"#.to_string(),
        },
    )
    .expect("seed connection");
    let library = upsert_library(
        conn,
        UpsertLibrary {
            connection_id: source.id,
            library_type: LibraryType::User,
            library_id: "0".to_string(),
            name: "Personal".to_string(),
            last_modified_version: Some(7),
        },
    )
    .expect("seed library");
    let item = upsert_item(
        conn,
        &library.id,
        BibliographicItemInput {
            item_key: item_key.to_string(),
            item_version: Some(3),
            native_json_snapshot: serde_json::json!({ "key": item_key, "itemType": "book" })
                .to_string(),
            csl_json_snapshot:
                serde_json::json!({ "id": item_key, "type": "book", "title": title }).to_string(),
            title: Some(title.to_string()),
            ..Default::default()
        },
    )
    .expect("seed item");
    item.id
}

/// Seeds one attachment at the pinned identity every test compares against.
fn seed_attachment(
    conn: &mut rusqlite::Connection,
    item_id: &str,
    attachment_key: &str,
    native_path: Option<&str>,
    filename: &str,
) -> String {
    upsert_attachment(
        conn,
        item_id,
        AttachmentInput {
            attachment_key: attachment_key.to_string(),
            content_type: Some("application/pdf".to_string()),
            link_mode: Some("linked_file".to_string()),
            filename: Some(filename.to_string()),
            native_path: native_path.map(String::from),
            url: None,
            md5: None,
            mtime: Some(TEST_MTIME),
            native_json_snapshot: serde_json::json!({
                "key": attachment_key, "itemType": "attachment", "linkMode": "linked_file",
                "contentType": "application/pdf",
            })
            .to_string(),
            native_version: Some(3),
        },
    )
    .expect("seed attachment")
    .id
}

/// The catalog mtime every seeded attachment pins.
const TEST_MTIME: i64 = 1_700_000_000;

fn plant_extraction(
    conn: &rusqlite::Connection,
    attachment_id: &str,
    item_id: &str,
    quality: &str,
    source_mtime: Option<i64>,
    source_bytes: i64,
) {
    entropia_desktop_lib::bibliography::repository::upsert_extraction_in_transaction(
        conn,
        &ExtractionRow {
            attachment_id: attachment_id.to_string(),
            item_id: item_id.to_string(),
            page_count: 1,
            method: "native".to_string(),
            text_content: String::new(),
            text_hash: "extraction-hash".to_string(),
            text_chars: 0,
            quality: quality.to_string(),
            source_mtime,
            source_bytes,
        },
        processing_repository::now_ms(),
    )
    .expect("plant extraction");
}

fn plant_page(
    conn: &rusqlite::Connection,
    attachment_id: &str,
    page_number: i64,
    method: &str,
    quality: &str,
    text: &str,
) {
    entropia_desktop_lib::bibliography::repository::upsert_page_text_in_transaction(
        conn,
        &PageTextRow {
            attachment_id: attachment_id.to_string(),
            page_number,
            method: method.to_string(),
            text_content: text.to_string(),
            text_hash: format!("page-hash-{attachment_id}-{page_number}"),
            text_chars: text.chars().count() as i64,
            quality: quality.to_string(),
        },
        processing_repository::now_ms(),
    )
    .expect("plant page row");
}

/// Admits one `bibliography_extract` task through the real admission (so the
/// fingerprint carries the real file identity prefix) and returns its id.
fn admit_extract_task(conn: &rusqlite::Connection, attachment_id: &str) -> String {
    let batch = processing_repository::ensure_system_batch(conn, "bibliography").expect("batch");
    processing_repository::admit_subject_or_attach(
        conn,
        &batch,
        "bibliography_extract",
        &TaskSubject {
            domain: "bibliography".to_string(),
            subject_kind: "attachment".to_string(),
            subject_id: attachment_id.to_string(),
        },
        0,
        "",
        "",
        None,
    )
    .expect("admit extraction task")
    .expect("no live reprocess reserves the attachment in this test")
    .task_id
}

fn settle_task(conn: &rusqlite::Connection, task_id: &str, state: &str, code: Option<&str>) {
    conn.execute(
        "UPDATE processing_tasks SET state = ?2, last_error_code = ?3 WHERE id = ?1",
        rusqlite::params![task_id, state, code],
    )
    .expect("settle task");
}

fn plant_receipt(conn: &rusqlite::Connection, task_id: &str, receipt: serde_json::Value) {
    conn.execute(
        "UPDATE processing_tasks SET result_receipt_json = ?2 WHERE id = ?1",
        rusqlite::params![task_id, receipt.to_string()],
    )
    .expect("plant receipt");
}

fn candidate_for<'a>(
    candidates: &'a [ReprocessCandidate],
    attachment_id: &str,
) -> Option<&'a ReprocessCandidate> {
    candidates
        .iter()
        .find(|candidate| candidate.attachment_id == attachment_id)
}

/// Builds a PDF with Helvetica text lines at explicit positions, one entry
/// per page. Pure lopdf synthesis — no fixtures, no OCR.
fn make_text_pdf_pages(pages: &[&[(f32, f32, &str)]]) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let pages_id = document.new_object_id();
    let mut page_ids = Vec::with_capacity(pages.len());
    for lines in pages {
        let mut content = String::from("BT /F1 12 Tf ");
        for (x, y, text) in lines.iter() {
            let escaped = text
                .replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)");
            content.push_str(&format!("{x} {y} Td ({escaped}) Tj "));
        }
        content.push_str("ET");
        let content_id = document.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = document.new_object_id();
        document.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                "Contents" => content_id,
            }),
        );
        page_ids.push(Object::Reference(page_id));
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids,
            "Count" => pages.len() as i64,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document
        .save_to(&mut bytes)
        .expect("serialize synthetic PDF");
    bytes
}

fn write_pdf(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("write synthetic PDF");
    path.to_string_lossy().to_string()
}

// ── Candidates (2.3 "Comandos nuevos" 1) ───────────────────────────────────

/// (a): a stored page the detectors flag lists the attachment; a clean one
/// does not.
#[test]
fn candidates_flag_attachments_with_garbled_stored_pages() {
    let (dir, mut conn) = migrated_db();
    let garbled_item = seed_item(&mut conn, "GARB00001", "A obra garbled");
    let garbled_path = write_pdf(
        &dir,
        "garbled.pdf",
        &make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]),
    );
    let garbled = seed_attachment(
        &mut conn,
        &garbled_item,
        "GARBATT1",
        Some(&garbled_path),
        "garbled.pdf",
    );
    plant_extraction(&conn, &garbled, &garbled_item, "rich", Some(TEST_MTIME), 10);
    plant_page(&conn, &garbled, 1, "native", "rich", GARBLED);

    let clean_item = seed_item(&mut conn, "CLEAN0001", "B obra limpia");
    let clean_path = write_pdf(
        &dir,
        "clean.pdf",
        &make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]),
    );
    let clean = seed_attachment(
        &mut conn,
        &clean_item,
        "CLEANATT",
        Some(&clean_path),
        "clean.pdf",
    );
    plant_extraction(&conn, &clean, &clean_item, "rich", Some(TEST_MTIME), 10);
    plant_page(&conn, &clean, 1, "native", "rich", CLEAN);

    let candidates = reprocess_candidates(&conn).expect("candidates");
    let garbled_row =
        candidate_for(&candidates, &garbled).expect("the garbled attachment is listed");
    assert_eq!(
        garbled_row.reasons,
        vec![REASON_GARBLED_STORED_PAGES],
        "rule (a) fires on the flagged stored page"
    );
    assert_eq!(garbled_row.flagged_pages, 1);
    assert_eq!(garbled_row.title, "A obra garbled");
    assert_eq!(garbled_row.filename.as_deref(), Some("garbled.pdf"));
    assert!(
        candidate_for(&candidates, &clean).is_none(),
        "clean stored text is not a candidate"
    );
    assert!(
        candidates
            .windows(2)
            .all(|pair| pair[0].title <= pair[1].title),
        "candidates are ordered by work title"
    );
}

/// (b): an `empty` extraction with no OCR answer for the current file is a
/// candidate; a succeeded `"ocrAttempted": true` receipt for the SAME file
/// settles it — and only that receipt does.
#[test]
fn candidates_include_empty_extractions_without_an_ocr_receipt() {
    let (dir, mut conn) = migrated_db();
    let items: Vec<(String, String, String)> = [
        ("EMP000001", "C obra vacia sin recibo"),
        ("EMP000002", "D obra vacia con recibo"),
        ("EMP000003", "E obra vacia recibo falso"),
        ("EMP000004", "F obra vacia recibo de otro archivo"),
    ]
    .iter()
    .enumerate()
    .map(|(index, (key, title))| {
        let item_id = seed_item(&mut conn, key, title);
        let path = write_pdf(
            &dir,
            &format!("empty-{index}.pdf"),
            &make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]),
        );
        let attachment_id = seed_attachment(
            &mut conn,
            &item_id,
            &format!("EMPATT{index:02}"),
            Some(&path),
            &format!("empty-{index}.pdf"),
        );
        plant_extraction(
            &conn,
            &attachment_id,
            &item_id,
            "empty",
            Some(TEST_MTIME),
            10,
        );
        (item_id, attachment_id, key.to_string())
    })
    .collect();

    let no_receipt = &items[0].1;
    let answered = &items[1].1;
    let lied = &items[2].1;
    let other_file = &items[3].1;

    let task = admit_extract_task(&conn, answered);
    settle_task(&conn, &task, "succeeded", None);
    plant_receipt(&conn, &task, serde_json::json!({ "ocrAttempted": true }));

    let task = admit_extract_task(&conn, lied);
    settle_task(&conn, &task, "succeeded", None);
    plant_receipt(&conn, &task, serde_json::json!({ "ocrAttempted": false }));

    let task = admit_extract_task(&conn, other_file);
    settle_task(&conn, &task, "succeeded", None);
    plant_receipt(&conn, &task, serde_json::json!({ "ocrAttempted": true }));
    conn.execute(
        "UPDATE processing_tasks SET input_fingerprint = ?2 WHERE id = ?1",
        rusqlite::params![task, format!("attachment|{other_file}|mtime:999|version:3")],
    )
    .expect("re-pin the receipt to another file");

    let candidates = reprocess_candidates(&conn).expect("candidates");
    assert_eq!(
        candidate_for(&candidates, no_receipt)
            .expect("the unanswered empty extraction is listed")
            .reasons,
        vec![REASON_EMPTY_WITHOUT_OCR],
        "rule (b) fires without any OCR answer"
    );
    assert!(
        candidate_for(&candidates, answered).is_none(),
        "an OCR pass that answered for this exact file settled the empty verdict"
    );
    assert_eq!(
        candidate_for(&candidates, lied)
            .expect("a receipt without ocrAttempted changes nothing")
            .reasons,
        vec![REASON_EMPTY_WITHOUT_OCR]
    );
    assert_eq!(
        candidate_for(&candidates, other_file)
            .expect("a receipt of ANOTHER file does not settle this one")
            .reasons,
        vec![REASON_EMPTY_WITHOUT_OCR]
    );
}

/// (c): a terminal failed OCR attempt or any cancelled attempt on the
/// current file is one spent attempt the owner recovers here; read failures
/// and attempts on an older file are not.
#[test]
fn candidates_include_attachments_with_a_spent_ocr_attempt() {
    let (dir, mut conn) = migrated_db();
    fn spawn(
        conn: &mut rusqlite::Connection,
        dir: &tempfile::TempDir,
        key: &str,
        title: &str,
        index: usize,
    ) -> String {
        let item_id = seed_item(conn, key, title);
        let path = write_pdf(
            dir,
            &format!("attempt-{index}.pdf"),
            &make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]),
        );
        let attachment_id = seed_attachment(
            conn,
            &item_id,
            &format!("ATTMPATT{index:02}"),
            Some(&path),
            &format!("attempt-{index}.pdf"),
        );
        plant_extraction(conn, &attachment_id, &item_id, "rich", Some(TEST_MTIME), 10);
        attachment_id
    }
    let failed = spawn(&mut conn, &dir, "ATT000001", "G obra fallida", 1);
    let read_failed = spawn(
        &mut conn,
        &dir,
        "ATT000002",
        "H obra con falla de lectura",
        2,
    );
    let cancelled = spawn(&mut conn, &dir, "ATT000003", "I obra cancelada", 3);
    let stale = spawn(&mut conn, &dir, "ATT000004", "J obra de otro archivo", 4);

    let task = admit_extract_task(&conn, &failed);
    settle_task(&conn, &task, "failed", Some("provider_transient"));

    let task = admit_extract_task(&conn, &read_failed);
    settle_task(&conn, &task, "failed", Some("extraction_io"));

    let task = admit_extract_task(&conn, &cancelled);
    settle_task(&conn, &task, "cancelled", None);

    let task = admit_extract_task(&conn, &stale);
    settle_task(&conn, &task, "failed", Some("provider_transient"));
    conn.execute(
        "UPDATE processing_tasks SET input_fingerprint = ?2 WHERE id = ?1",
        rusqlite::params![task, format!("attachment|{stale}|mtime:999|version:3")],
    )
    .expect("re-pin the attempt to another file");

    let candidates = reprocess_candidates(&conn).expect("candidates");
    assert_eq!(
        candidate_for(&candidates, &failed)
            .expect("the spent OCR attempt is listed")
            .reasons,
        vec![REASON_FAILED_OCR_ATTEMPT]
    );
    assert_eq!(
        candidate_for(&candidates, &cancelled)
            .expect("a cancelled paid run is listed")
            .reasons,
        vec![REASON_FAILED_OCR_ATTEMPT]
    );
    assert!(
        candidate_for(&candidates, &read_failed).is_none(),
        "a read failure is still re-demanded automatically, never reprocessed"
    );
    assert!(
        candidate_for(&candidates, &stale).is_none(),
        "an attempt on an older file is not an attempt on this one"
    );
}

/// 2.4: a successful reprocess of the CURRENT file at the CURRENT detector
/// version removes the attachment from the list — and a changed file or a
/// stale detector version brings it back.
#[test]
fn candidates_exclude_a_successful_reprocess_of_the_current_file() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "REPR00001", "K obra reprocesada");
    let path = write_pdf(
        &dir,
        "reprocessed.pdf",
        &make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]),
    );
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "REPRATT1",
        Some(&path),
        "reprocessed.pdf",
    );
    plant_extraction(
        &conn,
        &attachment_id,
        &item_id,
        "rich",
        Some(TEST_MTIME),
        4242,
    );
    plant_page(&conn, &attachment_id, 1, "native", "rich", GARBLED);

    let receipt = |detector_version: u32, source_bytes: i64| {
        serde_json::json!({
            "attachmentId": attachment_id,
            "reprocess": { "planHash": "cafebabe", "detectorVersion": detector_version },
            "sourceMtime": TEST_MTIME,
            "sourceBytes": source_bytes,
            "ocrFailedPages": [],
        })
    };

    let task = admit_extract_task(&conn, &attachment_id);
    settle_task(&conn, &task, "succeeded", None);
    // The receipt must name the CURRENT detector version: the exclusion is
    // keyed on it, so the fixture reads it from the constant, not a literal.
    plant_receipt(&conn, &task, receipt(BIBLIOGRAPHY_DETECTOR_VERSION, 4242));
    assert!(
        reprocess_candidates(&conn).expect("candidates").is_empty(),
        "the current file already went through a reprocess at this detector version"
    );

    let stale_detector = admit_extract_task(&conn, &attachment_id);
    settle_task(&conn, &stale_detector, "succeeded", None);
    plant_receipt(&conn, &stale_detector, receipt(0, 4242));
    // The matching receipt from the first task still excludes the file.
    assert!(
        reprocess_candidates(&conn).expect("candidates").is_empty(),
        "one matching receipt is enough"
    );

    // The file changes (new catalog mtime): the old receipts cover the old
    // file and the attachment comes back (2.4 "si cambia el archivo, vuelve
    // a aparecer").
    conn.execute(
        "UPDATE zotero_attachments SET mtime = ?2 WHERE id = ?1",
        rusqlite::params![attachment_id, TEST_MTIME + 1000],
    )
    .expect("replace the file");
    let candidates = reprocess_candidates(&conn).expect("candidates");
    assert_eq!(
        candidate_for(&candidates, &attachment_id)
            .expect("the changed file is listed again")
            .reasons,
        vec![REASON_GARBLED_STORED_PAGES]
    );
}

/// JD8-B-001 (2.4): the exclusion is only for a COMPLETE repair. A reprocess
/// that succeeded with failed pages — its `ocrFailedPages` is not empty —
/// left the damaged text standing, so the attachment must stay a candidate.
#[test]
fn a_successful_reprocess_with_failed_pages_stays_a_candidate() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "REPRFAIL1", "KA obra a medias");
    let path = write_pdf(
        &dir,
        "parcial.pdf",
        &make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]),
    );
    let attachment_id =
        seed_attachment(&mut conn, &item_id, "REPRFAILA", Some(&path), "parcial.pdf");
    plant_extraction(
        &conn,
        &attachment_id,
        &item_id,
        "rich",
        Some(TEST_MTIME),
        4242,
    );
    plant_page(&conn, &attachment_id, 1, "native", "rich", GARBLED);

    let task = admit_extract_task(&conn, &attachment_id);
    settle_task(&conn, &task, "succeeded", None);
    plant_receipt(
        &conn,
        &task,
        serde_json::json!({
            "attachmentId": attachment_id,
            "reprocess": {
                "planHash": "cafebabe",
                "detectorVersion": BIBLIOGRAPHY_DETECTOR_VERSION
            },
            "sourceMtime": TEST_MTIME,
            "sourceBytes": 4242,
            "ocrFailedPages": [1],
        }),
    );

    let candidates = reprocess_candidates(&conn).expect("candidates");
    assert_eq!(
        candidate_for(&candidates, &attachment_id)
            .expect("a repair that failed on some pages is no completed repair")
            .reasons,
        vec![REASON_GARBLED_STORED_PAGES]
    );
}

/// The scan reports one `(done, total)` step after every checked attachment:
/// the total is known from the first SELECT, the counter reaches it exactly
/// and never goes backwards.
#[test]
fn candidates_scan_reports_progress_per_checked_attachment() {
    let (dir, mut conn) = migrated_db();
    let garbled_item = seed_item(&mut conn, "PROG00001", "A obra con dano");
    let garbled_path = write_pdf(
        &dir,
        "prog-garbled.pdf",
        &make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]),
    );
    let garbled = seed_attachment(
        &mut conn,
        &garbled_item,
        "PROGATT1",
        Some(&garbled_path),
        "prog-garbled.pdf",
    );
    plant_extraction(&conn, &garbled, &garbled_item, "rich", Some(TEST_MTIME), 10);
    plant_page(&conn, &garbled, 1, "native", "rich", GARBLED);
    for index in 0..2 {
        let item_id = seed_item(
            &mut conn,
            &format!("PROGC{index:03}"),
            &format!("B obra limpia {index}"),
        );
        let path = write_pdf(
            &dir,
            &format!("prog-clean-{index}.pdf"),
            &make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]),
        );
        let attachment_id = seed_attachment(
            &mut conn,
            &item_id,
            &format!("PROGCAT{index}"),
            Some(&path),
            &format!("prog-clean-{index}.pdf"),
        );
        plant_extraction(
            &conn,
            &attachment_id,
            &item_id,
            "rich",
            Some(TEST_MTIME),
            10,
        );
        plant_page(&conn, &attachment_id, 1, "native", "rich", CLEAN);
    }

    let reports = std::cell::RefCell::new(Vec::new());
    let candidates = reprocess_candidates_with(&conn, &AtomicBool::new(false), |done, total| {
        reports.borrow_mut().push((done, total));
    })
    .expect("scan");
    let reports = reports.into_inner();

    assert!(
        candidate_for(&candidates, &garbled).is_some(),
        "the garbled attachment is still listed: {candidates:?}"
    );
    assert_eq!(
        reports.len(),
        3,
        "one report per checked attachment: {reports:?}"
    );
    assert!(
        reports.iter().all(|&(_, total)| total == 3),
        "the total is known up front: {reports:?}"
    );
    assert_eq!(
        reports.last().copied(),
        Some((3, 3)),
        "the counter reaches the total: {reports:?}"
    );
    assert!(
        reports.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "done never goes backwards: {reports:?}"
    );
}

/// The cancel flag stops the scan between attachments and answers the
/// cancelled outcome: a flag set before the first attachment checks nothing,
/// a flag set from inside the callback after the first report stops there.
#[test]
fn candidates_scan_stops_on_the_cancel_flag_with_the_cancelled_outcome() {
    let (dir, mut conn) = migrated_db();
    for index in 0..3 {
        let item_id = seed_item(
            &mut conn,
            &format!("STOP{index:04}"),
            &format!("C obra {index}"),
        );
        let path = write_pdf(
            &dir,
            &format!("stop-{index}.pdf"),
            &make_text_pdf_pages(&[&[(72.0, 700.0, GARBLED)]]),
        );
        let attachment_id = seed_attachment(
            &mut conn,
            &item_id,
            &format!("STOPATT{index}"),
            Some(&path),
            &format!("stop-{index}.pdf"),
        );
        plant_extraction(
            &conn,
            &attachment_id,
            &item_id,
            "rich",
            Some(TEST_MTIME),
            10,
        );
        plant_page(&conn, &attachment_id, 1, "native", "rich", GARBLED);
    }

    let pre_set = AtomicBool::new(true);
    let reports = std::cell::RefCell::new(Vec::new());
    let outcome = reprocess_candidates_with(&conn, &pre_set, |done, total| {
        reports.borrow_mut().push((done, total));
    });
    assert_eq!(
        outcome.unwrap_err(),
        CANDIDATES_CANCELLED,
        "a pre-set flag answers the cancelled outcome"
    );
    assert!(
        reports.into_inner().is_empty(),
        "nothing is checked past a set flag"
    );

    let cancel = AtomicBool::new(false);
    let reports = std::cell::RefCell::new(Vec::new());
    let outcome = {
        let reports = &reports;
        reprocess_candidates_with(&conn, &cancel, |done, total| {
            reports.borrow_mut().push((done, total));
            // Fire once the first report lands: the stop hits between rows.
            cancel.store(true, Ordering::SeqCst);
        })
    };
    assert_eq!(
        outcome.unwrap_err(),
        CANDIDATES_CANCELLED,
        "a flag set mid-scan answers the cancelled outcome"
    );
    assert_eq!(
        reports.into_inner(),
        vec![(1, 3)],
        "exactly one attachment is checked before the stop"
    );
}

// ── Preview (2.3 "Comandos nuevos" 2) ──────────────────────────────────────

/// A three-page synthetic PDF: page 1 rich and clean (its STORED row is
/// garbled → fixed without OCR), page 2 sparse (goes to OCR), page 3 sparse
/// with a stored `ocr` row of the matching source (reused, never re-sent).
#[test]
fn preview_totals_count_ocr_reused_and_fixed_pages() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "PREV00001", "L obra con plan");
    let pdf = make_text_pdf_pages(&[
        &[(72.0, 700.0, CLEAN)],
        &[(72.0, 700.0, "hi")],
        &[(72.0, 700.0, "yo")],
    ]);
    let path = write_pdf(&dir, "plan.pdf", &pdf);
    let attachment_id = seed_attachment(&mut conn, &item_id, "PREVATT1", Some(&path), "plan.pdf");
    plant_extraction(
        &conn,
        &attachment_id,
        &item_id,
        "rich",
        Some(TEST_MTIME),
        pdf.len() as i64,
    );
    plant_page(&conn, &attachment_id, 1, "native", "rich", GARBLED);
    plant_page(
        &conn,
        &attachment_id,
        3,
        "ocr",
        "sparse",
        "read by OCR before",
    );

    let cancel = AtomicBool::new(false);
    let mut progress = Vec::new();
    let preview = run_reprocess_preview(
        &conn,
        std::slice::from_ref(&attachment_id),
        &cancel,
        |update| progress.push(update),
    )
    .expect("preview");

    let units_total = 3 * 3;
    assert_eq!(
        progress.first().map(|update| (
            update.done,
            update.total,
            update.units_done,
            update.units_total
        )),
        Some((1, 1, 0, 0)),
        "the unit counter resets at the attachment boundary"
    );
    assert_eq!(
        progress.last().map(|update| (
            update.done,
            update.total,
            update.units_done,
            update.units_total
        )),
        Some((1, 1, units_total, units_total)),
        "progress ends at the attachment's full unit count"
    );
    assert!(!preview.cancelled);
    assert_eq!(preview.attachments.len(), 1);
    let entry = &preview.attachments[0];
    assert_eq!(entry.attachment_id, attachment_id);
    assert_eq!(entry.item_id, item_id);
    assert_eq!(entry.title, "L obra con plan");
    assert_eq!(entry.filename.as_deref(), Some("plan.pdf"));
    assert_eq!(entry.page_count, 3);
    assert_eq!(entry.ocr_pages, 1, "only the sparse page goes to GLM-OCR");
    assert_eq!(entry.reused_ocr_pages, 1, "the stored OCR row is reused");
    assert_eq!(
        entry.fixed_without_ocr, 1,
        "the flagged stored page is repaired by the read alone"
    );
    assert!(
        entry
            .plan_hash
            .as_deref()
            .is_some_and(|hash| hash.len() == 64),
        "a readable attachment carries its plan hash"
    );
    assert!(!entry.busy);
    assert!(entry.unreadable.is_none());
    assert_eq!(preview.totals.attachments, 1);
    assert_eq!(preview.totals.pages, 3);
    assert_eq!(preview.totals.ocr_pages, 1);
    assert_eq!(preview.totals.reused_ocr_pages, 1);
    assert_eq!(preview.totals.fixed_without_ocr, 1);
    assert_eq!(
        preview.totals.estimated_usd,
        estimated_usd(1),
        "the estimate follows the OCR page count"
    );
    assert!(preview.totals.estimated_usd > 0.0);
}

/// A live `bibliography_extract` task marks the entry `busy` — the preview
/// still plans it, but the confirm path will refuse to queue it.
#[test]
fn preview_marks_attachments_with_a_live_task_busy() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "BUSY00001", "M obra ocupada");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]);
    let path = write_pdf(&dir, "busy.pdf", &pdf);
    let attachment_id = seed_attachment(&mut conn, &item_id, "BUSYATT1", Some(&path), "busy.pdf");
    plant_extraction(
        &conn,
        &attachment_id,
        &item_id,
        "rich",
        Some(TEST_MTIME),
        pdf.len() as i64,
    );
    let task_id = admit_extract_task(&conn, &attachment_id);

    let cancel = AtomicBool::new(false);
    let preview =
        run_reprocess_preview(&conn, std::slice::from_ref(&attachment_id), &cancel, |_| {})
            .expect("preview");
    assert!(preview.attachments[0].busy, "a pending task is a live task");
    assert!(preview.attachments[0].unreadable.is_none());

    settle_task(&conn, &task_id, "succeeded", None);
    let preview =
        run_reprocess_preview(&conn, std::slice::from_ref(&attachment_id), &cancel, |_| {})
            .expect("preview");
    assert!(
        !preview.attachments[0].busy,
        "a settled task is not a live task"
    );
}

/// An attachment whose file cannot be resolved is unreadable: a named
/// reason, no plan hash, and zero weight in the totals.
#[test]
fn preview_reports_a_missing_file_unreadable() {
    let (_dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "MISS00001", "N obra sin archivo");
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "MISSATT1",
        Some("Z:/nowhere/missing.pdf"),
        "missing.pdf",
    );

    let cancel = AtomicBool::new(false);
    let preview =
        run_reprocess_preview(&conn, std::slice::from_ref(&attachment_id), &cancel, |_| {})
            .expect("preview");
    let entry = &preview.attachments[0];
    assert_eq!(entry.unreadable.as_deref(), Some(UNREADABLE_FILE_MISSING));
    assert!(
        entry.plan_hash.is_none(),
        "an unreadable attachment has no plan"
    );
    assert_eq!(entry.page_count, 0);
    assert!(!entry.busy);
    assert_eq!(preview.totals.attachments, 1);
    assert_eq!(preview.totals.pages, 0);
    assert_eq!(preview.totals.estimated_usd, 0.0);
}

/// The work-unit stream inside one attachment: `3 × pages` units (lopdf
/// pass pages + PDFium pass pages + whole-document extract pages), never
/// decreasing and settled at the full count even where a pass is skipped or
/// falls back to lopdf.
#[test]
fn preview_reports_work_units_inside_the_current_attachment() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "UNITS00001", "O obra con unidades");
    let pdf = make_text_pdf_pages(&[
        &[(72.0, 700.0, CLEAN)],
        &[(72.0, 700.0, "hi")],
        &[(72.0, 700.0, "yo")],
    ]);
    let path = write_pdf(&dir, "units.pdf", &pdf);
    let attachment_id = seed_attachment(&mut conn, &item_id, "UNITSATT1", Some(&path), "units.pdf");

    let cancel = AtomicBool::new(false);
    let mut progress = Vec::new();
    let preview = run_reprocess_preview(
        &conn,
        std::slice::from_ref(&attachment_id),
        &cancel,
        |update| progress.push(update),
    )
    .expect("preview");

    assert!(!preview.cancelled);
    let units_total = 3 * 3;
    assert_eq!(
        progress.first().map(|update| (
            update.done,
            update.total,
            update.units_done,
            update.units_total
        )),
        Some((1, 1, 0, 0)),
        "the unit counter resets at the attachment boundary"
    );
    assert!(
        progress
            .iter()
            .all(|update| (update.done, update.total) == (1, 1)),
        "every event names the one attachment: {progress:?}"
    );
    let units: Vec<(i64, i64)> = progress
        .iter()
        .filter(|update| update.units_total > 0)
        .map(|update| (update.units_done, update.units_total))
        .collect();
    assert!(
        units.iter().all(|(_, total)| *total == units_total),
        "the total is 3 × pages at every report: {units:?}"
    );
    assert!(
        units.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "units never move backwards: {units:?}"
    );
    for page in 1..=3 {
        assert!(
            units.contains(&(page, units_total)),
            "the lopdf pass reports every page: {units:?}"
        );
        assert!(
            units.contains(&(2 * 3 + page, units_total)),
            "the extract pass contributes one unit per page: {units:?}"
        );
    }
    assert_eq!(
        units.last().copied(),
        Some((units_total, units_total)),
        "the read ends at the full unit count"
    );
}

/// The basis reader's unit callback, driven directly on a multi-page PDF:
/// every report carries the attachment total `3 × pages`, the counts never
/// decrease, the extract pass contributes one unit per page, and the stream
/// ends on the total.
#[test]
fn native_basis_units_cover_both_passes_and_the_document_extract() {
    let pdf = make_text_pdf_pages(&[
        &[(72.0, 700.0, CLEAN)],
        &[(72.0, 700.0, "hi")],
        &[(72.0, 700.0, "yo")],
    ]);
    let mut units: Vec<(i64, i64)> = Vec::new();
    read_native_extraction_basis_with_cancel(
        &pdf,
        None,
        None,
        Some(&mut |done: i64, total: i64| units.push((done, total))),
    )
    .expect("basis");

    let units_total = 3 * 3;
    assert!(!units.is_empty(), "the callback fires");
    assert!(
        units.iter().all(|(_, total)| *total == units_total),
        "every report carries the attachment total: {units:?}"
    );
    assert!(
        units.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "units never move backwards: {units:?}"
    );
    for page in 1..=3 {
        assert!(
            units.contains(&(2 * 3 + page, units_total)),
            "the extract pass contributes one unit per page: {units:?}"
        );
    }
    assert_eq!(
        units.last().copied(),
        Some((units_total, units_total)),
        "the extract pass lands the final unit"
    );
}

/// Cancel set DURING the whole-document extract stops the pass at the next
/// page boundary and answers the cancelled outcome: the T3 stall at 99 %
/// (the extract counted as one unit) must be interruptible like every
/// per-page pass.
#[test]
fn native_basis_cancel_during_the_document_extract_stops_with_cancelled() {
    use entropia_desktop_lib::processing::scheduler::ExecOutput;
    let pdf = make_text_pdf_pages(&[
        &[(72.0, 700.0, CLEAN)],
        &[(72.0, 700.0, "hi")],
        &[(72.0, 700.0, "yo")],
    ]);
    let cancel = AtomicBool::new(false);
    let mut units: Vec<(i64, i64)> = Vec::new();
    let outcome = read_native_extraction_basis_with_cancel(
        &pdf,
        None,
        Some(&cancel),
        Some(&mut |done: i64, total: i64| {
            units.push((done, total));
            // The extract pass starts at 2 × pages + 1: fire on its first page.
            if done == 2 * 3 + 1 {
                cancel.store(true, Ordering::SeqCst);
            }
        }),
    );

    assert!(
        matches!(outcome, Err(ExecOutput::Stopped)),
        "the cancelled extract maps to the Stopped path"
    );
    assert_eq!(
        units.last().copied(),
        Some((2 * 3 + 1, 3 * 3)),
        "the pass stops at the page boundary where the flag fired: {units:?}"
    );
}

/// Cancel mid-attachment stops the read between pages exactly as before the
/// units existed: the preview reports `cancelled`, keeps nothing of the
/// abandoned attachment, and the unit stream stops where the flag fired.
#[test]
fn preview_cancel_stops_mid_attachment_between_pages() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "STOP00001", "P obra cancelada");
    let pdf = make_text_pdf_pages(&[
        &[(72.0, 700.0, CLEAN)],
        &[(72.0, 700.0, "hi")],
        &[(72.0, 700.0, "yo")],
    ]);
    let path = write_pdf(&dir, "stop.pdf", &pdf);
    let attachment_id = seed_attachment(&mut conn, &item_id, "STOPATT1", Some(&path), "stop.pdf");

    let cancel = AtomicBool::new(false);
    let mut progress = Vec::new();
    let preview = run_reprocess_preview(
        &conn,
        std::slice::from_ref(&attachment_id),
        &cancel,
        |update| {
            // Fire once the first page lands: the stop hits mid-file.
            if update.units_done >= 1 {
                cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            progress.push(update);
        },
    )
    .expect("preview");

    assert!(preview.cancelled, "the cancel flag stops the run");
    assert!(
        preview.attachments.is_empty(),
        "the abandoned attachment reports no plan"
    );
    assert!(
        progress.iter().any(|update| update.units_done >= 1),
        "the stop happened mid-read: {progress:?}"
    );
    assert!(
        progress.iter().all(|update| update.units_done < 3 * 3),
        "the read never finished its units: {progress:?}"
    );
}

/// Cerrar during the whole-document extract cancels the preview run: the
/// dialog's stop must reach the slow pass too, not only the per-page ones.
#[test]
fn preview_cancel_stops_inside_the_document_extract() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "STOPXTR1", "Q obra cancelada en extracto");
    let pdf = make_text_pdf_pages(&[
        &[(72.0, 700.0, CLEAN)],
        &[(72.0, 700.0, "hi")],
        &[(72.0, 700.0, "yo")],
    ]);
    let path = write_pdf(&dir, "stop-extract.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "STOPXATT1",
        Some(&path),
        "stop-extract.pdf",
    );

    let cancel = AtomicBool::new(false);
    let mut progress = Vec::new();
    let preview = run_reprocess_preview(
        &conn,
        std::slice::from_ref(&attachment_id),
        &cancel,
        |update| {
            // Past the per-page passes (2 × pages): the extract's first page
            // fires there.
            if update.units_done > 2 * 3 {
                cancel.store(true, Ordering::SeqCst);
            }
            progress.push(update);
        },
    )
    .expect("preview");

    assert!(preview.cancelled, "the flag reaches the extract pass");
    assert!(
        preview.attachments.is_empty(),
        "the abandoned attachment reports no plan"
    );
    assert!(
        progress.iter().any(|update| update.units_done > 2 * 3),
        "the stop happened inside the extract: {progress:?}"
    );
    assert!(
        progress.iter().all(|update| update.units_done < 3 * 3),
        "the extract never finished its pages: {progress:?}"
    );
}

// ── Measurements on a read-only database copy ──────────────────────────────

fn measure_db_path() -> Option<std::path::PathBuf> {
    match std::env::var("ENTROPIA_MEASURE_DB") {
        Ok(path) if !path.trim().is_empty() => Some(std::path::PathBuf::from(path)),
        _ => None,
    }
}

fn open_read_only_copy() -> Option<rusqlite::Connection> {
    let path = measure_db_path()?;
    match rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => Some(conn),
        Err(error) => panic!(
            "could not open the read-only copy {}: {error}",
            path.display()
        ),
    }
}

/// Where the candidate scan spends its time (attachments join vs page
/// conversion vs detector passes), on the real copy. The B3 scan is one
/// detector pass over every stored page; this probe keeps that cost visible
/// and separates it from the SQL work.
#[test]
#[ignore = "scan-cost measurement; run with ENTROPIA_MEASURE_DB set"]
fn measure_reprocess_scan_breakdown() {
    use entropia_desktop_lib::bibliography::processing::{
        garbled_bibliography_flags, is_garbled_text, ocr_markup_to_text,
        BIBLIOGRAPHY_DETECTOR_VERSION,
    };
    let Some(conn) = open_read_only_copy() else {
        eprintln!("ENTROPIA_MEASURE_DB unset; skipping");
        return;
    };
    let started = std::time::Instant::now();
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.item_id, a.content_type, a.filename, a.mtime, COALESCE(i.title, ''),
                    e.quality
             FROM zotero_attachments a
             JOIN bibliographic_items i ON i.id = a.item_id
             LEFT JOIN bibliographic_extractions e ON e.attachment_id = a.id
             ORDER BY COALESCE(i.title, ''), a.id",
        )
        .expect("attachments query");
    let rows: Vec<(String, Option<String>, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .expect("attachments rows")
        .filter_map(Result::ok)
        .collect();
    eprintln!(
        "attachments join: {} rows in {:.1} ms",
        rows.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    let pdf: Vec<&String> = rows
        .iter()
        .filter(|(_, content_type, filename)| {
            entropia_desktop_lib::bibliography::processing::is_pdf_attachment(
                content_type.as_deref(),
                filename.as_deref(),
            )
        })
        .map(|(id, _, _)| id)
        .collect();
    eprintln!("pdf attachments: {}", pdf.len());
    let started = std::time::Instant::now();
    let mut pages_total = 0usize;
    let mut convert_ms = 0f64;
    let mut detect_ms = 0f64;
    for attachment_id in &pdf {
        let mut pages = conn
            .prepare(
                "SELECT quality, text_content FROM bibliographic_page_texts
                 WHERE attachment_id = ?1 ORDER BY page_number",
            )
            .expect("pages query");
        let pages: Vec<(String, String)> = pages
            .query_map([attachment_id], |page| {
                Ok((page.get::<_, String>(0)?, page.get::<_, String>(1)?))
            })
            .expect("pages rows")
            .filter_map(Result::ok)
            .collect();
        for (_quality, text_content) in pages {
            pages_total += 1;
            let mark = std::time::Instant::now();
            let converted = ocr_markup_to_text(&text_content);
            convert_ms += mark.elapsed().as_secs_f64() * 1000.0;
            let mark = std::time::Instant::now();
            let _flags = garbled_bibliography_flags(&converted);
            let _plain = is_garbled_text(&converted);
            detect_ms += mark.elapsed().as_secs_f64() * 1000.0;
        }
    }
    eprintln!(
        "page scan: {} pages in {:.1} ms total; convert {:.1} ms, detect {:.1} ms (detector v{})",
        pages_total,
        started.elapsed().as_secs_f64() * 1000.0,
        convert_ms,
        detect_ms,
        BIBLIOGRAPHY_DETECTOR_VERSION
    );
}

/// Candidate scan over the real copy: how many attachments, in how many
/// milliseconds. Plan 2.5 expects the list to open fast (~82 attachments).
#[test]
#[ignore = "measurement on a read-only database copy; run with ENTROPIA_MEASURE_DB set"]
fn measure_reprocess_candidates_on_db_copy() {
    let Some(conn) = open_read_only_copy() else {
        eprintln!("ENTROPIA_MEASURE_DB unset; skipping");
        return;
    };
    let started = std::time::Instant::now();
    let candidates = reprocess_candidates(&conn).expect("candidates");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut reasons: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for candidate in &candidates {
        for reason in &candidate.reasons {
            *reasons.entry(reason.as_str()).or_insert(0) += 1;
        }
    }
    eprintln!(
        "candidates: {} attachments in {:.1} ms; reasons: {:?}",
        candidates.len(),
        elapsed_ms,
        reasons
    );
    for candidate in candidates.iter().take(5) {
        eprintln!(
            "  {} | {} | flagged={} | {:?}",
            candidate.title, candidate.attachment_id, candidate.flagged_pages, candidate.reasons
        );
    }
}

/// Preview over the first five real candidates: milliseconds per
/// attachment, per-attachment plan numbers and the totals including the
/// estimated USD. Read-only: the copy is never written.
#[test]
#[ignore = "measurement on a read-only database copy; run with ENTROPIA_MEASURE_DB set"]
fn measure_reprocess_preview_on_db_copy() {
    let Some(conn) = open_read_only_copy() else {
        eprintln!("ENTROPIA_MEASURE_DB unset; skipping");
        return;
    };
    let ids: Vec<String> = reprocess_candidates(&conn)
        .expect("candidates")
        .into_iter()
        .take(5)
        .map(|candidate| candidate.attachment_id)
        .collect();
    if ids.is_empty() {
        eprintln!("no candidates on this copy; skipping");
        return;
    }
    let cancel = AtomicBool::new(false);
    let started = std::time::Instant::now();
    let preview = run_reprocess_preview(&conn, &ids, &cancel, |update| {
        eprintln!(
            "progress: {}/{} (units {}/{})",
            update.done, update.total, update.units_done, update.units_total
        );
    })
    .expect("preview");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let per_attachment = elapsed_ms / preview.attachments.len().max(1) as f64;
    for entry in &preview.attachments {
        eprintln!(
            "  {} | pages={} ocr={} reused={} fixed={} busy={} unreadable={:?}",
            entry.title,
            entry.page_count,
            entry.ocr_pages,
            entry.reused_ocr_pages,
            entry.fixed_without_ocr,
            entry.busy,
            entry.unreadable
        );
    }
    eprintln!(
        "preview: {} attachments in {:.1} ms ({:.1} ms/attachment); totals: pages={} ocrPages={} reusedOcrPages={} fixedWithoutOcr={} estimatedUsd={:.6}",
        preview.attachments.len(),
        elapsed_ms,
        per_attachment,
        preview.totals.pages,
        preview.totals.ocr_pages,
        preview.totals.reused_ocr_pages,
        preview.totals.fixed_without_ocr,
        preview.totals.estimated_usd
    );
}

// ═══ B4: the confirm and the executor's reprocess mode (plan 2.3) ══════════
//
// The owner-approved plan becomes durable work: one user batch plus one
// `bibliography_extract` task per entry, pinned to the reprocess contract
// `prefix + <plan hash>`. The executor recomputes the plan from the current
// bytes and spends EXACTLY its `ocr_pages` — never a window, never a page the
// stored OCR rows already answered.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use entropia_desktop_lib::bibliography::attachment::attachment_ref_for;
use entropia_desktop_lib::bibliography::processing::BibliographyExtractExecutor;
use entropia_desktop_lib::bibliography::reprocess::{
    confirm_reprocess, insert_reprocess_extract_task, plan_reprocess_for_attachment, InsertOutcome,
    ReprocessConfirmEntry, ReprocessEntryStatus,
};
use entropia_desktop_lib::bibliography::selective_ocr::{PageOcrProvider, PageRenderer};
use entropia_desktop_lib::processing::scheduler::{
    run_one, ExecCtx, ExecOutput, Executor, ExecutorRegistry, RunOneOutcome, StopFlag,
};
use sha2::{Digest, Sha256};

fn ctx_of(dir: &tempfile::TempDir) -> ExecCtx {
    ExecCtx {
        db_path: dir.path().join("entropia.sqlite"),
    }
}

/// The internal row id of the seeded library, for the sync admission calls.
fn library_row_id(conn: &rusqlite::Connection) -> String {
    conn.query_row("SELECT id FROM zotero_libraries LIMIT 1", [], |row| {
        row.get(0)
    })
    .expect("library row")
}

fn attachment(
    conn: &rusqlite::Connection,
    attachment_id: &str,
) -> entropia_desktop_lib::bibliography::attachment::AttachmentRef {
    attachment_ref_for(conn, attachment_id)
        .expect("attachment lookup")
        .expect("attachment row exists")
}

fn entry(attachment_id: &str, plan_hash: &str) -> ReprocessConfirmEntry {
    ReprocessConfirmEntry {
        attachment_id: attachment_id.to_string(),
        plan_hash: plan_hash.to_string(),
    }
}

/// A well-formed but synthetic plan hash: 64 lowercase hex characters.
fn fake_plan_hash(seed: &str) -> String {
    seed.repeat(64 / seed.len())
}

fn queued_reprocess_task(conn: &rusqlite::Connection, attachment_id: &str) -> String {
    conn.query_row(
        "SELECT id FROM processing_tasks
          WHERE kind = 'bibliography_extract' AND subject_id = ?1
            AND contract_hash LIKE 'bibliography-extract-reprocess-v1|%'",
        [attachment_id],
        |row| row.get(0),
    )
    .expect("queued reprocess task")
}

fn task_state(conn: &rusqlite::Connection, task_id: &str) -> (String, Option<String>) {
    conn.query_row(
        "SELECT state, last_error_code FROM processing_tasks WHERE id = ?1",
        [task_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("task state")
}

fn checkpoint_rows(conn: &rusqlite::Connection, task_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id = ?1",
        [task_id],
        |row| row.get(0),
    )
    .expect("checkpoint count")
}

fn receipt_of(conn: &rusqlite::Connection, task_id: &str) -> serde_json::Value {
    let receipt: String = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks WHERE id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .expect("receipt read");
    serde_json::from_str(&receipt).expect("receipt JSON")
}

/// One live `bibliography_extract` task row linked to a running user batch —
/// exactly the queue shape the confirm produces — pinned to `contract` (the
/// FULL contract string, e.g. `reprocess_contract_hash(plan_hash)`). The
/// executor tests plant through it so they observe the executor, not the
/// confirm.
fn plant_linked_extract_task(
    conn: &rusqlite::Connection,
    task_id: &str,
    attachment_id: &str,
    contract: &str,
) -> String {
    conn.execute(
        "INSERT OR IGNORE INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, priority, created_at, updated_at)
         VALUES ('b4-batch', 'bibliography-reprocess-test', 'user', 'running', 'run', '[\"ocr\"]', 1, 2, 1, 1)",
        [],
    )
    .expect("plant batch");
    conn.execute(
        "INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
            input_revision, input_fingerprint, contract_hash, state, created_at, updated_at)
         VALUES (?1, 'bibliography_extract', ?2, 'bibliography', 'attachment', ?2,
            0, ?3, ?4, 'pending', 1, 1)",
        rusqlite::params![
            task_id,
            attachment_id,
            format!("attachment|{attachment_id}|mtime:{TEST_MTIME}|version:3"),
            contract
        ],
    )
    .expect("plant extract task");
    conn.execute(
        "INSERT INTO processing_batch_tasks
           (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind, subject_id, request_state)
         VALUES ('b4-batch', ?1, 'bibliography_extract', ?2, 'bibliography', 'attachment', ?2, 'active')",
        rusqlite::params![task_id, attachment_id],
    )
    .expect("link planted task");
    task_id.to_string()
}

/// One terminal `bibliography_extract` row: the spent-attempt history a
/// candidate reason (c) and the B1 rule read.
fn plant_terminal_extract_task(
    conn: &rusqlite::Connection,
    task_id: &str,
    attachment_id: &str,
    state: &str,
    code: Option<&str>,
) {
    conn.execute(
        "INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
            input_revision, input_fingerprint, contract_hash, state, last_error_code, created_at, updated_at)
         VALUES (?1, 'bibliography_extract', ?2, 'bibliography', 'attachment', ?2,
            0, ?3, 'bibliography-extract-v1', ?4, ?5, 1, 1)",
        rusqlite::params![
            task_id,
            attachment_id,
            format!("attachment|{attachment_id}|mtime:{TEST_MTIME}|version:3"),
            state,
            code
        ],
    )
    .expect("plant terminal extract task");
}

/// A page renderer that records which pages it rendered (1-based).
#[derive(Default)]
struct RecordingRenderer {
    pages: Mutex<Vec<u32>>,
}

impl PageRenderer for RecordingRenderer {
    fn render_page(&self, _pdf_bytes: &[u8], page_number: u32) -> Result<Vec<u8>, String> {
        self.pages.lock().expect("pages").push(page_number);
        Ok(vec![9, 9])
    }

    fn name(&self) -> &'static str {
        "recording-renderer"
    }
}

/// A provider answering from a script (a default text once the script runs
/// out), logging every per-page call and every whole-PDF window request, and
/// optionally tripping a stop flag after N calls — the interrupt seam.
struct RecordingProvider {
    answers: Mutex<VecDeque<Result<String, String>>>,
    fallback: String,
    calls: Mutex<usize>,
    pdf_ranges: Mutex<Vec<(u32, u32)>>,
    pdf_per_request: Option<usize>,
    stop_after: Option<(usize, Arc<StopFlag>)>,
}

impl RecordingProvider {
    fn with_text(text: &str) -> Arc<Self> {
        Self::with_options(text, [], None, None)
    }

    fn with_options(
        text: &str,
        answers: impl IntoIterator<Item = Result<String, String>>,
        pdf_per_request: Option<usize>,
        stop_after: Option<(usize, Arc<StopFlag>)>,
    ) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().collect()),
            fallback: text.to_string(),
            calls: Mutex::new(0),
            pdf_ranges: Mutex::new(Vec::new()),
            pdf_per_request,
            stop_after,
        })
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("calls")
    }
}

impl PageOcrProvider for RecordingProvider {
    fn recognize_page(&self, _image_bytes: &[u8]) -> Result<String, String> {
        let index = {
            let mut calls = self.calls.lock().expect("calls");
            *calls += 1;
            *calls
        };
        if let Some((after, flag)) = &self.stop_after {
            if index == *after {
                flag.stop();
            }
        }
        match self.answers.lock().expect("answers").pop_front() {
            Some(answer) => answer,
            None => Ok(self.fallback.clone()),
        }
    }

    fn pdf_pages_per_request(&self) -> Option<usize> {
        self.pdf_per_request
    }

    fn recognize_pdf_pages(
        &self,
        _pdf_bytes: &[u8],
        first_page: u32,
        last_page: u32,
    ) -> Result<Vec<String>, String> {
        self.pdf_ranges
            .lock()
            .expect("ranges")
            .push((first_page, last_page));
        Ok((first_page..=last_page)
            .map(|page| format!("Contenido reconocido de la pagina {page}"))
            .collect())
    }

    fn name(&self) -> &str {
        "recording-ocr"
    }
}

fn run_one_extract(
    dir: &tempfile::TempDir,
    conn: &rusqlite::Connection,
    renderer: &Arc<RecordingRenderer>,
    provider: &Arc<RecordingProvider>,
) -> RunOneOutcome {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(BibliographyExtractExecutor::with_selective_ocr(
        renderer.clone() as Arc<dyn PageRenderer>,
        provider.clone() as Arc<dyn PageOcrProvider>,
    )));
    run_one(
        conn,
        &ctx_of(dir),
        &registry,
        "b4-session",
        processing_repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run")
}

/// The long, rich text every OCR answer needs to grade `rich`.
const OCR_TEXT: &str =
    "Texto reconocido por OCR con longitud suficiente para que la pagina resulte rica y publicable";

/// 2.3 "las páginas que la vista previa informa son exactamente las que el
/// ejecutor manda, y las de OCR guardado se descuentan": the previewed plan's
/// `ocr_pages` is EXACTLY what reaches the provider, and a stored OCR row of
/// the matching source is reused — never re-sent.
#[test]
fn the_previewed_ocr_page_set_is_exactly_what_the_executor_sends() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4SET0001", "M obra exacta");
    let pdf = make_text_pdf_pages(&[
        &[(72.0, 700.0, CLEAN)],
        &[(72.0, 700.0, "hi")],
        &[(72.0, 700.0, "yo")],
        &[(72.0, 700.0, "no")],
    ]);
    let path = write_pdf(&dir, "exacto.pdf", &pdf);
    let attachment_id =
        seed_attachment(&mut conn, &item_id, "B4SETATT1", Some(&path), "exacto.pdf");
    // The stored extraction matches the source, so its stored OCR row of
    // page 3 is reused and never charged again.
    plant_extraction(
        &conn,
        &attachment_id,
        &item_id,
        "sparse",
        Some(TEST_MTIME),
        pdf.len() as i64,
    );
    plant_page(
        &conn,
        &attachment_id,
        3,
        "ocr",
        "rich",
        "read by OCR before",
    );

    let preview = run_reprocess_preview(
        &conn,
        std::slice::from_ref(&attachment_id),
        &AtomicBool::new(false),
        |_| {},
    )
    .expect("preview");
    let shown = &preview.attachments[0];
    assert!(shown.unreadable.is_none(), "{:?}", shown.unreadable);
    assert_eq!(shown.ocr_pages, 2, "two sparse pages go out");
    assert_eq!(
        shown.reused_ocr_pages, 1,
        "the stored OCR row is discounted"
    );
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    assert_eq!(plan.ocr_pages, vec![2, 4], "exactly the sparse pages");
    assert_eq!(
        shown.plan_hash.as_deref(),
        Some(plan.plan_hash.as_str()),
        "the preview and the planner agree on the hash"
    );

    plant_linked_extract_task(
        &conn,
        "b4-set-task",
        &attachment_id,
        &processing_repository::reprocess_contract_hash(&plan.plan_hash),
    );
    let renderer = Arc::new(RecordingRenderer::default());
    let provider = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        renderer.pages.lock().expect("pages").as_slice(),
        &[2, 4],
        "the provider saw exactly the previewed page set"
    );
    assert_eq!(provider.calls(), 2);
    // The reused page publishes the stored OCR text, not the native row.
    let (method, text): (String, String) = conn
        .query_row(
            "SELECT method, text_content FROM bibliographic_page_texts
              WHERE attachment_id = ?1 AND page_number = 3",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("page 3 row");
    assert_eq!(method, "ocr");
    assert!(
        text.contains("read by OCR before"),
        "the stored OCR answer is reused as-is: {text}"
    );
}

/// 2.3 "el reuso convierte el HTML a Markdown": a reused stored OCR row (GLM
/// answers tables as HTML) publishes the CONVERTED Markdown with method
/// `ocr` and a fresh hash of the converted text.
#[test]
fn a_reused_ocr_row_publishes_the_stored_text_converted_to_markdown() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4HTML001", "N obra con tabla");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]);
    let path = write_pdf(&dir, "tabla.pdf", &pdf);
    let attachment_id = seed_attachment(&mut conn, &item_id, "B4HTMLATT", Some(&path), "tabla.pdf");
    plant_extraction(
        &conn,
        &attachment_id,
        &item_id,
        "rich",
        Some(TEST_MTIME),
        pdf.len() as i64,
    );
    plant_page(
        &conn,
        &attachment_id,
        1,
        "ocr",
        "rich",
        "<table><tr><td>Alpha cell</td><td>Beta cell</td></tr></table>",
    );

    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    assert_eq!(plan.reused_ocr_pages.len(), 1);
    assert!(plan.ocr_pages.is_empty(), "the reused page is never sent");

    plant_linked_extract_task(
        &conn,
        "b4-html-task",
        &attachment_id,
        &processing_repository::reprocess_contract_hash(&plan.plan_hash),
    );
    let renderer = Arc::new(RecordingRenderer::default());
    let provider = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(provider.calls(), 0, "reused text costs no recognition");

    let (method, text, text_hash): (String, String, String) = conn
        .query_row(
            "SELECT method, text_content, text_hash FROM bibliographic_page_texts
              WHERE attachment_id = ?1 AND page_number = 1",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("page row");
    assert_eq!(method, "ocr");
    assert!(
        text.contains("Alpha cell") && text.contains("Beta cell"),
        "{text}"
    );
    assert!(
        !text.contains("<td") && !text.contains("<table"),
        "the stored HTML is converted, never published raw: {text}"
    );
    assert!(
        text.contains('|'),
        "the converted table is Markdown pipe syntax: {text}"
    );
    assert_eq!(
        text_hash,
        format!("{:x}", Sha256::digest(text.as_bytes())),
        "a fresh hash of the CONVERTED text"
    );
}

/// 2.3 "OCR solo por página… salta la rama de ventanas": even when every page
/// needs OCR and the provider offers whole-PDF windows, the reprocess mode
/// renders and recognizes page by page — no `recognize_pdf_pages` call.
#[test]
fn reprocess_mode_never_asks_for_whole_pdf_windows() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4WIN0001", "O obra escaneada");
    let blank: &[(f32, f32, &str)] = &[];
    let pdf = make_text_pdf_pages(&[blank, blank, blank]);
    let path = write_pdf(&dir, "escaneado.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "B4WINATT1",
        Some(&path),
        "escaneado.pdf",
    );
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    assert_eq!(
        plan.ocr_pages,
        vec![1, 2, 3],
        "every blank page needs OCR — well over the window threshold"
    );

    plant_linked_extract_task(
        &conn,
        "b4-win-task",
        &attachment_id,
        &processing_repository::reprocess_contract_hash(&plan.plan_hash),
    );
    let renderer = Arc::new(RecordingRenderer::default());
    let provider = RecordingProvider::with_options(OCR_TEXT, [], Some(100), None);
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert!(
        provider.pdf_ranges.lock().expect("ranges").is_empty(),
        "the reprocess mode is page-only: {:?}",
        provider.pdf_ranges.lock().expect("ranges")
    );
    assert_eq!(
        renderer.pages.lock().expect("pages").as_slice(),
        &[1, 2, 3],
        "each plan page is rendered and recognized on its own"
    );
    assert_eq!(provider.calls(), 3);
}

/// JD7-A-003 (2.3): a PDF replaced by another of the SAME SIZE is a different
/// file — the recomputed plan hash misses the approved one and the task dies
/// `reprocess_authorization_stale` with ZERO provider calls.
#[test]
fn a_pdf_replaced_by_one_of_the_same_size_fails_stale_with_zero_provider_calls() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4STALE01", "P obra reemplazada");
    let approved =
        make_text_pdf_pages(&[&[(72.0, 700.0, "aaaa bbbb cccc dddd eeee ffff gggg hhhh")]]);
    let replaced =
        make_text_pdf_pages(&[&[(72.0, 700.0, "zzzz yyyy xxxx wwww vvvv uuuu tttt ssss")]]);
    assert_eq!(
        approved.len(),
        replaced.len(),
        "the replacement must be byte-identical in size"
    );
    assert_ne!(approved, replaced, "but hold different text");
    let path = write_pdf(&dir, "reemplazado.pdf", &approved);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "B4STALATT",
        Some(&path),
        "reemplazado.pdf",
    );
    let attachment = attachment(&conn, &attachment_id);
    let plan =
        plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &approved)
            .expect("plan");
    // The owner approved `plan`; the file is swapped behind their back.
    std::fs::write(&path, &replaced).expect("replace the PDF");
    plant_linked_extract_task(
        &conn,
        "b4-stale-task",
        &attachment_id,
        &processing_repository::reprocess_contract_hash(&plan.plan_hash),
    );

    let renderer = Arc::new(RecordingRenderer::default());
    let provider = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert!(
        matches!(outcome, RunOneOutcome::Failed { .. }),
        "{outcome:?}"
    );
    let (state, code) = task_state(&conn, "b4-stale-task");
    assert_eq!(state, "failed");
    assert_eq!(code.as_deref(), Some("reprocess_authorization_stale"));
    assert_eq!(provider.calls(), 0, "no provider call before the gate");
    assert!(
        renderer.pages.lock().expect("pages").is_empty(),
        "not even a render"
    );
    let extractions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("extraction count");
    assert_eq!(extractions, 0, "nothing is published");
}

/// 2.3 "si hay una tarea viva para el adjunto, la marca `busy` y sigue" and
/// "Un lote sin ninguna entrada encolada se borra dentro de la misma
/// transacción": a confirm over a live task answers `busy`, keeps the batch
/// out of the world, and changes nothing.
#[test]
fn confirm_with_a_live_task_answers_busy_and_changes_nothing() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4BUSY001", "Q obra ocupada");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]);
    let path = write_pdf(&dir, "ocupada.pdf", &pdf);
    let attachment_id =
        seed_attachment(&mut conn, &item_id, "B4BUSYATT", Some(&path), "ocupada.pdf");
    let live = admit_extract_task(&conn, &attachment_id);
    let before_tasks: i64 = conn
        .query_row("SELECT COUNT(*) FROM processing_tasks", [], |row| {
            row.get(0)
        })
        .expect("task count");
    let before_batches: i64 = conn
        .query_row("SELECT COUNT(*) FROM processing_batches", [], |row| {
            row.get(0)
        })
        .expect("batch count");

    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &fake_plan_hash("ab"))]).expect("confirm");

    assert_eq!(response.batch_id, None, "an empty batch is deleted");
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].status, ReprocessEntryStatus::Busy);
    let after_tasks: i64 = conn
        .query_row("SELECT COUNT(*) FROM processing_tasks", [], |row| {
            row.get(0)
        })
        .expect("task count");
    let after_batches: i64 = conn
        .query_row("SELECT COUNT(*) FROM processing_batches", [], |row| {
            row.get(0)
        })
        .expect("batch count");
    assert_eq!(after_tasks, before_tasks, "no task was minted");
    assert_eq!(after_batches, before_batches, "no batch was left behind");
    let links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_batch_tasks WHERE task_id = ?1",
            [&live],
            |row| row.get(0),
        )
        .expect("link count");
    assert_eq!(links, 1, "the live task keeps only its original link");
    let (contract, state): (String, String) = conn
        .query_row(
            "SELECT contract_hash, state FROM processing_tasks WHERE id = ?1",
            [&live],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("live task row");
    assert_eq!(contract, "bibliography-extract-v1");
    assert_eq!(state, "pending");
}

/// JD7-B-001 (2.3): a live task landing between the confirm's check and its
/// plain INSERT collides with the partial unique — the INSERT answers `busy`
/// and the racing task is NEVER linked to anything.
#[test]
fn a_live_task_racing_the_insert_answers_busy_and_is_never_linked() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4RACE001", "R obra en carrera");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]);
    let path = write_pdf(&dir, "carrera.pdf", &pdf);
    let attachment_id =
        seed_attachment(&mut conn, &item_id, "B4RACEATT", Some(&path), "carrera.pdf");
    // The row another admitter plants BETWEEN the check and the INSERT.
    let live = admit_extract_task(&conn, &attachment_id);
    let attachment = attachment(&conn, &attachment_id);

    let outcome = insert_reprocess_extract_task(&conn, &attachment, &fake_plan_hash("cd"))
        .expect("the INSERT seam answers, never panics");

    assert_eq!(
        outcome,
        InsertOutcome::Busy,
        "the UNIQUE collision is one spent busy answer"
    );
    let links: i64 = conn
        .query_row("SELECT COUNT(*) FROM processing_batch_tasks", [], |row| {
            row.get(0)
        })
        .expect("link count");
    assert_eq!(
        links, 1,
        "only the original admitter's link exists: the racing INSERT never links"
    );
    let tasks: i64 = conn
        .query_row("SELECT COUNT(*) FROM processing_tasks", [], |row| {
            row.get(0)
        })
        .expect("task count");
    assert_eq!(tasks, 1, "only the live task exists");
    let state: String = conn
        .query_row(
            "SELECT state FROM processing_tasks WHERE id = ?1",
            [&live],
            |row| row.get(0),
        )
        .expect("live task");
    assert_eq!(state, "pending");
}

/// 2.3 "Las tres comparaciones de contrato": the reprocess contract claims
/// and commits like the automatic one (a full run publishes), while an
/// unknown contract parks `configuration_changed` exactly as before.
#[test]
fn the_reprocess_contract_passes_claim_and_commit_and_an_unknown_one_blocks() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4CTR0001", "S obra con contrato");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, "hi")]]);
    let path = write_pdf(&dir, "contrato.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "B4CTRATT1",
        Some(&path),
        "contrato.pdf",
    );
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");

    let response = confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)])
        .expect("confirm queues the approved plan");
    assert!(response.batch_id.is_some());
    assert_eq!(response.results[0].status, ReprocessEntryStatus::Queued);
    let task_id = queued_reprocess_task(&conn, &attachment_id);

    // Claim accepts the reprocess contract and the run publishes through the
    // commit gate: success end-to-end means both gates passed.
    let renderer = Arc::new(RecordingRenderer::default());
    let provider = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert!(
        matches!(&outcome, RunOneOutcome::Succeeded { task_id: done } if done == &task_id),
        "{outcome:?}"
    );
    assert_eq!(provider.calls(), 1, "exactly the plan's page was charged");

    // An unparsable contract blocks instead of running.
    plant_linked_extract_task(
        &conn,
        "b4-foreign-task",
        &attachment_id,
        "bibliography-extract-v2|x",
    );
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert_eq!(
        outcome,
        RunOneOutcome::Idle,
        "a foreign contract is never claimed"
    );
    let (state, code) = task_state(&conn, "b4-foreign-task");
    assert_eq!(state, "blocked");
    assert_eq!(code.as_deref(), Some("configuration_changed"));
}

/// 2.3 "Lote visible y cancelable": the confirm's batch is a user batch — it
/// lists, it cancels through `processing_control`'s core, its task becomes a
/// spent cancelled attempt, and the sync never re-queues it (B1).
#[test]
fn the_reprocess_batch_lists_as_a_user_batch_and_cancels_through_processing_control() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4BATCH01", "T obra con lote");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, "hi")]]);
    let path = write_pdf(&dir, "lote.pdf", &pdf);
    let attachment_id = seed_attachment(&mut conn, &item_id, "B4BATCHAT", Some(&path), "lote.pdf");

    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &fake_plan_hash("12"))]).expect("confirm");
    let batch_id = response.batch_id.expect("a queued entry mints a batch");
    let task_id = queued_reprocess_task(&conn, &attachment_id);

    let (listed, _) =
        processing_repository::list_batches(&conn, None, None, 50).expect("batch listing");
    assert!(
        listed.iter().any(|batch| batch.id == batch_id),
        "the reprocess batch appears in the batch tab's listing"
    );
    let snapshot =
        processing_repository::read_batch_snapshot(&conn, &batch_id).expect("batch snapshot");
    assert_eq!(snapshot.origin, "user", "only user batches list");
    assert_eq!(snapshot.priority, 2, "interactive work");
    assert_eq!(snapshot.operations, vec!["ocr".to_string()]);
    assert!(snapshot.planning_done, "no planning phase is needed");

    // Cancel through the same core `processing_control` drives.
    processing_repository::control_batch(
        &conn,
        &batch_id,
        processing_repository::BatchAction::Cancel,
        None,
    )
    .expect("cancel");
    let (task, _) = task_state(&conn, &task_id);
    assert_eq!(task, "cancelled", "the cancel reaches the queued task");
    let batch_state: String = conn
        .query_row(
            "SELECT state FROM processing_batches WHERE id = ?1",
            [&batch_id],
            |row| row.get(0),
        )
        .expect("batch state");
    assert_eq!(batch_state, "cancelled", "the batch finalizes alone");

    // B1: a cancelled attempt is spent — the sync must not re-queue it.
    let created =
        processing_repository::admit_stale_extraction_demands(&conn, &library_row_id(&conn))
            .expect("sync admission");
    assert_eq!(
        created, 0,
        "a cancelled reprocess is never silently repeated"
    );
}

/// 2.3 "Garantía de costo": across an interrupt, a terminal transient failure
/// and `processing_retry`, the TOTAL provider calls stay at `plan.ocr_pages`
/// — every settled page checkpoints, checkpoints survive the terminal
/// failure, and the retry pays only what is missing.
#[test]
fn the_cost_guarantee_holds_across_an_interrupt_a_failure_and_processing_retry() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4COST001", "U obra con costo");
    let sparse: &[(f32, f32, &str)] = &[(72.0, 700.0, "hi")];
    let pdf = make_text_pdf_pages(&[sparse, sparse, sparse]);
    let path = write_pdf(&dir, "costo.pdf", &pdf);
    let attachment_id = seed_attachment(&mut conn, &item_id, "B4COSTATT", Some(&path), "costo.pdf");
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    assert_eq!(plan.ocr_pages, vec![1, 2, 3]);

    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)]).expect("confirm");
    let batch_id = response.batch_id.expect("queued");
    let task_id = queued_reprocess_task(&conn, &attachment_id);

    // Attempt 1 pays page 1, checkpoints it, then the run is interrupted.
    let task = processing_repository::claim_next(
        &conn,
        "b4-session",
        &["bibliography_extract"],
        processing_repository::now_ms(),
    )
    .expect("claim")
    .expect("claimable");
    assert_eq!(task.task_id, task_id);
    let stop = Arc::new(StopFlag::new());
    let first = RecordingProvider::with_options(OCR_TEXT, [], None, Some((1, Arc::clone(&stop))));
    let result = BibliographyExtractExecutor::with_selective_ocr(
        Arc::new(RecordingRenderer::default()) as Arc<dyn PageRenderer>,
        Arc::clone(&first) as Arc<dyn PageOcrProvider>,
    )
    .run(&ctx_of(&dir), &task, &stop);
    assert!(
        matches!(result.output, ExecOutput::Stopped),
        "{:?}",
        result.output
    );
    assert_eq!(first.calls(), 1, "one paid page before the interrupt");
    assert_eq!(checkpoint_rows(&conn, &task_id), 1);

    // The queue closes the attempt with a transient verdict, terminally.
    let outcome = processing_repository::fail_attempt(
        &conn,
        &task_id,
        task.lease_epoch,
        task.attempt_number,
        "provider_transient",
        "request timed out after 30s",
        false,
        None,
        processing_repository::now_ms(),
    )
    .expect("fail");
    assert!(matches!(
        outcome,
        processing_repository::FailOutcome::Failed
    ));
    assert_eq!(
        checkpoint_rows(&conn, &task_id),
        1,
        "the paid page survives the terminal failure"
    );

    // `processing_retry` reopens the same task and the retry pays only the
    // missing pages: 1 + 2 = 3 = plan.ocr_pages, never more.
    assert_eq!(
        processing_repository::retry_failed(&conn, &batch_id, Some(&task_id)).expect("retry"),
        1
    );
    let renderer = Arc::new(RecordingRenderer::default());
    let second = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &second);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        second.calls(),
        2,
        "only the unpaid pages reach the provider"
    );
    assert_eq!(
        first.calls() + second.calls(),
        plan.ocr_pages.len() as usize,
        "the plan's page count is the whole cost of the reprocess"
    );
}

/// JD8-B-002 (2.3 "Garantía de costo"): the cumulative provider sends for
/// ONE approved plan never exceed `plan.ocr_pages`, even across a cancel and
/// a re-confirm. The paid page checkpoint survives the cancel (a cancelled
/// extraction keeps its checkpoints like a terminal failure) and the fresh
/// task of the SAME plan adopts it before the OCR pass.
#[test]
fn cancelling_and_reconfirming_the_same_plan_never_resends_the_paid_page() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4CXL0001", "X obra cancelada");
    let sparse: &[(f32, f32, &str)] = &[(72.0, 700.0, "hi")];
    let pdf = make_text_pdf_pages(&[sparse, sparse]);
    let path = write_pdf(&dir, "cancelada.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "B4CXLATT1",
        Some(&path),
        "cancelada.pdf",
    );
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    assert_eq!(plan.ocr_pages, vec![1, 2]);

    // Confirm, pay page 1, interrupt the run.
    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)]).expect("confirm");
    let batch_id = response.batch_id.expect("queued");
    let first_task = queued_reprocess_task(&conn, &attachment_id);
    let task = processing_repository::claim_next(
        &conn,
        "b4-session",
        &["bibliography_extract"],
        processing_repository::now_ms(),
    )
    .expect("claim")
    .expect("claimable");
    assert_eq!(task.task_id, first_task);
    let stop = Arc::new(StopFlag::new());
    let first = RecordingProvider::with_options(OCR_TEXT, [], None, Some((1, Arc::clone(&stop))));
    let result = BibliographyExtractExecutor::with_selective_ocr(
        Arc::new(RecordingRenderer::default()) as Arc<dyn PageRenderer>,
        Arc::clone(&first) as Arc<dyn PageOcrProvider>,
    )
    .run(&ctx_of(&dir), &task, &stop);
    assert!(
        matches!(result.output, ExecOutput::Stopped),
        "{:?}",
        result.output
    );
    assert_eq!(first.calls(), 1, "one paid page before the interrupt");
    assert_eq!(checkpoint_rows(&conn, &first_task), 1);
    processing_repository::requeue_task(&conn, &task.task_id, task.lease_epoch).expect("requeue");

    // The owner cancels the batch; the paid page survives the cancel.
    processing_repository::control_batch(
        &conn,
        &batch_id,
        processing_repository::BatchAction::Cancel,
        None,
    )
    .expect("cancel");
    assert_eq!(task_state(&conn, &first_task).0, "cancelled");
    assert_eq!(
        checkpoint_rows(&conn, &first_task),
        1,
        "the paid page survives the cancel"
    );

    // Re-confirming the SAME plan creates a fresh task that adopts the paid
    // page: only page 2 reaches the provider.
    let response = confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)])
        .expect("confirm the same plan again");
    assert!(response.batch_id.is_some());
    let second_task: String = conn
        .query_row(
            "SELECT id FROM processing_tasks
              WHERE kind = 'bibliography_extract' AND subject_id = ?1
                AND contract_hash LIKE 'bibliography-extract-reprocess-v1|%'
                AND state != 'cancelled'
              ORDER BY created_at DESC, rowid DESC LIMIT 1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("second reprocess task");
    assert_ne!(second_task, first_task);
    let renderer = Arc::new(RecordingRenderer::default());
    let second = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &second);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(second.calls(), 1, "the adopted checkpoint is never re-paid");
    assert_eq!(
        first.calls() + second.calls(),
        plan.ocr_pages.len() as usize,
        "one approved plan costs at most its page count, even across a cancel"
    );
}

/// JD8-B-002 round 2 (2.3 "Garantía de costo"): the STARTUP checkpoint GC
/// (`run_checkpoint_cleanup`, what every app restart runs first) must not
/// undo the paid-page retention. The page the owner paid before cancelling
/// a plan survives the restart, so re-confirming the SAME plan after it
/// never re-sends the page and the plan still costs at most
/// `plan.ocr_pages` in total.
#[test]
fn the_restart_checkpoint_gc_keeps_the_paid_pages_of_a_cancelled_plan() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4GC00001", "Z obra con reinicio");
    let sparse: &[(f32, f32, &str)] = &[(72.0, 700.0, "hi")];
    let pdf = make_text_pdf_pages(&[sparse, sparse]);
    let path = write_pdf(&dir, "reinicio.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "B4GCATT01",
        Some(&path),
        "reinicio.pdf",
    );
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    assert_eq!(plan.ocr_pages, vec![1, 2]);

    // Confirm, pay page 1, interrupt the run.
    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)]).expect("confirm");
    let batch_id = response.batch_id.expect("queued");
    let first_task = queued_reprocess_task(&conn, &attachment_id);
    let task = processing_repository::claim_next(
        &conn,
        "b4-gc-session",
        &["bibliography_extract"],
        processing_repository::now_ms(),
    )
    .expect("claim")
    .expect("claimable");
    assert_eq!(task.task_id, first_task);
    let stop = Arc::new(StopFlag::new());
    let first = RecordingProvider::with_options(OCR_TEXT, [], None, Some((1, Arc::clone(&stop))));
    let result = BibliographyExtractExecutor::with_selective_ocr(
        Arc::new(RecordingRenderer::default()) as Arc<dyn PageRenderer>,
        Arc::clone(&first) as Arc<dyn PageOcrProvider>,
    )
    .run(&ctx_of(&dir), &task, &stop);
    assert!(
        matches!(result.output, ExecOutput::Stopped),
        "{:?}",
        result.output
    );
    assert_eq!(first.calls(), 1, "one paid page before the interrupt");
    assert_eq!(checkpoint_rows(&conn, &first_task), 1);
    processing_repository::requeue_task(&conn, &task.task_id, task.lease_epoch).expect("requeue");

    // The owner cancels the batch; the paid page survives the cancel.
    processing_repository::control_batch(
        &conn,
        &batch_id,
        processing_repository::BatchAction::Cancel,
        None,
    )
    .expect("cancel");
    assert_eq!(task_state(&conn, &first_task).0, "cancelled");

    // The restart: the archive's connections close and the startup
    // checkpoint GC runs over the file.
    drop(conn);
    let db_path = dir.path().join("entropia.sqlite");
    run_checkpoint_cleanup(&db_path).expect("restart checkpoint GC");
    let conn = entropia_desktop_lib::db_open_for_tests(&db_path);
    assert_eq!(
        checkpoint_rows(&conn, &first_task),
        1,
        "the paid page survives the restart GC"
    );

    // Re-confirming the SAME plan adopts the paid page: only page 2 reaches
    // the provider.
    let response = confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)])
        .expect("confirm the same plan again");
    assert!(response.batch_id.is_some());
    let renderer = Arc::new(RecordingRenderer::default());
    let second = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &second);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(second.calls(), 1, "the adopted checkpoint is never re-paid");
    assert!(
        first.calls() + second.calls() <= plan.ocr_pages.len(),
        "the plan costs at most its page count across a restart: {} + {} > {}",
        first.calls(),
        second.calls(),
        plan.ocr_pages.len()
    );
}

/// JD8-A-001 (2.3 "Garantía de costo"): a metadata-only catalog edit
/// (same file, same `mtime` and size, new `native_version`) during a
/// reprocess must NOT re-pin the extraction fingerprint and re-send the paid
/// pages. A reprocess task pins the file identity WITHOUT the catalog
/// version — the plan hash in the contract, which covers `sourceSha256`, is
/// the content binding — so the page-1 checkpoint survives the edit and the
/// resumed run recognizes only page 2.
#[test]
fn a_metadata_only_edit_during_a_reprocess_never_resends_the_paid_page() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4META001", "W obra con edicion");
    let sparse: &[(f32, f32, &str)] = &[(72.0, 700.0, "hi")];
    let pdf = make_text_pdf_pages(&[sparse, sparse]);
    let path = write_pdf(&dir, "edicion.pdf", &pdf);
    let attachment_id =
        seed_attachment(&mut conn, &item_id, "B4METAATT", Some(&path), "edicion.pdf");
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    assert_eq!(plan.ocr_pages, vec![1, 2]);
    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)]).expect("confirm");
    assert!(response.batch_id.is_some());
    let task_id = queued_reprocess_task(&conn, &attachment_id);

    // Attempt 1 pays page 1, checkpoints it, then the run is interrupted.
    let task = processing_repository::claim_next(
        &conn,
        "b4-session",
        &["bibliography_extract"],
        processing_repository::now_ms(),
    )
    .expect("claim")
    .expect("claimable");
    assert_eq!(task.task_id, task_id);
    let stop = Arc::new(StopFlag::new());
    let first = RecordingProvider::with_options(OCR_TEXT, [], None, Some((1, Arc::clone(&stop))));
    let result = BibliographyExtractExecutor::with_selective_ocr(
        Arc::new(RecordingRenderer::default()) as Arc<dyn PageRenderer>,
        Arc::clone(&first) as Arc<dyn PageOcrProvider>,
    )
    .run(&ctx_of(&dir), &task, &stop);
    assert!(
        matches!(result.output, ExecOutput::Stopped),
        "{:?}",
        result.output
    );
    assert_eq!(first.calls(), 1, "one paid page before the interrupt");
    assert_eq!(checkpoint_rows(&conn, &task_id), 1);
    processing_repository::requeue_task(&conn, &task.task_id, task.lease_epoch).expect("requeue");

    // A metadata-only edit: the same file, a new catalog version.
    conn.execute(
        "UPDATE zotero_attachments SET native_version = 4 WHERE id = ?1",
        [&attachment_id],
    )
    .expect("metadata edit");

    // The resume recognizes only page 2: page 1 comes from its checkpoint.
    let renderer = Arc::new(RecordingRenderer::default());
    let second = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &second);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        second.calls(),
        1,
        "the paid page is never re-sent after a metadata-only edit"
    );
    assert_eq!(
        first.calls() + second.calls(),
        plan.ocr_pages.len() as usize,
        "the plan's page count is the whole cost of the reprocess"
    );
}

/// JD8-A-003: the automatic sync admission must NOT attach to a live
/// reprocess task. A system-batch link keeps the task wanted after the owner
/// pauses or cancels the reprocess batch, and paid OCR continues behind their
/// back: the attachment is SKIPPED instead. Cancelling the user batch then
/// cancels the task and the executor never calls the provider.
#[test]
fn automatic_admission_skips_a_live_reprocess_task_and_the_cancel_stops_it() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4AUTO001", "V obra compartida");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, "hi")]]);
    let path = write_pdf(&dir, "compartida.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "B4AUTOATT",
        Some(&path),
        "compartida.pdf",
    );
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)]).expect("confirm");
    let task_id = queued_reprocess_task(&conn, &attachment_id);
    let batch_id = response.batch_id.expect("a queued entry mints a batch");

    // The sync's own admission finds the live reprocess task and skips the
    // attachment: no demand is counted and NO link is created.
    let created =
        processing_repository::admit_stale_extraction_demands(&conn, &library_row_id(&conn))
            .expect("sync admission");
    assert_eq!(
        created, 0,
        "nothing is admitted while an owner-approved reprocess owns the attachment"
    );
    let tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_tasks WHERE subject_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("task count");
    assert_eq!(tasks, 1, "still one physical task");
    let links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_batch_tasks WHERE task_id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("link count");
    assert_eq!(
        links, 1,
        "the reprocess task keeps only its user-batch link"
    );

    // Cancelling the reprocess batch reaches the task — nothing else wants
    // it — and the executor never spends anything.
    processing_repository::control_batch(
        &conn,
        &batch_id,
        processing_repository::BatchAction::Cancel,
        None,
    )
    .expect("cancel");
    assert_eq!(
        task_state(&conn, &task_id).0,
        "cancelled",
        "the cancel reaches the task the moment no batch wants it"
    );
    let renderer = Arc::new(RecordingRenderer::default());
    let provider = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert_eq!(
        outcome,
        RunOneOutcome::Idle,
        "nothing is left to run: {outcome:?}"
    );
    assert_eq!(provider.calls(), 0, "zero provider calls");
}

/// 2.3: the executor honors its reprocess contract — the receipt carries the
/// plan hash, the detector version and the source identity 2.4 compares.
#[test]
fn the_reprocess_receipt_carries_the_plan_and_the_source_identity() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4RECEIPT", "W obra con recibo");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, "hi")]]);
    let path = write_pdf(&dir, "recibo.pdf", &pdf);
    let attachment_id =
        seed_attachment(&mut conn, &item_id, "B4RECATT1", Some(&path), "recibo.pdf");
    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)]).expect("confirm");
    let task_id = queued_reprocess_task(&conn, &attachment_id);
    assert!(response.batch_id.is_some());

    // The task runs under its pinned REPROCESS contract.
    let renderer = Arc::new(RecordingRenderer::default());
    let provider = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    let receipt = receipt_of(&conn, &task_id);
    assert_eq!(
        receipt["reprocess"]["planHash"],
        serde_json::json!(plan.plan_hash),
        "the executor followed exactly the approved plan: {receipt}"
    );
    assert!(receipt["reprocess"]["detectorVersion"].is_number());
    assert_eq!(receipt["sourceMtime"], serde_json::json!(TEST_MTIME));
    assert_eq!(receipt["sourceBytes"], serde_json::json!(pdf.len() as i64));
    assert!(receipt["sourceSha256"].is_string());
}

/// 2.4 end-to-end: after a successful reprocess the attachment leaves the
/// candidate list — even though its spent-attempt reason outlives the repair,
/// so ONLY the reprocess receipt can remove it.
#[test]
fn a_successful_reprocess_removes_the_attachment_from_the_candidates() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4ONCE001", "Z obra reparada");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]);
    let path = write_pdf(&dir, "reparada.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "B4ONCEATT",
        Some(&path),
        "reparada.pdf",
    );
    plant_extraction(
        &conn,
        &attachment_id,
        &item_id,
        "rich",
        Some(TEST_MTIME),
        pdf.len() as i64,
    );
    // A spent failed OCR attempt on the current file: reason (c), which no
    // repair can erase — the 2.4 receipt is the only exclusion left.
    plant_terminal_extract_task(
        &conn,
        "b4-spent-attempt",
        &attachment_id,
        "failed",
        Some("ocr_failed"),
    );
    let candidates = reprocess_candidates(&conn).expect("candidates");
    assert_eq!(
        candidate_for(&candidates, &attachment_id)
            .expect("the spent attempt lists the attachment")
            .reasons,
        vec![REASON_FAILED_OCR_ATTEMPT]
    );

    let attachment = attachment(&conn, &attachment_id);
    let plan = plan_reprocess_for_attachment(&conn, &attachment, std::path::Path::new(&path), &pdf)
        .expect("plan");
    let response =
        confirm_reprocess(&conn, &[entry(&attachment_id, &plan.plan_hash)]).expect("confirm");
    let task_id = queued_reprocess_task(&conn, &attachment_id);
    assert!(response.batch_id.is_some());

    let renderer = Arc::new(RecordingRenderer::default());
    let provider = RecordingProvider::with_text(OCR_TEXT);
    let outcome = run_one_extract(&dir, &conn, &renderer, &provider);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    let receipt = receipt_of(&conn, &task_id);
    assert_eq!(
        receipt["reprocess"]["planHash"],
        serde_json::json!(plan.plan_hash)
    );
    assert_eq!(receipt["sourceMtime"], serde_json::json!(TEST_MTIME));
    assert_eq!(receipt["sourceBytes"], serde_json::json!(pdf.len() as i64));

    let candidates = reprocess_candidates(&conn).expect("candidates after repair");
    assert!(
        candidate_for(&candidates, &attachment_id).is_none(),
        "one successful reprocess of this file is enough: {candidates:?}"
    );
}

/// The confirm refuses a plan hash that is not the approved shape (64
/// lowercase hex characters) before touching the queue.
#[test]
fn confirm_rejects_a_malformed_plan_hash() {
    let (dir, mut conn) = migrated_db();
    let item_id = seed_item(&mut conn, "B4HASH001", "AA obra con hash");
    let pdf = make_text_pdf_pages(&[&[(72.0, 700.0, CLEAN)]]);
    let path = write_pdf(&dir, "hash.pdf", &pdf);
    let attachment_id = seed_attachment(&mut conn, &item_id, "B4HASHATT", Some(&path), "hash.pdf");

    for bad in ["", "XYZ", &"a".repeat(63), &"a".repeat(65), &"A".repeat(64)] {
        let error = confirm_reprocess(&conn, &[entry(&attachment_id, bad)])
            .expect_err("a malformed plan hash is refused");
        assert!(
            error.contains("invalid_selection"),
            "{bad:?}: unexpected error {error}"
        );
    }
    let batches: i64 = conn
        .query_row("SELECT COUNT(*) FROM processing_batches", [], |row| {
            row.get(0)
        })
        .expect("batch count");
    assert_eq!(batches, 0, "a refused confirm touches nothing");
}
