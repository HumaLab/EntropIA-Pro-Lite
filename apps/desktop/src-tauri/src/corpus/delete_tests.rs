//! Behavior of the corpus delete service over an archive built by the real
//! migrations and real files in temporary folders.
//!
//! The service replaces the app's two renderer deletes (`item.repo.ts`
//! `deleteWithCascade` and `collection.repo.ts` `delete`), so these tests pin
//! every observable effect of that move: rows from every table the store
//! touched (directly or through a foreign-key cascade), the `D` tombstones the
//! sync triggers record, the files removed after the commit, the files that
//! must survive, and the impact counts the future approval dialog reads.

use std::path::Path;

use rusqlite::{params, Connection};

use super::delete::{delete_collection, delete_impact, delete_items, DeleteScope};
use crate::automation::test_archive::{item_id, seeded_archive, ALFA, BETA};

struct Fixture {
    _root: tempfile::TempDir,
    data_dir: std::path::PathBuf,
    cache_dir: std::path::PathBuf,
    conn: Connection,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");
    let cache_dir = root.path().join("cache");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(cache_dir.join("thumbnails")).unwrap();
    let conn = crate::db::open::open_archive_connection(&seeded_archive(&data_dir)).unwrap();
    // The delete service counts sync tombstones and leaves the OCR backup rows
    // the store leaves; both tables are created on demand in the app.
    crate::sync::capture::ensure_capture(&conn).unwrap();
    crate::llm::ocr_correction::ensure_schema(&conn).unwrap();
    Fixture {
        _root: root,
        data_dir: std::fs::canonicalize(&data_dir).unwrap(),
        cache_dir: std::fs::canonicalize(&cache_dir).unwrap(),
        conn,
    }
}

impl Fixture {
    fn count(&self, table: &str) -> i64 {
        self.conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn count_where(&self, sql: &str, params: impl rusqlite::Params) -> i64 {
        self.conn.query_row(sql, params, |row| row.get(0)).unwrap()
    }

    fn fts_rows_for_item(&self, item_id: &str) -> i64 {
        self.count_where(
            "SELECT count(*) FROM fts_items WHERE rowid = (SELECT rowid FROM items WHERE id = ?1)",
            [item_id],
        )
    }

    fn d_rows(&self, table: &str, row_id: &str) -> i64 {
        self.count_where(
            "SELECT count(*) FROM sync_oplog
             WHERE table_name = ?1 AND row_id = ?2 AND op = 'D'",
            params![table, row_id],
        )
    }

    fn item_folder(&self, collection: &str, item: &str) -> std::path::PathBuf {
        self.data_dir.join("assets").join(collection).join(item)
    }

    fn collection_folder(&self, collection: &str) -> std::path::PathBuf {
        self.data_dir.join("assets").join(collection)
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Every table that hangs off one item, with one distinguishable row each.
    fn seed_item_children(&self, item: &str, asset: &str) {
        self.conn
            .execute(
                "INSERT INTO extractions (id, asset_id, text_content, method, created_at)
                 VALUES (?1, ?2, 'texto reconocido', 'ocr', 1)",
                params![format!("ext-{asset}"), asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO layouts (id, asset_id, regions, blocks, model, image_width, image_height, created_at)
                 VALUES (?1, ?2, '[]', '[]', 'model', 100, 100, 1)",
                params![format!("lay-{asset}"), asset],
            )
            .unwrap();
        for (id, target_type, target_id) in [
            (format!("llm-asset-{asset}"), "asset", asset),
            (format!("llm-item-{item}"), "item", item),
        ] {
            self.conn
                .execute(
                    "INSERT INTO llm_results (id, target_id, target_type, job_type, result, created_at)
                     VALUES (?1, ?2, ?3, 'summary', '{}', 1)",
                    params![id, target_id, target_type],
                )
                .unwrap();
        }
        self.conn
            .execute(
                "INSERT INTO notes (id, item_id, asset_id, content, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'nota', 1, 1)",
                params![format!("note-{item}"), item, asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO entities (id, item_id, asset_id, entity_type, value, created_at)
                 VALUES (?1, ?2, ?3, 'person', 'Ana', 1)",
                params![format!("ent-{item}"), item, asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO triples (id, item_id, asset_id, subject, predicate, object, created_at)
                 VALUES (?1, ?2, ?3, 'Ana', 'escribe', 'carta', 1)",
                params![format!("tri-{item}"), item, asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO annotations (id, asset_id, page, kind, color, x, y, width, height, created_at, updated_at)
                 VALUES (?1, ?2, 1, 'rectangle', '#f00', 0, 0, 1, 1, 1, 1)",
                params![format!("ann-{asset}"), asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO transcriptions (id, asset_id, text_content, model, created_at)
                 VALUES (?1, ?2, 'transcripcion', 'model', 1)",
                params![format!("trx-{asset}"), asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO topics (id, name, created_at) VALUES ('top-1', 'historia', 1)",
                [],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO item_topics (id, item_id, topic_id, created_at)
                 VALUES (?1, ?2, 'top-1', 1)",
                params![format!("it-{item}"), item],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO rag_chunks (id, asset_id, item_id, source_kind, source_id,
                    chunk_ordinal, text_content, start_char, end_char, source_text_hash,
                    chunking_contract, embedding, embedding_model, embedding_contract, dimensions)
                 VALUES (?1, ?2, ?3, 'extraction', ?4, 0, 'texto', 0, 5, 'hash',
                    'chunk', X'00', 'model', 'contract', 1)",
                params![
                    format!("chunk-{asset}"),
                    asset,
                    item,
                    format!("ext-{asset}")
                ],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO processing_asset_revisions (asset_id, source_revision)
                 VALUES (?1, 1)",
                [asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO ocr_correction_backups (asset_id, original_text_content, created_at)
                 VALUES (?1, 'original', 1)",
                [asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO processing_tasks
                    (id, kind, asset_id_snapshot, state, created_at, updated_at)
                 VALUES (?1, 'ocr', ?2, 'succeeded', 1, 1)",
                params![format!("task-{item}"), asset],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO extraction_schemas (id, name, fields_json, created_at, updated_at)
                 VALUES ('schema-1', 'Ficha', '[]', 1, 1)",
                [],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO extraction_records (id, schema_id, asset_id, item_id, record_json, created_at)
                 VALUES ('record-1', 'schema-1', ?1, ?2, '{}', 1)",
                params![asset, item],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO vec_assets (asset_id, item_id, embedding) VALUES (?1, ?2, X'00')",
                params![asset, item],
            )
            .unwrap();
    }

    fn seed_two_assets(&self) {
        self.seed_item_children(&item_id(ALFA, 1), "ast_alfa_1");
        self.conn
            .execute(
                "INSERT INTO vec_assets (asset_id, item_id, embedding) VALUES ('ast_alfa_2', ?1, X'00')",
                [item_id(ALFA, 1)],
            )
            .unwrap();
    }

    fn enable_capture(&self) {
        crate::sync::capture::ensure_capture(&self.conn).unwrap();
        crate::sync::test_support::set_session_with_capture(&self.conn);
    }
}

#[test]
fn impact_counts_documents_pages_notes_annotations_and_recognized_text() {
    let f = fixture();
    f.seed_item_children(&item_id(ALFA, 1), "ast_alfa_1");
    f.conn
        .execute(
            "INSERT INTO notes (id, item_id, content, created_at, updated_at)
             VALUES ('note-beta', ?1, 'fuera del alcance', 1, 1)",
            [item_id(BETA, 1)],
        )
        .unwrap();

    let one = delete_impact(&f.conn, &DeleteScope::Items(vec![item_id(ALFA, 1)])).unwrap();
    assert_eq!(one.documents, 1);
    assert_eq!(one.pages, 2, "both seeded assets count as pages");
    assert_eq!(one.notes, 1);
    assert_eq!(one.annotations, 1);
    assert_eq!(one.documents_with_text, 1);

    let beta = delete_impact(&f.conn, &DeleteScope::Collection(BETA.to_string())).unwrap();
    assert_eq!(beta.documents, 2);
    assert_eq!(beta.pages, 0);
    assert_eq!(beta.notes, 1);
    assert_eq!(beta.annotations, 0);
    assert_eq!(beta.documents_with_text, 0);

    let missing = delete_impact(
        &f.conn,
        &DeleteScope::Items(vec![item_id(ALFA, 1), "no-such-item".to_string()]),
    )
    .unwrap_err();
    assert_eq!(missing.to_string(), "not_found: no item no-such-item");

    let missing = delete_impact(
        &f.conn,
        &DeleteScope::Collection("no-such-collection".to_string()),
    )
    .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "not_found: no collection no-such-collection"
    );
}

#[test]
fn deleting_items_removes_every_row_the_store_removes() {
    let f = fixture();
    f.seed_two_assets();
    assert_eq!(f.fts_rows_for_item(&item_id(ALFA, 1)), 1);

    delete_items(&f.conn, &f.data_dir, &f.cache_dir, &[item_id(ALFA, 1)]).unwrap();

    for table in [
        "assets",
        "extractions",
        "layouts",
        "llm_results",
        "notes",
        "entities",
        "triples",
        "annotations",
        "transcriptions",
        "item_topics",
        "rag_chunks",
        "processing_asset_revisions",
        "extraction_records",
        "vec_assets",
    ] {
        assert_eq!(f.count(table), 0, "{table} still holds rows");
    }
    assert_eq!(
        f.count_where(
            "SELECT count(*) FROM items WHERE id = ?1",
            [item_id(ALFA, 1)]
        ),
        0
    );
    assert_eq!(f.count("items"), 6, "the other documents stay");
    assert_eq!(f.fts_rows_for_item(&item_id(ALFA, 1)), 0);
    assert_eq!(f.fts_rows_for_item(&item_id(ALFA, 2)), 1);
    assert_eq!(
        f.count_where("SELECT count(*) FROM collections WHERE id = ?1", [ALFA]),
        1
    );
    // The store leaves these alone today; so does the service.
    assert_eq!(f.count("ocr_correction_backups"), 1);
    assert_eq!(f.count("processing_tasks"), 1);
    assert_eq!(f.count("topics"), 1, "the topic itself is not an item row");
}

#[test]
fn an_asset_stored_inside_assets_but_outside_its_item_folder_is_removed() {
    let f = fixture();
    let item = item_id(ALFA, 1);
    f.conn
        .execute(
            "UPDATE assets SET path = 'assets/col/legacy.pdf', type = 'pdf' WHERE id = 'ast_alfa_1'",
            [],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE assets SET path = 'assets/photo.jpg', type = 'image' WHERE id = 'ast_alfa_2'",
            [],
        )
        .unwrap();
    let pdf = f.data_dir.join("assets/col/legacy.pdf");
    let pages = f.data_dir.join("assets/col/legacy.pages/0001.png");
    let photo = f.data_dir.join("assets/photo.jpg");
    let photo_v2 = f.data_dir.join("assets/photo_v2.png");
    let photo_final = f.data_dir.join("assets/photo_final.jpg");
    for path in [&pdf, &pages, &photo, &photo_v2, &photo_final] {
        f.write_file(path, b"x");
    }

    delete_items(
        &f.conn,
        &f.data_dir,
        &f.cache_dir,
        std::slice::from_ref(&item),
    )
    .unwrap();

    assert!(!pdf.exists(), "a PDF outside the item folder is removed");
    assert!(!pages.exists(), "its generated pages directory goes too");
    assert!(
        !photo.exists(),
        "an image outside the item folder is removed"
    );
    assert!(!photo_v2.exists(), "the image's edited sibling goes too");
    assert!(
        photo_final.exists(),
        "a different image family is not touched"
    );
}

#[test]
fn a_path_a_surviving_asset_still_references_is_kept() {
    let f = fixture();
    let item = item_id(ALFA, 1);
    let shared = "assets/shared/photo.jpg";
    f.conn
        .execute(
            "UPDATE assets SET path = ?1, type = 'image' WHERE id = 'ast_alfa_1'",
            [shared],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO assets (id, item_id, path, type, sort_index, size, created_at)
             VALUES ('ast_alfa_keep', ?1, ?2, 'image', 0, 1, 1)",
            params![item_id(ALFA, 2), shared],
        )
        .unwrap();
    let file = f.data_dir.join(shared);
    f.write_file(&file, b"x");

    delete_items(
        &f.conn,
        &f.data_dir,
        &f.cache_dir,
        std::slice::from_ref(&item),
    )
    .unwrap();

    assert!(
        file.exists(),
        "a surviving asset row still references this path"
    );
    assert_eq!(
        f.count_where("SELECT count(*) FROM assets WHERE id = 'ast_alfa_keep'", []),
        1
    );
}

#[test]
fn stored_paths_outside_assets_are_kept() {
    let f = fixture();
    let item = item_id(ALFA, 1);
    let captured = f.data_dir.join("web-captures/x.pdf");
    let external = f._root.path().join("external.pdf");
    f.write_file(&captured, b"%PDF");
    f.write_file(&captured.with_extension("pages").join("0001.png"), b"png");
    f.write_file(&external, b"%PDF");
    f.conn
        .execute(
            "UPDATE assets SET path = ?1, type = 'pdf' WHERE id = 'ast_alfa_1'",
            [captured.to_string_lossy().to_string()],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE assets SET path = ?1, type = 'pdf' WHERE id = 'ast_alfa_2'",
            [external.to_string_lossy().to_string()],
        )
        .unwrap();

    delete_items(
        &f.conn,
        &f.data_dir,
        &f.cache_dir,
        std::slice::from_ref(&item),
    )
    .unwrap();

    assert!(
        captured.exists(),
        "a capture inside the data dir but outside assets is kept"
    );
    assert!(
        captured.with_extension("pages").exists(),
        "its pages directory is outside assets, so it stays too"
    );
    assert!(external.exists(), "a file outside the data dir is kept");
}

#[test]
fn a_dot_dot_item_id_cannot_remove_the_assets_root() {
    let f = fixture();
    f.conn
        .execute(
            "INSERT INTO collections (id, name, created_at, updated_at)
             VALUES ('col-dotdot', 'DotDot', 1, 1), ('col-dot', 'Dot item collection', 1, 1), ('.', 'Dot', 1, 1)",
            [],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO items (id, title, collection_id, created_at, updated_at)
             VALUES ('..', 'DotDot item', 'col-dotdot', 1, 1), ('.', 'Dot item', 'col-dot', 1, 1), ('item-dot', 'Dot collection item', '.', 1, 1)",
            [],
        )
        .unwrap();
    let root_marker = f.data_dir.join("assets/root-marker.txt");
    let collection_marker = f.data_dir.join("assets/col-dotdot/keep.txt");
    let dot_collection_marker = f.data_dir.join("assets/col-dot/keep.txt");
    f.write_file(&root_marker, b"x");
    f.write_file(&collection_marker, b"x");
    f.write_file(&dot_collection_marker, b"x");

    delete_items(&f.conn, &f.data_dir, &f.cache_dir, &["..".to_string()]).unwrap();
    assert!(root_marker.exists(), "a '..' item id cannot remove assets/");
    assert!(
        collection_marker.exists(),
        "the non-empty collection folder stays"
    );

    delete_items(&f.conn, &f.data_dir, &f.cache_dir, &[".".to_string()]).unwrap();
    assert!(root_marker.exists(), "a '.' item id cannot remove assets/");
    assert!(
        dot_collection_marker.exists(),
        "its collection folder stays"
    );

    delete_collection(&f.conn, &f.data_dir, &f.cache_dir, ".").unwrap();
    assert!(
        root_marker.exists(),
        "a '.' collection id cannot remove assets/"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_item_folder_does_not_delete_outside() {
    let f = fixture();
    let item = item_id(ALFA, 1);
    let outside = f._root.path().join("outside");
    let keep = outside.join("keep.txt");
    f.write_file(&keep, b"x");
    let collection_dir = f.collection_folder(ALFA);
    std::fs::create_dir_all(&collection_dir).unwrap();
    std::os::unix::fs::symlink(&outside, collection_dir.join(&item)).unwrap();

    delete_items(
        &f.conn,
        &f.data_dir,
        &f.cache_dir,
        std::slice::from_ref(&item),
    )
    .unwrap();

    assert!(
        keep.exists(),
        "a symlinked item folder never deletes its target"
    );
}

#[test]
fn deleting_the_last_items_removes_the_collection_they_emptied() {
    let f = fixture();
    f.write_file(&f.item_folder(BETA, &item_id(BETA, 1)).join("a.txt"), b"a");
    f.write_file(&f.item_folder(BETA, &item_id(BETA, 2)).join("b.txt"), b"b");

    delete_items(
        &f.conn,
        &f.data_dir,
        &f.cache_dir,
        &[item_id(BETA, 1), item_id(BETA, 2)],
    )
    .unwrap();

    assert_eq!(
        f.count_where("SELECT count(*) FROM collections WHERE id = ?1", [BETA]),
        0
    );
    assert!(!f.collection_folder(BETA).exists());
    assert_eq!(
        f.count_where("SELECT count(*) FROM collections WHERE id = ?1", [ALFA]),
        1,
        "another collection keeps its row"
    );
}

#[test]
fn deleting_items_removes_their_folders_and_thumbnails_but_leaves_outside_files() {
    let f = fixture();
    let item = item_id(ALFA, 1);
    f.write_file(&f.item_folder(ALFA, &item).join("uuid_acta.pdf"), b"%PDF");
    f.write_file(
        &f.item_folder(ALFA, &item_id(ALFA, 2)).join("otra.pdf"),
        b"%PDF",
    );
    let external = f._root.path().join("external.pdf");
    f.write_file(&external, b"%PDF");
    f.conn
        .execute(
            "UPDATE assets SET path = ?1 WHERE id = 'ast_alfa_2'",
            [external.to_string_lossy().to_string()],
        )
        .unwrap();

    let thumbnails = f.cache_dir.join("thumbnails");
    f.write_file(&thumbnails.join("ast_alfa_1.png"), b"png");
    f.write_file(&thumbnails.join("image-ast_alfa_2-abcdef.png"), b"png");
    f.write_file(&thumbnails.join("image-other-abcdef.png"), b"png");
    f.write_file(&thumbnails.join("other.png"), b"png");

    delete_items(
        &f.conn,
        &f.data_dir,
        &f.cache_dir,
        std::slice::from_ref(&item),
    )
    .unwrap();

    assert!(!f.item_folder(ALFA, &item).exists());
    assert!(
        f.item_folder(ALFA, &item_id(ALFA, 2)).exists(),
        "another item keeps its folder"
    );
    assert!(
        f.collection_folder(ALFA).exists(),
        "the collection still holds another item"
    );
    assert!(
        external.exists(),
        "an asset outside the archive is never moved"
    );
    assert!(!thumbnails.join("ast_alfa_1.png").exists());
    assert!(!thumbnails.join("image-ast_alfa_2-abcdef.png").exists());
    assert!(thumbnails.join("image-other-abcdef.png").exists());
    assert!(thumbnails.join("other.png").exists());
}

#[test]
fn a_file_that_cannot_be_removed_does_not_fail_the_delete_or_restore_rows() {
    let f = fixture();
    let item = item_id(ALFA, 1);
    // A directory whose path is occupied by a file: remove_dir_all fails, and
    // the service must log it and leave the database deleted.
    let item_folder = f.item_folder(ALFA, &item);
    f.write_file(&item_folder, b"not a directory");

    delete_items(
        &f.conn,
        &f.data_dir,
        &f.cache_dir,
        std::slice::from_ref(&item),
    )
    .unwrap();

    assert_eq!(
        f.count_where("SELECT count(*) FROM items WHERE id = ?1", [&item]),
        0
    );
    assert!(item_folder.exists(), "the unremovable path is left behind");
}

#[test]
fn unknown_ids_are_not_found_and_nothing_changes() {
    let f = fixture();
    f.seed_two_assets();
    f.write_file(
        &f.item_folder(ALFA, &item_id(ALFA, 1)).join("a.pdf"),
        b"%PDF",
    );
    let before = (
        f.count("items"),
        f.count("assets"),
        f.count("collections"),
        f.count("notes"),
        f.count("sync_oplog"),
    );

    let error = delete_items(
        &f.conn,
        &f.data_dir,
        &f.cache_dir,
        &[item_id(ALFA, 1), "no-such-item".to_string()],
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "not_found: no item no-such-item");

    let error =
        delete_collection(&f.conn, &f.data_dir, &f.cache_dir, "no-such-collection").unwrap_err();
    assert_eq!(
        error.to_string(),
        "not_found: no collection no-such-collection"
    );

    assert_eq!(
        (
            f.count("items"),
            f.count("assets"),
            f.count("collections"),
            f.count("notes"),
            f.count("sync_oplog"),
        ),
        before
    );
    assert!(f.item_folder(ALFA, &item_id(ALFA, 1)).exists());
}

#[test]
fn deleting_a_collection_removes_its_rows_files_and_records_d_tombstones() {
    let f = fixture();
    let first = item_id(ALFA, 1);
    f.seed_two_assets();
    f.write_file(&f.item_folder(ALFA, &first).join("a.pdf"), b"%PDF");
    f.write_file(&f.collection_folder(ALFA).join("stray.txt"), b"x");
    let external = f._root.path().join("external.pdf");
    f.write_file(&external, b"%PDF");
    f.conn
        .execute(
            "UPDATE assets SET path = ?1 WHERE id = 'ast_alfa_2'",
            [external.to_string_lossy().to_string()],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO llm_results (id, target_id, target_type, job_type, result, created_at)
             VALUES ('llm-collection', ?1, 'collection', 'summary', '{}', 1)",
            [ALFA],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO writing_documents
                (id, title, document_type, schema_version, current_content_json, created_at, updated_at)
             VALUES ('doc-1', 'Obra', 'article', 1, '{}', 1, 1)",
            [],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO writing_document_collections (document_id, collection_id, is_primary, created_at)
             VALUES ('doc-1', ?1, 1, 1)",
            [ALFA],
        )
        .unwrap();
    f.enable_capture();

    delete_collection(&f.conn, &f.data_dir, &f.cache_dir, ALFA).unwrap();

    for table in [
        "assets",
        "extractions",
        "layouts",
        "llm_results",
        "notes",
        "entities",
        "triples",
        "annotations",
        "transcriptions",
        "item_topics",
        "rag_chunks",
        "processing_asset_revisions",
        "extraction_records",
        "vec_assets",
    ] {
        assert_eq!(f.count(table), 0, "{table} still holds rows");
    }
    assert_eq!(
        f.count_where(
            "SELECT count(*) FROM items WHERE collection_id = ?1",
            [ALFA]
        ),
        0
    );
    assert_eq!(
        f.count("items"),
        2,
        "the other collection keeps its documents"
    );
    assert_eq!(
        f.count_where("SELECT count(*) FROM collections WHERE id = ?1", [ALFA]),
        0
    );
    assert_eq!(
        f.count_where("SELECT count(*) FROM collections WHERE id = ?1", [BETA]),
        1
    );
    assert_eq!(
        f.count_where(
            "SELECT count(*) FROM writing_document_collections WHERE collection_id = ?1",
            [ALFA]
        ),
        0,
        "the document-collection link cascades"
    );
    assert_eq!(f.count("writing_documents"), 1, "the document itself stays");
    // Left as today: neither table points at the corpus by foreign key.
    assert_eq!(f.count("ocr_correction_backups"), 1);
    assert_eq!(f.count("processing_tasks"), 1);

    assert_eq!(f.d_rows("collections", ALFA), 1);
    assert_eq!(f.d_rows("items", &first), 1);
    assert_eq!(f.d_rows("assets", "ast_alfa_1"), 1);
    assert_eq!(f.d_rows("vec_assets", "ast_alfa_1"), 1);

    assert!(!f.collection_folder(ALFA).exists());
    assert!(
        external.exists(),
        "an asset outside the archive is never moved"
    );
}
