//! Model-proposed reference validation (E7b): every citation a model
//! writes is checked against the verified catalog and the evidence set
//! that was actually supplied to it — per domain.
//!
//! Pure reads, no network, no model calls. Verdicts are advisory data,
//! never silent rewrites: the UI renders unverified references with
//! their reasons instead of presenting them as checked.

use rusqlite::{Connection, OptionalExtension as _};

use super::repository::{BibliographyError, BibliographyResult};

/// One reference exactly as the model proposed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedReference {
    /// Item key (bibliography) or item id (corpus) as written.
    pub key: String,
    /// External library namespace when the model gave one (`0`, `6680944`).
    pub library_id: Option<String>,
    /// Title as cited, when the model gave one.
    pub title: Option<String>,
    /// Which evidence set the reference claims: `corpus` or `bibliography`.
    pub domain: String,
}

/// The evidence actually supplied to the model, per domain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuppliedEvidence {
    /// Internal `bibliographic_items.id` rows shown to the model.
    pub bibliography_item_ids: Vec<String>,
    /// Corpus `items.id` rows shown to the model.
    pub corpus_item_ids: Vec<String>,
}

/// One checked reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceVerdict {
    pub proposed_index: usize,
    pub verified: bool,
    /// Internal row id when resolved (`bibliographic_items.id` or corpus
    /// `items.id`); `None` when the key resolves nowhere.
    pub item_id: Option<String>,
    /// Machine reasons, empty when verified.
    pub reasons: Vec<String>,
}

/// Checks every proposed reference against the catalog and the supplied
/// evidence, in order. Reasons are stable strings: `unknown_key`,
/// `tombstoned`, `title_mismatch`, `not_in_evidence`, `wrong_domain`,
/// `bad_domain`.
pub fn validate_references(
    conn: &Connection,
    proposed: &[ProposedReference],
    evidence: &SuppliedEvidence,
) -> BibliographyResult<Vec<ReferenceVerdict>> {
    let mut verdicts = Vec::with_capacity(proposed.len());
    for (proposed_index, reference) in proposed.iter().enumerate() {
        verdicts.push(validate_one(conn, proposed_index, reference, evidence)?);
    }
    Ok(verdicts)
}

fn normalize_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn validate_one(
    conn: &Connection,
    proposed_index: usize,
    reference: &ProposedReference,
    evidence: &SuppliedEvidence,
) -> BibliographyResult<ReferenceVerdict> {
    let mut reasons: Vec<String> = Vec::new();
    let mut item_id: Option<String> = None;
    match reference.domain.as_str() {
        "bibliography" => {
            // A corpus id claimed as bibliography (or the reverse) is a
            // domain error even when the row exists somewhere.
            if !reference.key.is_empty()
                && conn
                    .query_row(
                        "SELECT COUNT(*) FROM items WHERE id = ?1",
                        [&reference.key],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|count| count > 0)
                    .unwrap_or(false)
            {
                reasons.push("wrong_domain".to_string());
            }
            let row: Option<(String, String, String, Option<String>)> = conn
                .query_row(
                    "SELECT i.id, i.library_id, i.title,
                            (SELECT t.item_id FROM zotero_item_tombstones t WHERE t.item_id = i.id)
                     FROM bibliographic_items i WHERE i.item_key = ?1",
                    [&reference.key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
                .map_err(|error| {
                    BibliographyError::new(
                        "sql_error",
                        format!("Failed to resolve proposed reference: {error}"),
                    )
                })?;
            match row {
                None => reasons.push("unknown_key".to_string()),
                Some((id, library_id, title, tombstone)) => {
                    item_id = Some(id.clone());
                    if tombstone.is_some() {
                        reasons.push("tombstoned".to_string());
                    }
                    if let Some(library) = reference.library_id.as_deref() {
                        // External namespaces (`0`, group ids) resolve
                        // through the library row's external id, not the
                        // internal row id.
                        let external: Option<String> = conn
                            .query_row(
                                "SELECT library_id FROM zotero_libraries WHERE id = ?1",
                                [&library_id],
                                |row| row.get(0),
                            )
                            .optional()
                            .map_err(|error| {
                                BibliographyError::new(
                                    "sql_error",
                                    format!("Failed to resolve library namespace: {error}"),
                                )
                            })?;
                        if external.as_deref() != Some(library) {
                            reasons.push("unknown_key".to_string());
                        }
                    }
                    if let Some(cited) = reference.title.as_deref() {
                        if normalize_title(cited) != normalize_title(&title) {
                            reasons.push("title_mismatch".to_string());
                        }
                    }
                    if !evidence.bibliography_item_ids.contains(&id) {
                        reasons.push("not_in_evidence".to_string());
                    }
                }
            }
        }
        "corpus" => {
            if !reference.key.is_empty()
                && conn
                    .query_row(
                        "SELECT COUNT(*) FROM bibliographic_items WHERE item_key = ?1",
                        [&reference.key],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|count| count > 0)
                    .unwrap_or(false)
            {
                reasons.push("wrong_domain".to_string());
            }
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM items WHERE id = ?1",
                    [&reference.key],
                    |row| row.get::<_, i64>(0),
                )
                .map(|count| count > 0)
                .map_err(|error| {
                    BibliographyError::new(
                        "sql_error",
                        format!("Failed to resolve corpus reference: {error}"),
                    )
                })?;
            if !exists {
                reasons.push("unknown_key".to_string());
            } else {
                item_id = Some(reference.key.clone());
                if !evidence.corpus_item_ids.contains(&reference.key) {
                    reasons.push("not_in_evidence".to_string());
                }
            }
        }
        _ => reasons.push("bad_domain".to_string()),
    }
    Ok(ReferenceVerdict {
        proposed_index,
        verified: reasons.is_empty(),
        item_id,
        reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL);")
            .expect("corpus items");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0038_bibliography_catalog.sql"
        ))
        .expect("apply catalog foundation");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0039_bibliography_relations.sql"
        ))
        .expect("apply relations");
        conn
    }

    fn seed_catalog(conn: &mut Connection) -> (String, String) {
        use super::super::repository::{
            upsert_connection, upsert_item, upsert_library, BibliographicItemInput, LibraryType,
            SourceOrigin, UpsertConnection, UpsertLibrary,
        };
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
        .expect("connection");
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
        .expect("library");
        let live = upsert_item(
            conn,
            &library.id,
            BibliographicItemInput {
                item_key: "LIVE0001".to_string(),
                item_version: Some(3),
                native_json_snapshot: r#"{"key":"LIVE0001","version":3}"#.to_string(),
                csl_json_snapshot: r#"{"id":"LIVE0001","type":"book","title":"Obra viva"}"#
                    .to_string(),
                title: Some("Obra viva".to_string()),
                ..Default::default()
            },
        )
        .expect("live item");
        let dead = upsert_item(
            conn,
            &library.id,
            BibliographicItemInput {
                item_key: "DEAD0001".to_string(),
                item_version: Some(2),
                native_json_snapshot: r#"{"key":"DEAD0001","version":2}"#.to_string(),
                csl_json_snapshot: r#"{"id":"DEAD0001","type":"book","title":"Obra revocada"}"#
                    .to_string(),
                title: Some("Obra revocada".to_string()),
                ..Default::default()
            },
        )
        .expect("dead item");
        conn.execute(
            "INSERT INTO zotero_item_tombstones (item_id, observed_at, reason)
             VALUES (?1, 1, 'deleted')",
            [dead.id],
        )
        .expect("tombstone the dead item");
        conn.execute(
            "INSERT INTO items (id, title) VALUES ('corpus-1', 'Documento corpus')",
            [],
        )
        .expect("corpus item");
        (live.id, library.id)
    }

    fn evidence(live_id: &str) -> SuppliedEvidence {
        SuppliedEvidence {
            bibliography_item_ids: vec![live_id.to_string()],
            corpus_item_ids: vec!["corpus-1".to_string()],
        }
    }

    fn proposed(key: &str, domain: &str) -> ProposedReference {
        ProposedReference {
            key: key.to_string(),
            library_id: None,
            title: None,
            domain: domain.to_string(),
        }
    }

    #[test]
    fn verified_references_pass_with_identity() {
        let mut conn = validate_db();
        let (live_id, _) = seed_catalog(&mut conn);
        let verdicts = validate_references(
            &conn,
            &[ProposedReference {
                key: "LIVE0001".to_string(),
                library_id: Some("0".to_string()),
                title: Some("Obra viva".to_string()),
                domain: "bibliography".to_string(),
            }],
            &evidence(&live_id),
        )
        .expect("validate");
        assert_eq!(verdicts.len(), 1);
        assert!(verdicts[0].verified, "a supplied live work verifies");
        assert_eq!(verdicts[0].item_id.as_deref(), Some(live_id.as_str()));
        assert!(verdicts[0].reasons.is_empty());
    }

    #[test]
    fn hallucinations_tombstones_and_mismatches_fail_honestly() {
        let mut conn = validate_db();
        let (live_id, _) = seed_catalog(&mut conn);
        let verdicts = validate_references(
            &conn,
            &[
                proposed("NOEXIST1", "bibliography"),
                proposed("DEAD0001", "bibliography"),
                ProposedReference {
                    key: "LIVE0001".to_string(),
                    library_id: None,
                    title: Some("Título inventado".to_string()),
                    domain: "bibliography".to_string(),
                },
                proposed("LIVE0001", "corpus"),
                proposed("corpus-1", "bibliography"),
            ],
            &evidence(&live_id),
        )
        .expect("validate");
        assert_eq!(verdicts.len(), 5);
        assert!(!verdicts[0].verified);
        assert!(verdicts[0].reasons.contains(&"unknown_key".to_string()));
        assert!(verdicts[1].reasons.contains(&"tombstoned".to_string()));
        assert!(verdicts[2].reasons.contains(&"title_mismatch".to_string()));
        assert!(verdicts[3].reasons.contains(&"wrong_domain".to_string()));
        assert!(verdicts[4].reasons.contains(&"wrong_domain".to_string()));
        assert!(
            verdicts.iter().all(|verdict| !verdict.verified),
            "nothing unverified passes"
        );
    }

    #[test]
    fn valid_works_outside_the_evidence_set_do_not_verify() {
        let mut conn = validate_db();
        let (live_id, _) = seed_catalog(&mut conn);
        let verdicts = validate_references(
            &conn,
            &[proposed("LIVE0001", "bibliography")],
            &SuppliedEvidence::default(),
        )
        .expect("validate");
        assert!(!verdicts[0].verified);
        assert!(verdicts[0].reasons.contains(&"not_in_evidence".to_string()));
        assert_eq!(
            verdicts[0].item_id.as_deref(),
            Some(live_id.as_str()),
            "the work still resolves — only the evidence link fails"
        );
    }

    #[test]
    fn corpus_references_verify_against_corpus_evidence() {
        let mut conn = validate_db();
        let (live_id, _) = seed_catalog(&mut conn);
        let verdicts = validate_references(
            &conn,
            &[proposed("corpus-1", "corpus")],
            &evidence(&live_id),
        )
        .expect("validate");
        assert!(verdicts[0].verified);
        assert_eq!(verdicts[0].item_id.as_deref(), Some("corpus-1"));
    }

    #[test]
    fn unknown_domains_fail_closed() {
        let mut conn = validate_db();
        let (live_id, _) = seed_catalog(&mut conn);
        let verdicts = validate_references(
            &conn,
            &[proposed("LIVE0001", "annotations")],
            &evidence(&live_id),
        )
        .expect("validate");
        assert!(!verdicts[0].verified);
        assert!(verdicts[0].reasons.contains(&"bad_domain".to_string()));
    }
}
