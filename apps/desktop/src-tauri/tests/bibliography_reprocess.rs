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

use std::sync::atomic::AtomicBool;

use entropia_desktop_lib::bibliography::repository::{
    upsert_attachment, upsert_connection, upsert_item, upsert_library, AttachmentInput,
    BibliographicItemInput, ExtractionRow, LibraryType, PageTextRow, SourceOrigin,
    UpsertConnection, UpsertLibrary,
};
use entropia_desktop_lib::bibliography::reprocess::{
    estimated_usd, reprocess_candidates, run_reprocess_preview, ReprocessCandidate,
    REASON_EMPTY_WITHOUT_OCR, REASON_FAILED_OCR_ATTEMPT, REASON_GARBLED_STORED_PAGES,
    UNREADABLE_FILE_MISSING,
};
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
        })
    };

    let task = admit_extract_task(&conn, &attachment_id);
    settle_task(&conn, &task, "succeeded", None);
    plant_receipt(&conn, &task, receipt(1, 4242));
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
        |done, total| progress.push((done, total)),
    )
    .expect("preview");

    assert_eq!(
        progress,
        vec![(1, 1)],
        "progress fires after each attachment"
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
    let preview = run_reprocess_preview(
        &conn,
        std::slice::from_ref(&attachment_id),
        &cancel,
        |_, _| {},
    )
    .expect("preview");
    assert!(preview.attachments[0].busy, "a pending task is a live task");
    assert!(preview.attachments[0].unreadable.is_none());

    settle_task(&conn, &task_id, "succeeded", None);
    let preview = run_reprocess_preview(
        &conn,
        std::slice::from_ref(&attachment_id),
        &cancel,
        |_, _| {},
    )
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
    let preview = run_reprocess_preview(
        &conn,
        std::slice::from_ref(&attachment_id),
        &cancel,
        |_, _| {},
    )
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
    let preview = run_reprocess_preview(&conn, &ids, &cancel, |done, total| {
        eprintln!("progress: {done}/{total}");
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
