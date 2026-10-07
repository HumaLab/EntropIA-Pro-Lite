//! Read-only adapter for the Biblioteca section (P2): listing the works of
//! the synced Zotero libraries, projecting one work's ficha, and preparing
//! one cataloged attachment for the in-app viewer.
//!
//! Everything here reads the E1b/E4 catalog tables as they already exist —
//! no migration, no schema change, no write of any kind. The attachment
//! opening reuses the same resolver/validator pair `prepare_passage_open`
//! uses, so the only path ever produced comes from a registered attachment
//! row, and the command layer grants exactly that one file.

use rusqlite::{Connection, OptionalExtension};

use super::attachment::OriginalKind;
use super::detail::{project_item, ItemDetailItem};
use super::repository::{
    get_extraction, page_texts_for_attachment, read_item, BibliographyError, BibliographyResult,
};
use crate::bibliography::attachment::{
    attachment_ref_for, is_html_snapshot, resolve_attachment_file, validate_pdf_original,
    AttachmentResolution,
};
use crate::bibliography::retrieval::{
    csl_authors, csl_year, read_work_display, resolve_zotero_library_rows, ViewableOriginal,
};

/// One work row in the Biblioteca list: the reading metadata a row shows
/// plus the identities needed to open it and to scope the next page.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkListEntry {
    /// The catalog's stable internal id (`bibliographic_items.id`). It is
    /// minted once and preserved across upserts, so it addresses the work
    /// for the whole life of the catalog row.
    pub item_id: String,
    /// The durable Zotero native key, kept alongside for display and for any
    /// flow that must survive a rebuilt catalog.
    pub item_key: String,
    /// Internal `zotero_libraries.id`, the scope a page of rows belongs to.
    pub library_row_id: String,
    pub title: String,
    /// CSL family names (or literal names), comma-separated; empty when none.
    pub authors: String,
    pub year: Option<i64>,
    pub library_name: String,
    /// `user` or `group` plus the native id: the identity Zotero uses.
    pub library_type: String,
    pub library_native_id: String,
    /// The catalog's last CSL-JSON, the offline fallback of the ficha.
    pub csl_json: String,
}

/// One page of works plus how many the scope holds in total, so the UI can
/// page without counting on its own.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkListPage {
    pub works: Vec<WorkListEntry>,
    pub total: i64,
}

/// How one page of works is ordered. `Title` is the default — what every
/// caller without a preference names — and `Recent` is Zotero's own recency:
/// `item_version` descending, works whose version is unknown last (recency
/// never guesses an order for a work it cannot date), ties by the same title
/// order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkListSort {
    #[default]
    Title,
    Recent,
}

/// The scope and paging of one [`list_works`] read. The library is named the
/// way the UI names it (`user`/`group` + native id); both fields must be
/// present for the scope to apply, exactly like the search commands.
pub struct WorkListRequest<'a> {
    pub library_type: Option<&'a str>,
    pub library_native_id: Option<&'a str>,
    /// Substring match over title and creators. `None` lists everything.
    pub query: Option<&'a str>,
    /// The order one page is read in. Callers without a preference send
    /// [`WorkListSort::Title`].
    pub sort: WorkListSort,
    pub offset: i64,
    pub limit: i64,
}

/// One page of a library's works, in the requested order ([`WorkListSort`])
/// with the internal id as a stable tiebreak. Tombstoned works are not
/// listed: Zotero no longer carries them. Works with no title fall back to
/// their key.
pub fn list_works(
    conn: &Connection,
    request: &WorkListRequest<'_>,
) -> BibliographyResult<WorkListPage> {
    let limit = request.limit.clamp(1, 200);
    let offset = request.offset.max(0);
    // The scope is the Zotero identity, exactly like the search commands:
    // both fields or neither, resolved here to the internal library rows.
    let scoped_rows = match (request.library_type, request.library_native_id) {
        (Some(library_type), Some(library_id)) => {
            Some(resolve_zotero_library_rows(conn, library_type, library_id)?)
        }
        _ => None,
    };
    if let Some(rows) = &scoped_rows {
        if rows.is_empty() {
            // The library was never synced into the catalog: nothing to list,
            // which is not the same as listing it and finding nothing.
            return Ok(WorkListPage {
                works: Vec::new(),
                total: 0,
            });
        }
    }

    let mut conditions = vec!["t.item_id IS NULL".to_string()];
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(rows) = &scoped_rows {
        let placeholders = (1..=rows.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        conditions.push(format!("i.library_id IN ({placeholders})"));
        for row in rows {
            params.push(Box::new(row.clone()));
        }
    }
    if let Some(query) = request.query.map(str::trim).filter(|q| !q.is_empty()) {
        // A plain substring match over title and creators: meaning-level
        // matching is `bibliography_search_works`, never this browse read.
        let start = params.len();
        conditions.push(format!(
            "(i.title LIKE ?{} OR i.creators_json LIKE ?{} OR i.csl_json_snapshot LIKE ?{})",
            start + 1,
            start + 2,
            start + 3
        ));
        let like = format!("%{query}%");
        for _ in 0..3 {
            params.push(Box::new(like.clone()));
        }
    }
    let where_clause = conditions.join(" AND ");
    let base = format!(
        "FROM bibliographic_items i
           JOIN zotero_libraries l ON l.id = i.library_id
           LEFT JOIN zotero_item_tombstones t ON t.item_id = i.id
          WHERE {where_clause}"
    );

    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) {base}"),
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            |row| row.get(0),
        )
        .map_err(|e| error("sql_error", format!("Failed to count catalog works: {e}")))?;

    // Title order is the default and the tiebreak of every order: on the
    // displayed name (falls back to the key), case-insensitive, with the
    // internal id for a stable end. `Recent` leads with Zotero's own
    // `item_version` descending and leaves works without a known version
    // until after every dated one.
    let order_by = match request.sort {
        WorkListSort::Title => "COALESCE(i.title, i.item_key) COLLATE NOCASE ASC, i.id ASC",
        WorkListSort::Recent => {
            "(i.item_version IS NULL) ASC, i.item_version DESC, \
             COALESCE(i.title, i.item_key) COLLATE NOCASE ASC, i.id ASC"
        }
    };
    let sql = format!(
        "SELECT i.id, i.item_key, i.library_id, COALESCE(i.title, i.item_key),
                i.csl_json_snapshot, l.name, l.library_type, l.library_id
          {base}
         ORDER BY {order_by}
         LIMIT ?{} OFFSET ?{}",
        params.len() + 1,
        params.len() + 2
    );
    let mut all_params = params;
    all_params.push(Box::new(limit));
    all_params.push(Box::new(offset));
    let mut statement = conn
        .prepare(&sql)
        .map_err(|e| error("sql_error", format!("Failed to prepare work listing: {e}")))?;
    let works = statement
        .query_map(
            rusqlite::params_from_iter(all_params.iter().map(|p| p.as_ref())),
            |row| {
                let csl_json: String = row.get(4)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    csl_json,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|e| error("sql_error", format!("Failed to read catalog works: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| error("sql_error", format!("Failed to read catalog works: {e}")))?
        .into_iter()
        .map(
            |(
                item_id,
                item_key,
                library_row_id,
                title,
                csl_json,
                library_name,
                library_type,
                library_native_id,
            )| {
                let csl: serde_json::Value =
                    serde_json::from_str(&csl_json).unwrap_or(serde_json::Value::Null);
                WorkListEntry {
                    item_id,
                    item_key,
                    library_row_id,
                    title,
                    authors: csl_authors(&csl),
                    year: csl_year(&csl),
                    library_name,
                    library_type,
                    library_native_id,
                    csl_json,
                }
            },
        )
        .collect();
    Ok(WorkListPage { works, total })
}

/// One work's ficha as the Biblioteca work view shows it: the display line
/// (authors, year, library) plus the confirmed catalog projection of the
/// item — metadata, collections, tags and attachment metadata. Read-only:
/// this claims nothing about Zotero's current state, only about the catalog.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDetail {
    pub item_id: String,
    pub item_key: String,
    pub title: String,
    pub authors: String,
    pub year: Option<i64>,
    pub library_name: String,
    pub library_type: String,
    pub library_native_id: String,
    pub csl_json: String,
    pub item: ItemDetailItem,
}

/// Reads one work by its internal item id. `unknown_work` when the catalog
/// has no such row.
pub fn work_detail(conn: &Connection, item_id: &str) -> BibliographyResult<WorkDetail> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT library_id, item_key FROM bibliographic_items WHERE id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| {
            error(
                "sql_error",
                format!("Failed to read bibliographic item: {e}"),
            )
        })?;
    let Some((library_row_id, item_key)) = row else {
        return Err(error(
            "unknown_work",
            format!("work {item_id} does not exist in the catalog"),
        ));
    };
    let item = read_item(conn, &library_row_id, &item_key)?;
    let snapshot = project_item(conn, &library_row_id, &item)?;
    let display =
        read_work_display(conn, item_id)?.unwrap_or(crate::bibliography::retrieval::WorkDisplay {
            authors: String::new(),
            year: None,
            library_name: String::new(),
            library_type: String::new(),
            library_native_id: String::new(),
            csl_json: item.csl_json_snapshot.clone(),
        });
    Ok(WorkDetail {
        item_id: item.id.clone(),
        item_key: item.identity.item_key.clone(),
        title: item
            .title
            .clone()
            .unwrap_or_else(|| item.identity.item_key.clone()),
        authors: display.authors,
        year: display.year,
        library_name: display.library_name,
        library_type: display.library_type,
        library_native_id: display.library_native_id,
        csl_json: display.csl_json,
        item: snapshot,
    })
}

/// [`work_detail`] with its "opened" bookkeeping side effect: opening a work
/// in the Biblioteca records it for the "opened works first" admission
/// order. Recording is best-effort — a failure is logged and never fails
/// the read.
pub fn work_detail_recording_open(
    conn: &Connection,
    item_id: &str,
    now_ms: i64,
) -> BibliographyResult<WorkDetail> {
    let detail = work_detail(conn, item_id)?;
    crate::settings::record_work_opened_best_effort(conn, item_id, now_ms);
    Ok(detail)
}

/// One extracted page text of a work attachment, exactly as the catalog
/// stores it (`bibliographic_page_texts`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkPageText {
    pub page_number: i64,
    /// `native` or `ocr`.
    pub method: String,
    /// `rich`, `sparse`, `empty` or `unreadable`.
    pub quality: String,
    pub text: String,
}

/// One attachment prepared for the in-app viewer: the original validated
/// for viewing (or the reason there is none), the per-page extracted texts
/// and the whole-document extraction. Nothing here launches anything; the
/// command layer grants the one validated file to the webview.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkAttachmentOpen {
    pub item_id: String,
    pub item_key: String,
    pub title: String,
    pub attachment_id: String,
    pub attachment_key: String,
    /// The original validated for viewing: a PDF with its canonical path, or
    /// an HTML snapshot shown from the catalog's stored text.
    pub original: Option<ViewableOriginal>,
    /// `(reason, detail)` when no original is viewable: the resolver's or
    /// the validator's stable strings, shown verbatim by the UI.
    pub reason: Option<(String, String)>,
    pub pages: Vec<WorkPageText>,
    /// The whole-document extraction (`bibliographic_extractions`), empty
    /// when the attachment was never extracted.
    pub snapshot_text: String,
    /// False when neither page texts nor a whole-document extraction exist:
    /// the "Texto" tab then shows its pending state instead of a blank.
    pub extracted: bool,
}

/// Prepares one cataloged attachment of one work for viewing inside
/// EntropIA. The frontend names the work and the attachment key, never a
/// path: the file comes from the attachment row, is canonicalized and
/// validated as an existing regular PDF (HTML snapshots need no file), and
/// the caller may then allow that single file on the asset protocol. An
/// unknown work or attachment fails with `unknown_work` /
/// `unknown_attachment`; an unviewable original travels as `reason`, never
/// as an error, so the extracted text still renders.
pub fn prepare_work_attachment_open(
    conn: &Connection,
    item_id: &str,
    attachment_key: &str,
    zotero_data_dir: Option<&str>,
) -> BibliographyResult<WorkAttachmentOpen> {
    let item: Option<(String, String)> = conn
        .query_row(
            "SELECT item_key, COALESCE(title, item_key)
               FROM bibliographic_items WHERE id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| {
            error(
                "sql_error",
                format!("Failed to read bibliographic item: {e}"),
            )
        })?;
    let Some((item_key, title)) = item else {
        return Err(error(
            "unknown_work",
            format!("work {item_id} does not exist in the catalog"),
        ));
    };
    let attachment_id: String = conn
        .query_row(
            "SELECT id FROM zotero_attachments WHERE item_id = ?1 AND attachment_key = ?2",
            rusqlite::params![item_id, attachment_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| error("sql_error", format!("Failed to read attachment: {e}")))?
        .ok_or_else(|| {
            error(
                "unknown_attachment",
                format!("attachment {attachment_key} is not cataloged for work {item_id}"),
            )
        })?;

    // The exact resolution chain `prepare_passage_open` runs: the file comes
    // from the attachment row alone, canonicalized and validated before any
    // grant; an HTML snapshot needs no file; an unviewable original travels
    // as `reason`, never as an error.
    let attachment = attachment_ref_for(conn, &attachment_id).map_err(|e| {
        error(
            "sql_error",
            format!("Failed to read passage attachment: {e}"),
        )
    })?;
    let (original, reason) = match attachment {
        None => (
            None,
            Some((
                "unknown_attachment".to_string(),
                "the attachment left the catalog".to_string(),
            )),
        ),
        Some(attachment) if is_html_snapshot(&attachment) => (
            Some(ViewableOriginal {
                kind: OriginalKind::Html,
                path: None,
            }),
            None,
        ),
        Some(attachment) => match resolve_attachment_file(&attachment, zotero_data_dir) {
            AttachmentResolution::File(path) => {
                match validate_pdf_original(&path, attachment.content_type.as_deref()) {
                    Ok(canonical) => (
                        Some(ViewableOriginal {
                            kind: OriginalKind::Pdf,
                            path: Some(canonical),
                        }),
                        None,
                    ),
                    Err(reason) => (None, Some(reason)),
                }
            }
            AttachmentResolution::Unavailable { reason, detail } => {
                (None, Some((reason.to_string(), detail)))
            }
        },
    };

    let pages = page_texts_for_attachment(conn, &attachment_id)?
        .into_iter()
        .map(|row| WorkPageText {
            page_number: row.page_number,
            method: row.method,
            quality: row.quality,
            text: row.text_content,
        })
        .collect::<Vec<_>>();
    let extraction = get_extraction(conn, &attachment_id)?;
    let has_extraction = extraction.is_some();
    let snapshot_text = extraction.map(|row| row.text_content).unwrap_or_default();
    Ok(WorkAttachmentOpen {
        item_id: item_id.to_string(),
        item_key,
        title,
        attachment_id,
        attachment_key: attachment_key.to_string(),
        original,
        reason,
        extracted: has_extraction || !pages.is_empty(),
        pages,
        snapshot_text,
    })
}

/// [`prepare_work_attachment_open`] with the same "opened" bookkeeping side
/// effect: opening a work attachment opens its work. Recording is
/// best-effort — a failure is logged and never fails the read.
pub fn prepare_work_attachment_recording_open(
    conn: &Connection,
    item_id: &str,
    attachment_key: &str,
    zotero_data_dir: Option<&str>,
    now_ms: i64,
) -> BibliographyResult<WorkAttachmentOpen> {
    let plan = prepare_work_attachment_open(conn, item_id, attachment_key, zotero_data_dir)?;
    crate::settings::record_work_opened_best_effort(conn, item_id, now_ms);
    Ok(plan)
}

/// The stable error constructor these reads use (same shape as the rest of
/// the bibliography module).
fn error(code: &str, message: impl Into<String>) -> BibliographyError {
    BibliographyError::new(code, message.into())
}

#[cfg(test)]
mod tests {
    use super::super::repository::{
        tombstone_item, upsert_attachment, upsert_collection, upsert_connection,
        upsert_extraction_in_transaction, upsert_item, upsert_item_collection, upsert_item_tag,
        upsert_library, upsert_page_text_in_transaction, upsert_tag, AttachmentInput,
        BibliographicItemInput, CollectionInput, ExtractionRow, ItemCollectionInput, ItemTagInput,
        LibraryType as CatalogLibraryType, PageTextRow, SourceOrigin, TagInput, TombstoneInput,
        UpsertConnection, UpsertLibrary,
    };
    use super::*;
    use rusqlite::Connection;

    fn migrated_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
        ))
        .expect("apply catalog foundation");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
        ))
        .expect("apply relations");
        // 0052's per-attachment extraction table, copied verbatim from that
        // migration. The migration's other half rebuilds the processing
        // tables this catalog-only harness does not carry; the shape read
        // here is the one the registry ships.
        conn.execute_batch(
            "CREATE TABLE bibliographic_extractions (
                attachment_id TEXT PRIMARY KEY NOT NULL REFERENCES zotero_attachments(id) ON DELETE CASCADE,
                item_id TEXT NOT NULL,
                page_count INTEGER NOT NULL,
                method TEXT NOT NULL CHECK(method IN ('native')),
                text_content TEXT NOT NULL,
                text_hash TEXT NOT NULL,
                text_chars INTEGER NOT NULL,
                quality TEXT NOT NULL CHECK(quality IN ('rich', 'sparse', 'empty')),
                source_mtime INTEGER,
                source_bytes INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX idx_bibliographic_extractions_item
                ON bibliographic_extractions(item_id);",
        )
        .expect("apply extraction table");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0053_bibliographic_page_texts.sql"
        ))
        .expect("apply page texts");
        conn
    }

    fn connection(id: &str) -> UpsertConnection {
        UpsertConnection {
            id: id.to_string(),
            source_origin: SourceOrigin::Local,
            source_instance_id: None,
            endpoint: None,
            capabilities_json: "{}".to_string(),
        }
    }

    fn library(
        connection_id: &str,
        library_type: CatalogLibraryType,
        library_id: &str,
    ) -> UpsertLibrary {
        UpsertLibrary {
            connection_id: connection_id.to_string(),
            library_type,
            library_id: library_id.to_string(),
            name: format!("Biblioteca {library_id}"),
            last_modified_version: Some(1),
        }
    }

    fn item(
        key: &str,
        title: &str,
        creators_json: Option<&str>,
        csl: &str,
    ) -> BibliographicItemInput {
        BibliographicItemInput {
            item_key: key.to_string(),
            item_version: Some(1),
            native_json_snapshot: format!(r#"{{"key":"{key}"}}"#),
            csl_json_snapshot: csl.to_string(),
            item_type: Some("book".to_string()),
            title: Some(title.to_string()),
            creators_json: creators_json.map(str::to_string),
            publication_title: None,
            publisher: None,
            date: None,
            doi: None,
            isbn: None,
            abstract_text: None,
            language: None,
            url: None,
        }
    }

    fn csl_with_author(family: &str, year: i64) -> String {
        format!(
            r#"{{"id":"csl","type":"book","author":[{{"family":"{family}"}}],"issued":{{"date-parts":[[{year}]]}}}}"#
        )
    }

    fn seed_library(conn: &mut Connection, native_id: &str) -> String {
        let source = upsert_connection(conn, connection("conn-wv")).expect("connection");
        let saved = upsert_library(
            conn,
            library(&source.id, CatalogLibraryType::User, native_id),
        )
        .expect("library");
        saved.id
    }

    fn seed_work(
        conn: &mut Connection,
        library_row_id: &str,
        key: &str,
        title: &str,
        creators_json: Option<&str>,
        csl: &str,
    ) -> String {
        let saved =
            upsert_item(conn, library_row_id, item(key, title, creators_json, csl)).expect("item");
        saved.id
    }

    fn attachment(
        key: &str,
        content_type: Option<&str>,
        link_mode: Option<&str>,
        filename: Option<&str>,
        native_path: Option<&str>,
    ) -> AttachmentInput {
        AttachmentInput {
            attachment_key: key.to_string(),
            content_type: content_type.map(str::to_string),
            link_mode: link_mode.map(str::to_string),
            filename: filename.map(str::to_string),
            native_path: native_path.map(str::to_string),
            url: None,
            md5: None,
            mtime: None,
            native_json_snapshot: format!(r#"{{"key":"{key}"}}"#),
            native_version: Some(1),
        }
    }

    // ── list_works ──────────────────────────────────────────────────────

    #[test]
    fn lists_pages_by_title_with_the_scope_total() {
        let mut conn = migrated_db();
        let lib_a = seed_library(&mut conn, "a");
        let lib_b = seed_library(&mut conn, "b");
        seed_work(&mut conn, &lib_a, "K1", "B work", None, "{}");
        seed_work(&mut conn, &lib_a, "K2", "a work", None, "{}");
        seed_work(&mut conn, &lib_b, "K3", "C work", None, "{}");

        let page = list_works(
            &conn,
            &WorkListRequest {
                library_type: None,
                library_native_id: None,
                query: None,
                offset: 0,
                limit: 2,
                sort: WorkListSort::Title,
            },
        )
        .expect("page");
        let titles: Vec<_> = page.works.iter().map(|w| w.title.as_str()).collect();
        // Case-insensitive title order, not insertion order.
        assert_eq!(titles, ["a work", "B work"]);
        assert_eq!(page.total, 3);

        let next = list_works(
            &conn,
            &WorkListRequest {
                library_type: None,
                library_native_id: None,
                query: None,
                offset: 2,
                limit: 2,
                sort: WorkListSort::Title,
            },
        )
        .expect("next page");
        let titles: Vec<_> = next.works.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["C work"]);

        let scoped = list_works(
            &conn,
            &WorkListRequest {
                library_type: Some("user"),
                library_native_id: Some("b"),
                query: None,
                offset: 0,
                limit: 10,
                sort: WorkListSort::Title,
            },
        )
        .expect("scoped page");
        assert_eq!(scoped.total, 1);
        assert_eq!(scoped.works[0].title, "C work");
        assert_eq!(scoped.works[0].library_row_id, lib_b);
        assert_eq!(scoped.works[0].library_native_id, "b");
    }

    #[test]
    fn an_unsynced_library_scope_is_empty_not_an_error() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        seed_work(&mut conn, &lib, "K1", "A work", None, "{}");

        let page = list_works(
            &conn,
            &WorkListRequest {
                library_type: Some("user"),
                library_native_id: Some("never-synced"),
                query: None,
                offset: 0,
                limit: 10,
                sort: WorkListSort::Title,
            },
        )
        .expect("page");
        assert!(page.works.is_empty());
        assert_eq!(page.total, 0);
    }

    #[test]
    fn the_query_matches_title_and_creators() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        seed_work(
            &mut conn,
            &lib,
            "K1",
            "El oficio de historiador",
            Some(r#"[{"creatorType":"author","firstName":"Marc","lastName":"Bloch"}]"#),
            &csl_with_author("Bloch", 1949),
        );
        seed_work(&mut conn, &lib, "K2", "La sociedad", None, "{}");

        let by_title = list_works(
            &conn,
            &WorkListRequest {
                library_type: None,
                library_native_id: None,
                query: Some("oficio"),
                offset: 0,
                limit: 10,
                sort: WorkListSort::Title,
            },
        )
        .expect("by title");
        assert_eq!(by_title.total, 1);
        assert_eq!(by_title.works[0].item_key, "K1");
        assert_eq!(by_title.works[0].authors, "Bloch");
        assert_eq!(by_title.works[0].year, Some(1949));

        let by_creator = list_works(
            &conn,
            &WorkListRequest {
                library_type: None,
                library_native_id: None,
                query: Some("Bloch"),
                offset: 0,
                limit: 10,
                sort: WorkListSort::Title,
            },
        )
        .expect("by creator");
        assert_eq!(by_creator.total, 1);

        let none = list_works(
            &conn,
            &WorkListRequest {
                library_type: None,
                library_native_id: None,
                query: Some("zzz"),
                offset: 0,
                limit: 10,
                sort: WorkListSort::Title,
            },
        )
        .expect("no match");
        assert!(none.works.is_empty());
        assert_eq!(none.total, 0);
    }

    #[test]
    fn tombstoned_works_are_never_listed() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        let kept = seed_work(&mut conn, &lib, "K1", "Kept", None, "{}");
        let _gone = seed_work(&mut conn, &lib, "K2", "Gone", None, "{}");
        let _ = kept;
        tombstone_item(
            &mut conn,
            &lib,
            "K2",
            TombstoneInput {
                remote_version: Some(2),
                reason: "removed upstream".to_string(),
            },
        )
        .expect("tombstone");

        let page = list_works(
            &conn,
            &WorkListRequest {
                library_type: None,
                library_native_id: None,
                query: None,
                offset: 0,
                limit: 10,
                sort: WorkListSort::Title,
            },
        )
        .expect("page");
        let titles: Vec<_> = page.works.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["Kept"]);
        assert_eq!(page.total, 1);
    }

    #[test]
    fn recent_order_sorts_by_item_version_newest_first_and_unknown_last() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        seed_work(&mut conn, &lib, "K1", "Zeta", None, "{}");
        seed_work(&mut conn, &lib, "K2", "Alpha", None, "{}");
        seed_work(&mut conn, &lib, "K3", "Middle", None, "{}");
        seed_work(&mut conn, &lib, "K4", "Beta", None, "{}");
        conn.execute(
            "UPDATE bibliographic_items SET item_version = 2 WHERE item_key = 'K1'",
            [],
        )
        .expect("Zeta at version 2");
        conn.execute(
            "UPDATE bibliographic_items SET item_version = 5 WHERE item_key IN ('K2', 'K4')",
            [],
        )
        .expect("Alpha and Beta at version 5");
        conn.execute(
            "UPDATE bibliographic_items SET item_version = NULL WHERE item_key = 'K3'",
            [],
        )
        .expect("Middle without a known version");

        let recent = list_works(
            &conn,
            &WorkListRequest {
                library_type: None,
                library_native_id: None,
                query: None,
                offset: 0,
                limit: 10,
                sort: WorkListSort::Recent,
            },
        )
        .expect("recent page");
        let titles: Vec<_> = recent.works.iter().map(|w| w.title.as_str()).collect();
        // Zotero's item_version descending, ties by title, unknown versions
        // last: recency never invents an order for a work it cannot date.
        assert_eq!(titles, ["Alpha", "Beta", "Zeta", "Middle"]);

        let by_title = list_works(
            &conn,
            &WorkListRequest {
                library_type: None,
                library_native_id: None,
                query: None,
                offset: 0,
                limit: 10,
                sort: WorkListSort::Title,
            },
        )
        .expect("title page");
        let titles: Vec<_> = by_title.works.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["Alpha", "Beta", "Middle", "Zeta"]);
    }

    // ── work_detail ─────────────────────────────────────────────────────

    #[test]
    fn detail_projects_metadata_memberships_and_attachments() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        let item_id = seed_work(
            &mut conn,
            &lib,
            "K1",
            "El oficio de historiador",
            Some(r#"[{"creatorType":"author","firstName":"Marc","lastName":"Bloch"}]"#),
            &csl_with_author("Bloch", 1949),
        );
        let collection = upsert_collection(
            &mut conn,
            &lib,
            CollectionInput {
                collection_key: "C1".to_string(),
                name: "Historia".to_string(),
                parent_collection_key: None,
                native_json_snapshot: r#"{"key":"C1"}"#.to_string(),
                native_version: Some(1),
            },
        )
        .expect("collection");
        upsert_item_collection(
            &mut conn,
            &lib,
            ItemCollectionInput {
                item_id: item_id.clone(),
                collection_id: collection.id.clone(),
            },
        )
        .expect("membership");
        let tag = upsert_tag(
            &mut conn,
            &lib,
            TagInput {
                tag_text: "metodología".to_string(),
                tag_type: None,
                native_json_snapshot: r#"{"tag":"metodología"}"#.to_string(),
                native_version: Some(1),
            },
        )
        .expect("tag");
        upsert_item_tag(
            &mut conn,
            &lib,
            ItemTagInput {
                item_id: item_id.clone(),
                tag_id: tag.id.clone(),
            },
        )
        .expect("item tag");
        let saved_attachment = upsert_attachment(
            &mut conn,
            &item_id,
            attachment(
                "ATT1",
                Some("application/pdf"),
                Some("imported_file"),
                Some("oficio.pdf"),
                None,
            ),
        )
        .expect("attachment");

        let detail = work_detail(&conn, &item_id).expect("detail");
        assert_eq!(detail.item_id, item_id);
        assert_eq!(detail.item_key, "K1");
        assert_eq!(detail.title, "El oficio de historiador");
        assert_eq!(detail.authors, "Bloch");
        assert_eq!(detail.year, Some(1949));
        assert_eq!(detail.library_name, "Biblioteca a");
        assert_eq!(detail.library_type, "user");
        assert_eq!(detail.library_native_id, "a");
        assert_eq!(detail.item.collections, ["Historia"]);
        assert_eq!(detail.item.tags, ["metodología"]);
        assert_eq!(detail.item.attachments.len(), 1);
        let first = &detail.item.attachments[0];
        assert_eq!(first.attachment_key, "ATT1");
        assert_eq!(first.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(first.filename.as_deref(), Some("oficio.pdf"));
        assert_eq!(saved_attachment.attachment_key, "ATT1");
    }

    #[test]
    fn detail_of_an_unknown_work_says_so() {
        let conn = migrated_db();
        let error = work_detail(&conn, "no-such-item").expect_err("unknown work");
        assert_eq!(error.code, "unknown_work");
    }

    // ── prepare_work_attachment_open ────────────────────────────────────

    fn seed_pdf_attachment(
        conn: &mut Connection,
        item_id: &str,
        key: &str,
        path: &std::path::Path,
    ) {
        upsert_attachment(
            conn,
            item_id,
            attachment(
                key,
                Some("application/pdf"),
                Some("linked_file"),
                Some("doc.pdf"),
                Some(path.to_str().expect("utf-8 path")),
            ),
        )
        .expect("attachment");
    }

    fn seed_page_texts(conn: &Connection, attachment_id: &str) {
        for (page, text) in [(1i64, "Primera página."), (2, "Segunda página.")] {
            upsert_page_text_in_transaction(
                conn,
                &PageTextRow {
                    attachment_id: attachment_id.to_string(),
                    page_number: page,
                    method: "native".to_string(),
                    text_content: text.to_string(),
                    text_hash: format!("hash-{page}"),
                    text_chars: text.len() as i64,
                    quality: "rich".to_string(),
                },
                1,
            )
            .expect("page text");
        }
    }

    fn seed_extraction(conn: &Connection, attachment_id: &str, item_id: &str, text: &str) {
        upsert_extraction_in_transaction(
            conn,
            &ExtractionRow {
                attachment_id: attachment_id.to_string(),
                item_id: item_id.to_string(),
                page_count: 1,
                method: "native".to_string(),
                text_content: text.to_string(),
                text_hash: "hash".to_string(),
                text_chars: text.len() as i64,
                quality: "rich".to_string(),
                source_mtime: Some(1),
                source_bytes: 10,
            },
            1,
        )
        .expect("extraction");
    }

    fn temp_pdf(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("entropia-work-view-{name}.pdf"));
        std::fs::write(&path, b"%PDF-1.4\nsynthetic original\n%%EOF").expect("write temp pdf");
        path
    }

    #[test]
    fn a_pdf_original_validates_and_carries_its_page_texts() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        let item_id = seed_work(&mut conn, &lib, "K1", "Obra", None, "{}");
        let pdf = temp_pdf("valid");
        seed_pdf_attachment(&mut conn, &item_id, "ATT1", &pdf);
        let attachment_id: String = conn
            .query_row(
                "SELECT id FROM zotero_attachments WHERE attachment_key = 'ATT1'",
                [],
                |r| r.get(0),
            )
            .expect("attachment id");
        seed_page_texts(&conn, &attachment_id);

        let plan = prepare_work_attachment_open(&conn, &item_id, "ATT1", None).expect("prepare");
        assert_eq!(plan.item_key, "K1");
        assert_eq!(plan.title, "Obra");
        let original = plan.original.expect("viewable original");
        assert_eq!(original.kind, OriginalKind::Pdf);
        let granted_path = original.path.expect("pdf path");
        assert!(granted_path.is_file());
        assert!(plan.reason.is_none());
        let pages: Vec<_> = plan
            .pages
            .iter()
            .map(|p| (p.page_number, p.text.as_str()))
            .collect();
        assert_eq!(pages, [(1, "Primera página."), (2, "Segunda página.")]);
        assert!(plan.extracted);
    }

    #[test]
    fn an_html_snapshot_needs_no_file_and_carries_its_stored_text() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        let item_id = seed_work(&mut conn, &lib, "K1", "Obra web", None, "{}");
        upsert_attachment(
            &mut conn,
            &item_id,
            attachment(
                "ATT1",
                Some("text/html"),
                Some("imported_url"),
                Some("captura.html"),
                None,
            ),
        )
        .expect("attachment");
        let attachment_id: String = conn
            .query_row(
                "SELECT id FROM zotero_attachments WHERE attachment_key = 'ATT1'",
                [],
                |r| r.get(0),
            )
            .expect("attachment id");
        seed_extraction(&conn, &attachment_id, &item_id, "Texto de la captura.");

        let plan = prepare_work_attachment_open(&conn, &item_id, "ATT1", None).expect("prepare");
        let original = plan.original.expect("viewable original");
        assert_eq!(original.kind, OriginalKind::Html);
        assert!(original.path.is_none());
        assert!(plan.reason.is_none());
        assert_eq!(plan.snapshot_text, "Texto de la captura.");
        assert!(plan.extracted);
    }

    #[test]
    fn an_unviewable_file_travels_as_a_reason_not_an_error() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        let item_id = seed_work(&mut conn, &lib, "K1", "Obra", None, "{}");
        let missing = std::env::temp_dir().join("entropia-work-view-definitely-missing.pdf");
        let _ = std::fs::remove_file(&missing);
        seed_pdf_attachment(&mut conn, &item_id, "ATT1", &missing);

        let plan = prepare_work_attachment_open(&conn, &item_id, "ATT1", None).expect("prepare");
        assert!(plan.original.is_none());
        let (reason, _detail) = plan.reason.expect("reason");
        assert_eq!(reason, "linked_file_missing");
        // The text still travels: the reason never gates the "Texto" tab.
        assert!(plan.pages.is_empty());
        assert!(!plan.extracted);
    }

    #[test]
    fn unknown_works_and_attachments_are_errors() {
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        let item_id = seed_work(&mut conn, &lib, "K1", "Obra", None, "{}");

        let unknown_work =
            prepare_work_attachment_open(&conn, "no-such-item", "ATT1", None).expect_err("work");
        assert_eq!(unknown_work.code, "unknown_work");
        let unknown_attachment =
            prepare_work_attachment_open(&conn, &item_id, "NOPE", None).expect_err("attachment");
        assert_eq!(unknown_attachment.code, "unknown_attachment");
    }

    // ── opened-works bookkeeping on the read commands ───────────────────

    fn create_app_settings(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("app_settings table");
    }

    fn recorded_opens(conn: &Connection) -> Vec<(String, i64)> {
        let raw: String = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'bibliography_recently_opened'",
                [],
                |row| row.get(0),
            )
            .expect("recorded list");
        serde_json::from_str::<Vec<crate::settings::RecentlyOpenedEntry>>(&raw)
            .expect("valid recently-opened JSON")
            .into_iter()
            .map(|entry| (entry.item_id, entry.opened_at))
            .collect()
    }

    #[test]
    fn reading_a_work_detail_records_the_opened_work() {
        let mut conn = migrated_db();
        create_app_settings(&conn);
        let lib = seed_library(&mut conn, "a");
        let item_id = seed_work(&mut conn, &lib, "K1", "Obra", None, "{}");

        let detail =
            work_detail_recording_open(&conn, &item_id, 1_700_000_000_123).expect("detail");

        assert_eq!(detail.item_id, item_id);
        assert_eq!(recorded_opens(&conn), vec![(item_id, 1_700_000_000_123)]);
    }

    #[test]
    fn a_work_read_succeeds_even_when_recording_the_open_fails() {
        // No `app_settings` table: recording cannot persist anything.
        let mut conn = migrated_db();
        let lib = seed_library(&mut conn, "a");
        let item_id = seed_work(&mut conn, &lib, "K1", "Obra", None, "{}");

        let detail = work_detail_recording_open(&conn, &item_id, 5).expect("read must not fail");

        assert_eq!(detail.item_id, item_id);
    }

    #[test]
    fn opening_an_attachment_records_the_work_and_survives_a_recording_failure() {
        let mut conn = migrated_db();
        create_app_settings(&conn);
        let lib = seed_library(&mut conn, "a");
        let item_id = seed_work(&mut conn, &lib, "K1", "Obra", None, "{}");
        let pdf = temp_pdf("recording");
        seed_pdf_attachment(&mut conn, &item_id, "ATT1", &pdf);

        let plan = prepare_work_attachment_recording_open(&conn, &item_id, "ATT1", None, 42)
            .expect("prepare");

        assert_eq!(plan.item_id, item_id);
        assert_eq!(recorded_opens(&conn), vec![(item_id.clone(), 42)]);

        // With recording broken the read still succeeds.
        conn.execute("DROP TABLE app_settings", []).expect("drop");
        prepare_work_attachment_recording_open(&conn, &item_id, "ATT1", None, 43)
            .expect("read must not fail");
    }
}
