//! The help assistant's local index (S9, D6): the bundled knowledge documents
//! (for now the user manual only; T9/T10 add the FAQ and the screen map)
//! indexed in a standalone SQLite file under the cache dir — never in the
//! user's archive, so no migration and no sync.
//!
//! The file stores the SHA-256 of the bundled knowledge next to the index: a
//! different hash on open drops and rebuilds the index in one transaction,
//! the same hash reuses it untouched. Retrieval is FTS5 with the `trigram`
//! tokenizer for fuzzy matching, ranked by BM25 with the title weighted above
//! the body. Accents are folded in Rust (see `super::text`) on both the
//! indexed text and the query, so matching never depends on which diacritic
//! options the bundled SQLite was compiled with.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use super::{manual, text};

const CONTENT_HASH_KEY: &str = "content_hash";

/// Where the bundled knowledge comes from. Only the manual ships today; the
/// FAQ (T9) and the screen map (T10) extend this enum without changing the
/// index code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Manual,
    ScreenMap,
    Faq,
}

impl Source {
    fn as_str(&self) -> &'static str {
        match self {
            Source::Manual => "manual",
            Source::ScreenMap => "screen-map",
            Source::Faq => "faq",
        }
    }

    fn parse(value: &str) -> Option<Source> {
        match value {
            "manual" => Some(Source::Manual),
            "screen-map" => Some(Source::ScreenMap),
            "faq" => Some(Source::Faq),
            _ => None,
        }
    }
}

/// One indexable document: the manual (later the FAQ and the screen map)
/// converted to a common shape at the command layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KnowledgeDoc {
    pub source: Source,
    pub chapter: String,
    pub title: String,
    pub body: String,
}

/// One ranked hit: the original (unfolded) text plus its BM25 rank, where a
/// lower score answered better.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    pub source: Source,
    pub chapter: String,
    pub title: String,
    pub body: String,
    pub score: f64,
}

impl Hit {
    pub(crate) fn is_troubleshooting(&self) -> bool {
        manual::is_troubleshooting_title(&self.title)
    }

    /// Characters the hit takes in the prompt.
    pub(crate) fn len_chars(&self) -> usize {
        self.title.chars().count() + self.body.chars().count()
    }
}

/// The standalone help index over `path` (the command layer passes
/// `<cache_dir>/help-index.sqlite`).
pub(crate) struct HelpIndex {
    conn: Connection,
    // Test-only witness that `open` rebuilt rather than reused: nothing in
    // the app reads it, so it only exists where tests can see it.
    #[cfg(test)]
    rebuilt: bool,
}

/// SHA-256 over every document in order (source, chapter, title, body): any
/// bundled-manual change flips it and the next open rebuilds.
fn content_hash(docs: &[KnowledgeDoc]) -> String {
    let mut hasher = Sha256::new();
    for doc in docs {
        hasher.update(doc.source.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(doc.chapter.as_bytes());
        hasher.update([0]);
        hasher.update(doc.title.as_bytes());
        hasher.update([0]);
        hasher.update(doc.body.as_bytes());
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The FTS5 MATCH expression for a natural-language question: the
/// accent-folded words (>= 3 chars, stopwords dropped), each quoted, with
/// its 5-letter stem beside it, OR-joined. A misspelled word then simply
/// contributes nothing while the rest still retrieve; every term is long
/// enough for the trigram tokenizer to use its index.
fn match_expression(query: &str) -> String {
    let mut terms: Vec<String> = Vec::new();
    for word in text::fold(query).split(|c: char| !c.is_alphanumeric()) {
        if word.chars().count() < 3 || text::STOPWORDS.contains(&word) {
            continue;
        }
        let stem: String = word.chars().take(5).collect();
        for term in [word.to_string(), stem] {
            if !terms.contains(&term) {
                terms.push(term);
            }
        }
    }
    terms
        .iter()
        .map(|term| format!("\"{term}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

impl HelpIndex {
    /// The manual sections as index documents (T9/T10 append theirs here).
    pub(crate) fn manual_docs() -> Vec<KnowledgeDoc> {
        manual::sections()
            .iter()
            .map(|section| KnowledgeDoc {
                source: Source::Manual,
                chapter: section.chapter.clone(),
                title: section.title.clone(),
                body: section.body.clone(),
            })
            .collect()
    }

    /// Opens (or creates) the index file. When the stored content hash differs
    /// from `docs` — or there is none — the index is dropped and rebuilt in a
    /// single transaction; otherwise it is reused untouched.
    pub(crate) fn open(path: &Path, docs: &[KnowledgeDoc]) -> Result<HelpIndex, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("help index dir failed: {error}"))?;
            }
        }
        let mut conn =
            Connection::open(path).map_err(|error| format!("help index open failed: {error}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        )
        .map_err(|error| format!("help index meta failed: {error}"))?;
        let hash = content_hash(docs);
        let stored: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![CONTENT_HASH_KEY],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("help index hash read failed: {error}"))?;
        let rebuilt = stored.as_deref() != Some(hash.as_str());
        if rebuilt {
            let tx = conn
                .transaction()
                .map_err(|error| format!("help index rebuild failed: {error}"))?;
            tx.execute_batch(
                "DROP TABLE IF EXISTS docs_fts;
                 DROP TABLE IF EXISTS docs;
                 CREATE TABLE docs(
                   id INTEGER PRIMARY KEY,
                   source TEXT NOT NULL,
                   chapter TEXT NOT NULL,
                   title TEXT NOT NULL,
                   body TEXT NOT NULL
                 );
                 CREATE VIRTUAL TABLE docs_fts USING fts5(
                   title_folded,
                   body_folded,
                   tokenize='trigram'
                 );",
            )
            .map_err(|error| format!("help index rebuild failed: {error}"))?;
            {
                let mut insert_doc = tx
                    .prepare(
                        "INSERT INTO docs(source, chapter, title, body) VALUES (?1, ?2, ?3, ?4)",
                    )
                    .map_err(|error| format!("help index rebuild failed: {error}"))?;
                let mut insert_fts = tx
                    .prepare(
                        "INSERT INTO docs_fts(rowid, title_folded, body_folded) VALUES (?1, ?2, ?3)",
                    )
                    .map_err(|error| format!("help index rebuild failed: {error}"))?;
                for doc in docs {
                    insert_doc
                        .execute(params![
                            doc.source.as_str(),
                            doc.chapter,
                            doc.title,
                            doc.body
                        ])
                        .map_err(|error| format!("help index rebuild failed: {error}"))?;
                    let id = tx.last_insert_rowid();
                    insert_fts
                        .execute(params![id, text::fold(&doc.title), text::fold(&doc.body)])
                        .map_err(|error| format!("help index rebuild failed: {error}"))?;
                }
            }
            tx.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
                params![CONTENT_HASH_KEY, hash],
            )
            .map_err(|error| format!("help index rebuild failed: {error}"))?;
            tx.commit()
                .map_err(|error| format!("help index rebuild failed: {error}"))?;
        }
        Ok(HelpIndex {
            conn,
            #[cfg(test)]
            rebuilt,
        })
    }

    #[cfg(test)]
    pub(crate) fn rebuilt(&self) -> bool {
        self.rebuilt
    }

    /// The pinned troubleshooting chapter («¿Qué hago si…?»), independent of
    /// any query: `pick_sections` leads with it on failures.
    pub(crate) fn troubleshooting(&self) -> Option<Hit> {
        let mut stmt = self
            .conn
            .prepare("SELECT source, chapter, title, body FROM docs")
            .ok()?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .ok()?;
        for row in rows.filter_map(|row| row.ok()) {
            let (source, chapter, title, body) = row;
            if manual::is_troubleshooting_title(&title) {
                return Source::parse(&source).map(|source| Hit {
                    source,
                    chapter,
                    title,
                    body,
                    // Pinned, not ranked: sorts ahead of every BM25 hit.
                    score: f64::NEG_INFINITY,
                });
            }
        }
        None
    }

    /// The best `limit` hits for a natural-language question, best first. The
    /// MATCH expression OR-joins the accent-folded query words (>= 3 chars,
    /// stopwords dropped) with their 5-letter stems, so a misspelled word
    /// simply contributes nothing while the rest still retrieve. An empty or
    /// stopword-only question matches nothing.
    pub(crate) fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let expression = match_expression(query);
        if expression.is_empty() || limit == 0 {
            return Vec::new();
        }
        // Title weighted above body; a lower BM25 rank answered better, so
        // best first is ascending. A failed query answers nothing rather than
        // failing the whole help request: the index is a cache, rebuildable.
        let mut stmt = match self.conn.prepare(
            "SELECT docs.source, docs.chapter, docs.title, docs.body, bm25(docs_fts, 3.0, 1.0) AS rank
             FROM docs_fts JOIN docs ON docs.id = docs_fts.rowid
             WHERE docs_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![expression, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|row| row.ok())
            .filter_map(|(source, chapter, title, body, score)| {
                Source::parse(&source).map(|source| Hit {
                    source,
                    chapter,
                    title,
                    body,
                    score,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::text::fold;
    use super::*;

    fn sample_docs() -> Vec<KnowledgeDoc> {
        vec![
            KnowledgeDoc {
                source: Source::Manual,
                chapter: "Capítulo 1. Inicio rápido".to_string(),
                title: "1.1. Importar archivos".to_string(),
                body: "Arrastrá los PDF a la colección para importarlos.".to_string(),
            },
            KnowledgeDoc {
                source: Source::Manual,
                chapter: "Capítulo 2. Transcribir".to_string(),
                title: "2.1. Transcripción automática".to_string(),
                body: "La transcripción convierte el audio en texto.".to_string(),
            },
            KnowledgeDoc {
                source: Source::Manual,
                chapter: "Capítulo 19. ¿Qué hago si…?".to_string(),
                title: "Capítulo 19. ¿Qué hago si…?".to_string(),
                body: "Si algo falla, revisá la clave de OpenRouter.".to_string(),
            },
        ]
    }

    fn open_temp(docs: &[KnowledgeDoc]) -> (tempfile::TempDir, HelpIndex) {
        let dir = tempfile::tempdir().expect("tempdir");
        let index =
            HelpIndex::open(&dir.path().join("help-index.sqlite"), docs).expect("open index");
        (dir, index)
    }

    fn open_real_manual() -> (tempfile::TempDir, HelpIndex) {
        let dir = tempfile::tempdir().expect("tempdir");
        let docs = HelpIndex::manual_docs();
        assert!(docs.len() >= 20, "the manual must index its chapters");
        let index =
            HelpIndex::open(&dir.path().join("help-index.sqlite"), &docs).expect("open index");
        (dir, index)
    }

    #[test]
    fn first_open_builds_the_index() {
        let (_dir, index) = open_temp(&sample_docs());
        assert!(index.rebuilt(), "first open must rebuild");
        let hits = index.search("importar archivos", 10);
        assert!(
            hits.iter().any(|hit| hit.title.contains("Importar")),
            "import section searchable after build: {hits:?}"
        );
    }

    #[test]
    fn reopen_with_the_same_docs_reuses_the_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("help-index.sqlite");
        let docs = sample_docs();
        let first = HelpIndex::open(&path, &docs).expect("first open");
        assert!(first.rebuilt());
        drop(first);
        let second = HelpIndex::open(&path, &docs).expect("second open");
        assert!(!second.rebuilt(), "same docs must reuse the index");
        assert!(
            !second.search("importar archivos", 10).is_empty(),
            "reused index still answers"
        );
    }

    #[test]
    fn a_changed_body_rebuilds_the_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("help-index.sqlite");
        let docs = sample_docs();
        let first = HelpIndex::open(&path, &docs).expect("first open");
        assert!(first.rebuilt());
        drop(first);

        let mut changed = sample_docs();
        changed[0].body = "Arrastrá los PDF a la colección para zambullirlos.".to_string();
        let second = HelpIndex::open(&path, &changed).expect("reopen");
        assert!(second.rebuilt(), "a changed body must rebuild");
        assert!(
            second
                .search("zambullirlos", 10)
                .iter()
                .any(|hit| hit.title.contains("Importar")),
            "rebuilt index answers from the new body"
        );
    }

    #[test]
    fn how_to_query_finds_the_import_section_in_the_top_three() {
        let (_dir, index) = open_real_manual();
        let hits = index.search("¿Cómo importo archivos PDF?", 3);
        assert!(
            hits.iter()
                .any(|hit| hit.title.contains("Importar archivos")),
            "top 3: {:?}",
            hits.iter().map(|hit| &hit.title).collect::<Vec<_>>()
        );
    }

    #[test]
    fn misspelled_query_still_finds_an_import_section_in_the_top_five() {
        let (_dir, index) = open_real_manual();
        let hits = index.search("impotar archivos pdf", 5);
        assert!(
            hits.iter().any(|hit| hit.title.contains("Importar")),
            "top 5: {:?}",
            hits.iter().map(|hit| &hit.title).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_is_accent_insensitive() {
        let (_dir, index) = open_real_manual();
        let hits = index.search("transcripcion", 10);
        assert!(
            hits.iter()
                .any(|hit| fold(&hit.title).contains("transcripcion")
                    || fold(&hit.body).contains("transcripcion")),
            "unaccented query must meet «transcripción»: {:?}",
            hits.iter().map(|hit| &hit.title).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_respects_the_limit() {
        let (_dir, index) = open_real_manual();
        let hits = index.search("colección transcripción importar", 2);
        assert!(hits.len() <= 2, "{hits:?}");
    }

    #[test]
    fn empty_and_stopword_only_queries_return_no_hits() {
        let (_dir, index) = open_temp(&sample_docs());
        assert!(index.search("", 10).is_empty());
        assert!(index.search("   ", 10).is_empty());
        assert!(index.search("¿cómo que para con los las?", 10).is_empty());
    }
}
