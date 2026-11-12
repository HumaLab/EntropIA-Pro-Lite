//! Behavior of the ingest service over an archive built by the real migrations
//! and real files in temporary folders.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::ingest::*;
use crate::automation::test_archive::{seeded_archive, ALFA, BETA};

struct Fixture {
    _root: tempfile::TempDir,
    data_dir: PathBuf,
    inbox: PathBuf,
    conn: Connection,
}

/// An archive whose data directory is `root/data`, and an enabled folder
/// `root/inbox` outside it.
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");
    let inbox = root.path().join("inbox");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&inbox).unwrap();
    let conn = crate::db::open::open_archive_connection(&seeded_archive(&data_dir)).unwrap();
    Fixture {
        data_dir: std::fs::canonicalize(&data_dir).unwrap(),
        inbox: crate::bibliography::attachment::plain_canonical(&inbox).unwrap(),
        _root: root,
        conn,
    }
}

impl Fixture {
    fn folders(&self) -> Vec<PathBuf> {
        vec![self.inbox.clone()]
    }

    fn write(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.inbox.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().to_string()
    }

    fn check(&self, path: &str) -> Result<SourceFile, String> {
        check_source(
            path,
            &self.folders(),
            &self.data_dir,
            &SourceLimits::default(),
        )
    }

    /// The app's own check: the owner chose the path, so no folder is needed.
    fn app_check(&self, path: &str) -> Result<SourceFile, String> {
        check_app_source(path, &self.data_dir, &SourceLimits::default())
    }

    /// A file written inside the data directory, such as one the app staged
    /// for the Internet Archive, the Navegador copy or the sample collection.
    fn write_in_data(&self, relative: &str, bytes: &[u8]) -> String {
        let path = self.data_dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().to_string()
    }

    fn items_in(&self, collection: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT count(*) FROM items WHERE collection_id = ?1",
                [collection],
                |row| row.get(0),
            )
            .unwrap()
    }
}

/// One asset row: id, path, sort index, parent, page number.
type AssetRow = (String, String, i64, Option<String>, Option<i64>);

/// A PDF with `count` pages of long, plain-language text: every page passes
/// the executor's `is_quality_text` rule, so S4 stores it as native text.
fn pdf_with_rich_pages(count: u32) -> Vec<u8> {
    use lopdf::{dictionary, Document, Object, Stream};
    let mut document = Document::with_version("1.5");
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources = document.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let pages_id = document.new_object_id();
    let kids: Vec<Object> = (0..count)
        .map(|index| {
            let line = format!(
                "El cabildo de Buenos Aires sesiono en asamblea general el dia veinticinco de mayo MARCADORPAGINA{index} fin del acta capitular"
            );
            let content = document.add_object(Stream::new(
                dictionary! {},
                format!("BT /F1 12 Tf 20 100 Td ({line}) Tj ET").into_bytes(),
            ));
            document
                .add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                    "Resources" => resources,
                    "Contents" => content,
                })
                .into()
        })
        .collect();
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Count" => count as i64, "Kids" => kids,
        }),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

/// A one-page PDF lopdf splits fine but the text parser cannot read: its font
/// uses a custom encoding with no ToUnicode map, so decoding the text layer
/// fails (a panic the parser contains into an error) while the page
/// structure stays intact.
fn pdf_with_undecodable_page() -> Vec<u8> {
    use lopdf::{dictionary, Document, Object, ObjectId, Stream};
    let mut document = Document::with_version("1.5");
    let differences: Vec<Object> = vec![42.into()];
    let font = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => dictionary! { "BaseEncoding" => "NoSuchEncoding", "Differences" => differences },
    });
    let resources = document.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let pages_id: ObjectId = document.new_object_id();
    let content = document.add_object(Stream::new(
        dictionary! {},
        b"BT /F1 12 Tf 20 100 Td <010203010203> Tj ET".to_vec(),
    ));
    let page: Object = document
        .add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Resources" => resources,
            "Contents" => content,
        })
        .into();
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Count" => 1_i64, "Kids" => vec![page],
        }),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

/// S4: a PDF whose text layer is usable stores that text at import, one
/// native extraction per page, exactly as the OCR executor would publish it.
#[test]
fn importing_a_pdf_with_a_usable_text_layer_stores_native_text_at_once() {
    let f = fixture();
    let path = f.write("Actas.pdf", &pdf_with_rich_pages(2));
    let source = f.check(&path).unwrap();
    let ImportOutcome::Imported {
        item_id, assets, ..
    } = import_file(
        &f.conn,
        &f.data_dir,
        BETA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap()
    else {
        panic!("expected an import");
    };
    assert_eq!(assets, 3, "the original plus two pages");

    let mut statement = f
        .conn
        .prepare(
            "SELECT id, path, page_number FROM assets
             WHERE item_id = ?1 AND parent_asset_id IS NOT NULL ORDER BY page_number",
        )
        .unwrap();
    let pages: Vec<(String, String, i64)> = statement
        .query_map([&item_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(pages.len(), 2);
    for (index, (asset_id, asset_path, page_number)) in pages.iter().enumerate() {
        assert_eq!(*page_number, index as i64 + 1);
        let (method, text, source_path, source_sha256): (
            String,
            String,
            Option<String>,
            Option<String>,
        ) = f
            .conn
            .query_row(
                "SELECT method, text_content, source_path, source_sha256
                 FROM extractions WHERE asset_id = ?1",
                [asset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(method, "native", "page {page_number}: stored as native");
        assert!(
            text.contains(&format!("MARCADORPAGINA{index}")),
            "page {page_number}: its own text, got {text:?}"
        );
        // The stamp is the page file the asset points to, as the executor's
        // native result stamps the file it read.
        assert_eq!(source_path.as_deref(), Some(asset_path.as_str()));
        let on_disk = crate::asset_history::sha256_file(&f.data_dir.join(asset_path)).unwrap();
        assert_eq!(source_sha256.as_deref(), Some(on_disk.as_str()));
        let (process, tool): (String, String) = f
            .conn
            .query_row(
                "SELECT process, tool FROM asset_process_events
                 WHERE asset_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [asset_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(process, "ocr");
        assert_eq!(tool, "pdfium (capa de texto del PDF)");
    }
    // The container keeps no extraction of its own.
    let parent: String = f
        .conn
        .query_row(
            "SELECT id FROM assets WHERE item_id = ?1 AND parent_asset_id IS NULL",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    let parent_extractions: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(*) FROM extractions WHERE asset_id = ?1",
            [&parent],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(parent_extractions, 0);
}

/// S4: a page without a usable layer (too little text to be quality) imports
/// exactly as before: no extraction, no failure.
#[test]
fn importing_a_pdf_without_a_usable_text_layer_stores_no_text() {
    let f = fixture();
    // "Pagina 0" is far below the quality bar, like a scanned page.
    let path = f.write("Corta.pdf", &pdf_with_pages(1));
    let source = f.check(&path).unwrap();
    let ImportOutcome::Imported {
        item_id, assets, ..
    } = import_file(
        &f.conn,
        &f.data_dir,
        BETA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap()
    else {
        panic!("expected an import");
    };
    assert_eq!(assets, 2, "the original plus one page");
    let page: String = f
        .conn
        .query_row(
            "SELECT id FROM assets WHERE item_id = ?1 AND parent_asset_id IS NOT NULL",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    let extractions: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(*) FROM extractions WHERE asset_id = ?1",
            [&page],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(extractions, 0, "no usable layer, no extraction");
}

/// S4: a text layer that cannot be read never fails the import: the page
/// imports without text, like a scan.
#[test]
fn an_unreadable_text_layer_never_fails_the_import() {
    let bytes = pdf_with_undecodable_page();
    assert!(
        crate::ocr::pdf::extract_pdf_text(&bytes).is_err(),
        "the fixture must actually break the text parser"
    );
    // The import reads the split page files, not the source: the broken
    // encoding must survive the split for this test to exercise the
    // failure path instead of the empty-layer one.
    let pages = crate::ocr::pdf::split_pdf_to_single_page_bytes(&bytes).unwrap();
    assert_eq!(pages.len(), 1);
    assert!(
        crate::ocr::pdf::extract_pdf_text(&pages[0].1).is_err(),
        "the split page must break the text parser too"
    );
    let f = fixture();
    let path = f.write("Danada.pdf", &bytes);
    let source = f.check(&path).unwrap();
    let ImportOutcome::Imported { item_id, .. } = import_file(
        &f.conn,
        &f.data_dir,
        BETA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap() else {
        panic!("expected an import");
    };
    let page: String = f
        .conn
        .query_row(
            "SELECT id FROM assets WHERE item_id = ?1 AND parent_asset_id IS NOT NULL",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    let extractions: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(*) FROM extractions WHERE asset_id = ?1",
            [&page],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(extractions, 0, "the broken layer leaves no text behind");
}

/// S4: pages stored as native at import read as already done, so a batch
/// without "Incluir texto nativo" skips them instead of re-running OCR.
#[test]
fn imported_native_pages_read_as_already_done() {
    use crate::processing::eligibility::{ocr_decision, OcrDecision};
    let f = fixture();
    let path = f.write("Actas.pdf", &pdf_with_rich_pages(2));
    let source = f.check(&path).unwrap();
    let ImportOutcome::Imported { item_id, .. } = import_file(
        &f.conn,
        &f.data_dir,
        BETA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap() else {
        panic!("expected an import");
    };
    let mut statement = f
        .conn
        .prepare("SELECT id FROM assets WHERE item_id = ?1 AND parent_asset_id IS NOT NULL")
        .unwrap();
    let pages: Vec<String> = statement
        .query_map([&item_id], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(pages.len(), 2);
    for page in &pages {
        assert_eq!(
            ocr_decision(&f.conn, page).unwrap(),
            OcrDecision::AlreadyDone {
                method: "native".to_string()
            },
            "{page}: a batch without native inclusion skips it"
        );
    }
}
/// A PDF with `count` pages of text, built in memory.
fn pdf_with_pages(count: u32) -> Vec<u8> {
    use lopdf::{dictionary, Document, Object, Stream};
    let mut document = Document::with_version("1.5");
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources = document.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let pages_id = document.new_object_id();
    let kids: Vec<Object> = (0..count)
        .map(|index| {
            let content = document.add_object(Stream::new(
                dictionary! {},
                format!("BT /F1 12 Tf 20 100 Td (Pagina {index}) Tj ET").into_bytes(),
            ));
            document
                .add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                    "Resources" => resources,
                    "Contents" => content,
                })
                .into()
        })
        .collect();
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Count" => count as i64, "Kids" => kids,
        }),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn file_kinds_follow_the_apps_own_list() {
    for (name, kind) in [
        ("carta.PDF", Some(FileKind::Pdf)),
        ("foto.jpeg", Some(FileKind::Image)),
        ("scan.tif", Some(FileKind::Image)),
        ("entrevista.m4a", Some(FileKind::Audio)),
        // The app's own audio list includes `ogg`, so an import that used to
        // accept it keeps accepting it now that the service does the work.
        ("entrevista.ogg", Some(FileKind::Audio)),
        ("notas.txt", None),
        ("sin_extension", None),
    ] {
        assert_eq!(classify(Path::new(name)), kind, "{name}");
    }
}

#[test]
fn a_supported_file_directly_in_an_enabled_folder_is_accepted() {
    let f = fixture();
    let path = f.write("Carta de Sarmiento.pdf", b"%PDF-1.4 x");
    let source = f.check(&path).unwrap();
    assert_eq!(source.name, "Carta de Sarmiento.pdf");
    assert_eq!(source.kind, FileKind::Pdf);
    assert_eq!(source.size_bytes, 10);
    // The real path is kept as the OS spells it (verbatim on Windows, which
    // allows long paths for the copy); what the app records is the plain form.
    assert_eq!(
        crate::path_utils::normalize_windows_path(&source.path),
        f.inbox.join("Carta de Sarmiento.pdf")
    );
    assert_eq!(
        original_path(&source),
        f.inbox.join("Carta de Sarmiento.pdf").to_string_lossy()
    );
    assert!(source.modified_at.is_some());
}

#[test]
fn files_outside_an_enabled_folder_or_in_the_archive_are_refused() {
    let f = fixture();
    std::fs::create_dir(f.inbox.join("sub")).unwrap();
    std::fs::write(f.inbox.join("sub").join("a.pdf"), b"%PDF").unwrap();
    std::fs::write(f.data_dir.join("b.pdf"), b"%PDF").unwrap();
    let outside = f._root.path().join("c.pdf");
    std::fs::write(&outside, b"%PDF").unwrap();
    for (path, reason) in [
        (f.inbox.join("sub").join("a.pdf"), "not_in_folder"),
        (f.data_dir.join("b.pdf"), "in_archive"),
        (outside.clone(), "not_in_folder"),
        (f.inbox.join("missing.pdf"), "missing"),
        (f.inbox.join("sub"), "not_a_file"),
        (PathBuf::from(f.write("notas.txt", b"x")), "unsupported"),
    ] {
        let error = f.check(&path.to_string_lossy()).unwrap_err();
        assert!(error.starts_with(reason), "{}: {error}", path.display());
    }
}

/// An enabled folder that holds the archive itself never exposes it.
#[test]
fn the_archive_is_refused_even_inside_an_enabled_folder() {
    let f = fixture();
    let parent = f.data_dir.parent().unwrap().to_path_buf();
    std::fs::write(f.data_dir.join("x.pdf"), b"%PDF").unwrap();
    std::fs::write(parent.join("ok.pdf"), b"%PDF").unwrap();
    let folders = vec![parent.clone(), f.data_dir.clone()];
    let limits = SourceLimits::default();
    assert!(check_source(
        &parent.join("ok.pdf").to_string_lossy(),
        &folders,
        &f.data_dir,
        &limits
    )
    .is_ok());
    let inside = check_source(
        &f.data_dir.join("x.pdf").to_string_lossy(),
        &folders,
        &f.data_dir,
        &limits,
    )
    .unwrap_err();
    assert!(inside.starts_with("in_archive"), "{inside}");
}

#[cfg(unix)]
#[test]
fn a_link_that_leads_out_of_the_folder_is_refused() {
    let f = fixture();
    let outside = f._root.path().join("secreto.pdf");
    std::fs::write(&outside, b"%PDF").unwrap();
    std::os::unix::fs::symlink(&outside, f.inbox.join("enlace.pdf")).unwrap();
    let error = f
        .check(&f.inbox.join("enlace.pdf").to_string_lossy())
        .unwrap_err();
    assert!(error.starts_with("not_in_folder"), "{error}");
}

#[test]
fn a_file_over_the_size_cap_is_refused() {
    let f = fixture();
    let path = f.write("grande.pdf", b"%PDF-1.4 0123456789");
    let error = check_source(
        &path,
        &f.folders(),
        &f.data_dir,
        &SourceLimits { max_bytes: 4 },
    )
    .unwrap_err();
    assert!(error.starts_with("too_large"), "{error}");
}

#[test]
fn listing_a_folder_shows_its_supported_files_by_name() {
    let f = fixture();
    f.write("b.pdf", b"%PDF");
    f.write("a.jpg", b"jpg");
    f.write("c.txt", b"x");
    std::fs::create_dir(f.inbox.join("sub")).unwrap();
    std::fs::write(f.inbox.join("sub").join("d.pdf"), b"%PDF").unwrap();
    let listing = list_folder(&f.inbox, &f.folders(), &f.data_dir, 500).unwrap();
    let names: Vec<&str> = listing
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    assert_eq!(names, vec!["a.jpg", "b.pdf"]);
    assert!(!listing.truncated);
    let capped = list_folder(&f.inbox, &f.folders(), &f.data_dir, 1).unwrap();
    assert_eq!(capped.files.len(), 1);
    assert!(capped.truncated);
    let refused = list_folder(&f.inbox.join("sub"), &f.folders(), &f.data_dir, 500).unwrap_err();
    assert!(refused.starts_with("not_enabled"), "{refused}");
}

#[test]
fn importing_an_image_creates_an_item_with_its_copy_and_provenance() {
    let f = fixture();
    let path = f.write("Retrato de Mitre.jpg", b"jpeg bytes");
    let source = f.check(&path).unwrap();
    let outcome = import_file(
        &f.conn,
        &f.data_dir,
        ALFA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap();
    let ImportOutcome::Imported {
        item_id,
        title,
        assets,
    } = outcome
    else {
        panic!("expected an import, got {outcome:?}");
    };
    assert_eq!(title, "Retrato de Mitre");
    assert_eq!(assets, 1);
    let (stored_title, metadata): (String, String) = f
        .conn
        .query_row(
            "SELECT title, metadata FROM items WHERE id = ?1 AND collection_id = ?2",
            [&item_id, ALFA],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_title, "Retrato de Mitre");
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    let provenance = &metadata["__entropia_file_metadata"];
    assert_eq!(provenance["originalName"], "Retrato de Mitre.jpg");
    assert_eq!(provenance["originalPath"], path);
    assert_eq!(provenance["sizeBytes"], 10);
    assert_eq!(provenance["isFile"], true);
    let (asset_path, kind, size): (String, String, i64) = f
        .conn
        .query_row(
            "SELECT path, type, size FROM assets WHERE item_id = ?1",
            [&item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(
        asset_path.starts_with(&format!("assets/{ALFA}/{item_id}/")),
        "stored as the relative key: {asset_path}"
    );
    assert!(asset_path.ends_with("_Retrato de Mitre.jpg"));
    assert_eq!(kind, "image");
    assert_eq!(size, 10);
    assert_eq!(
        std::fs::read(f.data_dir.join(&asset_path)).unwrap(),
        b"jpeg bytes"
    );
    let found: i64 = f
        .conn
        .query_row(
            "SELECT count(*) FROM fts_items f JOIN items i ON i.rowid = f.rowid
             WHERE fts_items MATCH 'mitre' AND i.id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(found, 1, "the title is searchable at once");
}

#[test]
fn importing_a_pdf_keeps_the_original_and_adds_one_asset_per_page() {
    let f = fixture();
    let path = f.write("Actas.pdf", &pdf_with_pages(3));
    let source = f.check(&path).unwrap();
    let ImportOutcome::Imported {
        item_id, assets, ..
    } = import_file(
        &f.conn,
        &f.data_dir,
        BETA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap()
    else {
        panic!("expected an import");
    };
    assert_eq!(assets, 4, "the original plus three pages");
    let mut statement = f
        .conn
        .prepare(
            "SELECT id, path, sort_index, parent_asset_id, page_number FROM assets
             WHERE item_id = ?1 ORDER BY parent_asset_id IS NOT NULL, sort_index",
        )
        .unwrap();
    let rows: Vec<AssetRow> = statement
        .query_map([&item_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let parent = &rows[0];
    assert_eq!(parent.3, None);
    assert_eq!(parent.4, None);
    for (index, page) in rows[1..].iter().enumerate() {
        assert_eq!(page.2, index as i64, "sort index follows the page");
        assert_eq!(page.3.as_deref(), Some(parent.0.as_str()));
        assert_eq!(page.4, Some(index as i64 + 1));
        assert!(f.data_dir.join(&page.1).is_file(), "{}", page.1);
    }
}

#[test]
fn the_same_unchanged_file_is_not_imported_twice_into_a_collection() {
    let f = fixture();
    let path = f.write("Carta.pdf", &pdf_with_pages(1));
    let source = f.check(&path).unwrap();
    let ImportOutcome::Imported { item_id, .. } = import_file(
        &f.conn,
        &f.data_dir,
        ALFA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap() else {
        panic!("expected an import");
    };
    let before = f.items_in(ALFA);
    assert_eq!(
        import_file(
            &f.conn,
            &f.data_dir,
            ALFA,
            &source,
            &ImportOverrides::default()
        )
        .unwrap(),
        ImportOutcome::Duplicate { item_id }
    );
    assert_eq!(f.items_in(ALFA), before);
    assert!(
        matches!(
            import_file(
                &f.conn,
                &f.data_dir,
                BETA,
                &source,
                &ImportOverrides::default()
            )
            .unwrap(),
            ImportOutcome::Imported { .. }
        ),
        "another collection may hold its own copy"
    );
}

#[test]
fn a_failed_import_leaves_no_item_and_no_files() {
    let f = fixture();
    let path = f.write("Roto.pdf", b"%PDF-1.4 not really a pdf");
    let source = f.check(&path).unwrap();
    let before = f.items_in(ALFA);
    let error = import_file(
        &f.conn,
        &f.data_dir,
        ALFA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(f.items_in(ALFA), before);
    let leftovers = std::fs::read_dir(f.data_dir.join("assets").join(ALFA))
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "the item's folder was removed");
}

#[test]
fn importing_into_an_unknown_collection_is_refused_before_anything_is_copied() {
    let f = fixture();
    let path = f.write("a.jpg", b"x");
    let source = f.check(&path).unwrap();
    let error = import_file(
        &f.conn,
        &f.data_dir,
        "col_missing",
        &source,
        &ImportOverrides::default(),
    )
    .unwrap_err();
    assert!(error.starts_with("not_found"), "{error}");
    assert!(!f.data_dir.join("assets").join("col_missing").exists());
}

/// D10(a): the app's own import reads what it stages inside the data directory
/// (Internet Archive staging, the Navegador copy, the sample collection) and
/// never a file it already imported under `assets/`.
#[test]
fn the_app_may_import_what_it_stages_but_never_its_own_assets() {
    let f = fixture();
    let staged = f.write_in_data("staging/internet-archive/Carta.pdf", b"%PDF");
    assert_eq!(f.app_check(&staged).unwrap().kind, FileKind::Pdf);
    let sample = f.write_in_data("sample-staging/acta.jpg", b"jpg");
    assert_eq!(f.app_check(&sample).unwrap().kind, FileKind::Image);

    let already_imported = f.write_in_data("assets/col-1/item-1/abc_Carta.pdf", b"%PDF");
    let error = f.app_check(&already_imported).unwrap_err();
    assert!(error.starts_with("in_archive"), "{error}");

    for (path, reason) in [
        (
            f.data_dir
                .join("staging")
                .join("missing.pdf")
                .to_string_lossy()
                .to_string(),
            "missing",
        ),
        (
            f.data_dir.join("staging").to_string_lossy().to_string(),
            "not_a_file",
        ),
        (f.write_in_data("staging/notas.txt", b"x"), "unsupported"),
    ] {
        let error = f.app_check(&path).unwrap_err();
        assert!(error.starts_with(reason), "{path}: {error}");
    }

    let big = f.write_in_data("staging/grande.pdf", b"%PDF-1.4 0123456789");
    let error = check_app_source(&big, &f.data_dir, &SourceLimits { max_bytes: 4 }).unwrap_err();
    assert!(error.starts_with("too_large"), "{error}");
}

/// D10(c): the app has always written `importedAt` with milliseconds
/// (`new Date().toISOString()`), and a copy's provenance is compared against it.
#[test]
fn an_import_records_when_it_happened_in_milliseconds() {
    let f = fixture();
    let path = f.write("foto.jpg", b"jpg");
    let source = f.app_check(&path).unwrap();
    let ImportOutcome::Imported { item_id, .. } = import_file(
        &f.conn,
        &f.data_dir,
        ALFA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap() else {
        panic!("expected an import");
    };
    let metadata: String = f
        .conn
        .query_row(
            "SELECT metadata FROM items WHERE id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    let imported_at = metadata["__entropia_file_metadata"]["importedAt"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(imported_at.len(), 24, "{imported_at}");
    assert!(imported_at.ends_with('Z'), "{imported_at}");
    assert_eq!(&imported_at[19..20], ".", "{imported_at}");
    assert!(
        imported_at[20..23].chars().all(|c| c.is_ascii_digit()),
        "{imported_at}"
    );
    assert_eq!(&imported_at[4..5], "-", "{imported_at}");
}

/// D10(b): the app names the item and the stored copy. A saved web capture, for
/// instance, arrives with the page's title and a file name made from it.
#[test]
fn the_app_can_name_the_item_the_copy_and_the_pages() {
    let f = fixture();
    let path = f.write("capture-1.pdf", &pdf_with_pages(2));
    let source = f.app_check(&path).unwrap();
    let overrides = ImportOverrides {
        title: Some("The paper".to_string()),
        file_name: Some("The paper.pdf".to_string()),
        ..ImportOverrides::default()
    };
    let ImportOutcome::Imported { item_id, title, .. } =
        import_file(&f.conn, &f.data_dir, ALFA, &source, &overrides).unwrap()
    else {
        panic!("expected an import");
    };
    assert_eq!(title, "The paper");
    let (stored_title, metadata): (String, String) = f
        .conn
        .query_row(
            "SELECT title, metadata FROM items WHERE id = ?1",
            [&item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_title, "The paper");
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(
        metadata["__entropia_file_metadata"]["originalName"], "The paper.pdf",
        "the copy's original name follows the name the app chose"
    );
    assert_eq!(metadata["__entropia_file_metadata"]["originalPath"], path);

    let mut statement = f
        .conn
        .prepare("SELECT path FROM assets WHERE item_id = ?1 ORDER BY sort_index")
        .unwrap();
    let paths: Vec<String> = statement
        .query_map([&item_id], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(paths[0].ends_with("_The paper.pdf"), "{}", paths[0]);
    assert!(paths[1].ends_with("The paper_page_1.pdf"), "{}", paths[1]);
    assert!(paths[2].ends_with("The paper_page_2.pdf"), "{}", paths[2]);
}

/// D10(b): `extraMetadata` is written beside `__entropia_file_metadata` and
/// can never replace it, whatever the caller sent.
#[test]
fn extra_metadata_sits_beside_the_import_provenance_and_never_replaces_it() {
    let f = fixture();
    let path = f.write("captura.jpg", b"jpg");
    let source = f.app_check(&path).unwrap();
    let extra: serde_json::Map<String, serde_json::Value> = serde_json::json!({
        "__entropia_web_capture": { "captureId": "c1" },
        "__entropia_file_metadata": { "originalName": "forged.jpg" },
        "tema": "historia"
    })
    .as_object()
    .unwrap()
    .clone();
    let overrides = ImportOverrides {
        extra_metadata: Some(extra),
        ..ImportOverrides::default()
    };
    let ImportOutcome::Imported { item_id, .. } =
        import_file(&f.conn, &f.data_dir, ALFA, &source, &overrides).unwrap()
    else {
        panic!("expected an import");
    };
    let metadata: String = f
        .conn
        .query_row(
            "SELECT metadata FROM items WHERE id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["__entropia_web_capture"]["captureId"], "c1");
    assert_eq!(metadata["tema"], "historia");
    assert_eq!(
        metadata["__entropia_file_metadata"]["originalName"],
        "captura.jpg"
    );
}

/// D10(b): a caller that already decided the file is new (the Navegador copy
/// asked the person) imports it even when an identical source is there.
#[test]
fn a_duplicate_can_be_allowed_for_a_file_the_app_already_checked() {
    let f = fixture();
    let path = f.write("Carta.pdf", &pdf_with_pages(1));
    let source = f.app_check(&path).unwrap();
    let first = import_file(
        &f.conn,
        &f.data_dir,
        ALFA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap();
    assert!(matches!(first, ImportOutcome::Imported { .. }));
    let allowed = ImportOverrides {
        allow_duplicate: true,
        ..ImportOverrides::default()
    };
    let before = f.items_in(ALFA);
    let second = import_file(&f.conn, &f.data_dir, ALFA, &source, &allowed).unwrap();
    assert!(
        matches!(second, ImportOutcome::Imported { .. }),
        "{second:?}"
    );
    assert_eq!(f.items_in(ALFA), before + 1);
}

/// The agent's own import keeps its rule: no overrides, a duplicate is skipped.
#[test]
fn an_import_without_overrides_keeps_its_title_and_its_duplicate_rule() {
    let f = fixture();
    let path = f.write("Acta de 1891.jpg", b"jpg");
    let source = f.check(&path).unwrap();
    let ImportOutcome::Imported { item_id, title, .. } = import_file(
        &f.conn,
        &f.data_dir,
        ALFA,
        &source,
        &ImportOverrides::default(),
    )
    .unwrap() else {
        panic!("expected an import");
    };
    assert_eq!(title, "Acta de 1891");
    assert_eq!(
        import_file(
            &f.conn,
            &f.data_dir,
            ALFA,
            &source,
            &ImportOverrides::default()
        )
        .unwrap(),
        ImportOutcome::Duplicate { item_id }
    );
}

/// On Windows `canonicalize` answers `\\?\C:\…`. Provenance, duplicates and the
/// enabled folders use the plain form the renderer always wrote, so an earlier
/// import still matches and the owner never sees the prefix.
#[cfg(windows)]
#[test]
fn windows_paths_are_recorded_and_matched_without_the_verbatim_prefix() {
    let data = tempfile::tempdir().unwrap();
    let inbox = tempfile::tempdir().unwrap();
    let file = inbox.path().join("Carta.jpg");
    std::fs::write(&file, b"jpg").unwrap();
    let plain_folder =
        crate::path_utils::normalize_windows_path(std::fs::canonicalize(inbox.path()).unwrap());
    assert!(!plain_folder.to_string_lossy().starts_with(r"\\?\"));
    let checked = check_source(
        &file.to_string_lossy(),
        std::slice::from_ref(&plain_folder),
        data.path(),
        &SourceLimits::default(),
    )
    .expect("a file directly inside a plainly stored folder is accepted");
    let verbatim_folder = std::fs::canonicalize(inbox.path()).unwrap();
    check_source(
        &file.to_string_lossy(),
        std::slice::from_ref(&verbatim_folder),
        data.path(),
        &SourceLimits::default(),
    )
    .expect("a folder list in the verbatim spelling still matches");
    let listed = list_folder(inbox.path(), &[plain_folder], data.path(), 10).unwrap();
    assert_eq!(listed.files.len(), 1);
    let original = original_path(&checked);
    assert!(!original.starts_with(r"\\?\"), "{original}");
}
