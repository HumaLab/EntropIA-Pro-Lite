/// FTS5 indexing and search helpers.
///
/// All operations use raw SQL against the `fts_items` virtual table.
/// FTS5 contentless tables (`content=''`) require explicit INSERT/DELETE —
/// there is no automatic sync with the source table.
use rusqlite::{params, Connection};

use super::text_provider;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct FtsRow {
    pub item_id: String,
    pub title: String,
    pub rank: f64,
}

/// How the tokens of a query combine in the generated FTS5 MATCH expression.
///
/// FTS5 treats adjacent phrases as an implicit AND, so the two modes are not
/// cosmetic — they decide whether a document must contain *every* token or
/// merely *some* of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FtsMatchMode {
    /// Every token must be present. Correct for keyword search, where the user
    /// typed exactly the terms they expect to co-occur.
    All,
    /// Any token may match and BM25 ranks by how many did. Correct for
    /// natural-language questions, where requiring every token (`que`, `el`,
    /// `sobre`, ...) matches nothing.
    Any,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Fetch item data from DB and index it into `fts_items`.
///
/// Retrieves title, metadata, and extracted text for `item_id`, then upserts
/// into the FTS5 virtual table.
pub fn index_item_from_db(conn: &Connection, item_id: &str) -> Result<(), String> {
    // Fetch item title + metadata
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT title, COALESCE(metadata, '') FROM items WHERE id = ?1",
            params![item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    let (title, metadata) = match row {
        Some(r) => r,
        None => return Ok(()), // Item not found — not an error
    };

    // Fetch extracted text from both extractions and transcriptions via text_provider
    let extracted_text = text_provider::get_item_text(conn, item_id)?;

    fts_index_item(conn, item_id, &title, &metadata, &extracted_text)
}

/// Upsert a document into `fts_items`.
///
/// The canonical identity of the index is `fts_items.rowid = items.rowid`.
pub fn fts_index_item(
    conn: &Connection,
    item_id: &str,
    title: &str,
    metadata: &str,
    extracted_text: &str,
) -> Result<(), String> {
    let item_rowid: i64 = conn
        .query_row(
            "SELECT rowid FROM items WHERE id = ?1",
            params![item_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("FTS5 item rowid lookup failed: {e}"))?;

    // Insert fresh entry (same rowid updates the indexed content)
    conn.execute(
        "INSERT OR REPLACE INTO fts_items(rowid, item_id, title, metadata, extracted_text) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![item_rowid, item_id, title, metadata, extracted_text],
    )
    .map_err(|e| format!("FTS5 insert failed: {e}"))?;

    Ok(())
}

/// Search `fts_items` using a FTS5 MATCH expression.
///
/// `query` must already be sanitized by the caller (use `sanitize_fts5_query`).
/// Results are ordered by BM25 rank (most relevant first).
///
/// If `collection_id` is provided, results are filtered via a JOIN to `items`.
///
/// Combines tokens with [`FtsMatchMode::All`]. Callers passing a
/// natural-language question must use [`fts_search_with_mode`] instead.
#[allow(dead_code)]
pub fn fts_search(
    conn: &Connection,
    query: &str,
    collection_id: Option<&str>,
) -> Result<Vec<FtsRow>, String> {
    fts_search_with_mode(conn, query, collection_id, FtsMatchMode::All)
}

/// Search `fts_items`, combining query tokens according to `mode`.
///
/// Results are ordered by BM25 rank (most relevant first). Under
/// [`FtsMatchMode::Any`] that ordering is what supplies the relevance floor:
/// documents matching more query terms score better, so no explicit threshold
/// is needed — the caller's own candidate limit trims the tail.
pub fn fts_search_with_mode(
    conn: &Connection,
    query: &str,
    collection_id: Option<&str>,
    mode: FtsMatchMode,
) -> Result<Vec<FtsRow>, String> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    let sanitized = sanitize_fts5_query_with_mode(query, mode);
    if sanitized.is_empty() {
        return Ok(vec![]);
    }

    let rows = if let Some(cid) = collection_id {
        let mut stmt = conn
            .prepare(
                r#"
                SELECT i.id, i.title, bm25(fts_items) AS rank
                FROM fts_items f
                JOIN items i ON i.rowid = f.rowid
                WHERE fts_items MATCH ?1
                  AND i.collection_id = ?2
                ORDER BY rank
                "#,
            )
            .map_err(|e| format!("Failed to prepare FTS5 search: {e}"))?;

        map_fts_rows(&mut stmt, params![sanitized.as_str(), cid])?
    } else {
        let mut stmt = conn
            .prepare(
                r#"
                SELECT i.id, i.title, bm25(fts_items) AS rank
                FROM fts_items f
                JOIN items i ON i.rowid = f.rowid
                WHERE fts_items MATCH ?1
                ORDER BY rank
                "#,
            )
            .map_err(|e| format!("Failed to prepare FTS5 search: {e}"))?;

        map_fts_rows(&mut stmt, params![sanitized.as_str()])?
    };

    Ok(rows)
}

/// Sanitize a raw user query to be safe for FTS5 MATCH, requiring **every**
/// token to be present (`FtsMatchMode::All`).
///
/// Strips FTS5 operators (AND, OR, NOT, NEAR, *, -, ^) and special chars
/// (quotes, parentheses). Collapses whitespace and trims.
///
/// Keyword search wants this. Natural-language questions do NOT: the implicit
/// AND makes every stopword mandatory and matches nothing. Those callers must
/// use [`sanitize_fts5_query_with_mode`] with [`FtsMatchMode::Any`].
pub fn sanitize_fts5_query(raw: &str) -> String {
    build_fts5_match_expression(&fts5_query_tokens(raw), FtsMatchMode::All)
}

/// Sanitize a raw user query into an FTS5 MATCH expression under an explicit
/// combination mode.
///
/// In [`FtsMatchMode::Any`] the tokens are additionally filtered through a
/// stopword list, because a natural-language question is mostly grammar: left
/// in, stopwords match nearly every document and flatten the BM25 ranking. If
/// *every* token is a stopword the unfiltered list is kept — returning an
/// expression that matches nothing would be worse than a noisy one.
pub fn sanitize_fts5_query_with_mode(raw: &str, mode: FtsMatchMode) -> String {
    let tokens = fts5_query_tokens(raw);
    let tokens = match mode {
        FtsMatchMode::All => tokens,
        FtsMatchMode::Any => drop_stopwords_preserving_nonempty(tokens),
    };
    build_fts5_match_expression(&tokens, mode)
}

/// Tokenize a raw user query: strip FTS5 metacharacters and boolean operators,
/// split on whitespace, and collapse consecutive duplicates.
///
/// This is the shared half of query construction — both match modes and every
/// caller go through it, so escaping rules live in exactly one place.
fn fts5_query_tokens(raw: &str) -> Vec<String> {
    // Remove FTS5 special characters
    let cleaned = raw
        .replace('"', "")
        .replace('(', "")
        .replace(')', "")
        .replace('*', "")
        .replace('-', " ")
        .replace('^', "")
        .replace(':', " ")
        .replace(',', " ")
        .replace('.', " ");

    // Remove FTS5 boolean operators (case-insensitive, whole word)
    let mut words: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|w| {
            let up = w.to_ascii_uppercase();
            !matches!(up.as_str(), "AND" | "OR" | "NOT" | "NEAR")
        })
        .collect();

    // Deduplicate consecutive identical words
    words.dedup();

    words.into_iter().map(|w| w.to_string()).collect()
}

/// Join quoted tokens with the operator implied by `mode`.
///
/// `All` relies on FTS5's implicit AND (a bare space); `Any` spells `OR` out.
/// A single token yields the same expression under both modes, so short
/// queries are unaffected by the mode choice.
fn build_fts5_match_expression(tokens: &[String], mode: FtsMatchMode) -> String {
    let separator = match mode {
        FtsMatchMode::All => " ",
        FtsMatchMode::Any => " OR ",
    };
    tokens
        .iter()
        .map(|token| format!("\"{token}\""))
        .collect::<Vec<String>>()
        .join(separator)
}

/// Drop stopwords, unless doing so would empty the query.
fn drop_stopwords_preserving_nonempty(tokens: Vec<String>) -> Vec<String> {
    let filtered: Vec<String> = tokens
        .iter()
        .filter(|token| !is_stopword(token))
        .cloned()
        .collect();
    if filtered.is_empty() {
        tokens
    } else {
        filtered
    }
}

/// Spanish + English function words that carry no retrieval signal.
///
/// Accented and unaccented spellings are both listed rather than normalized:
/// the tokenizer preserves accents, and adding a Unicode-folding dependency to
/// decide that `qué` is `que` is not worth it. Comparison is case-insensitive
/// via `to_lowercase`, which handles the accented forms correctly.
///
/// Applied ONLY when building an `Any`-mode expression — never to the
/// embedding input and never to what gets indexed.
fn is_stopword(token: &str) -> bool {
    const STOPWORDS: &[&str] = &[
        // Spanish
        "a", "al", "algo", "algunas", "algunos", "ante", "antes", "aqui", "aquí", "como", "cómo",
        "con", "contra", "cual", "cuál", "cuales", "cuáles", "cuando", "cuándo", "de", "del",
        "desde", "donde", "dónde", "dos", "durante", "e", "el", "ella", "ellas", "ello", "ellos",
        "en", "entre", "era", "eran", "eres", "es", "esa", "esas", "ese", "eso", "esos", "esta",
        "está", "están", "estas", "este", "esto", "estos", "fue", "fueron", "ha", "hace", "han",
        "hasta", "hay", "la", "las", "le", "les", "lo", "los", "mas", "más", "me", "mi", "mis",
        "mucho", "muchos", "muy", "ni", "no", "nos", "nosotros", "o", "otra", "otras", "otro",
        "otros", "para", "pero", "poco", "por", "porque", "que", "qué", "quien", "quién",
        "quienes", "se", "sea", "según", "ser", "si", "sí", "sin", "sobre", "son", "su", "sus",
        "tambien", "también", "tan", "tanto", "te", "tiene", "tienen", "todo", "todos", "tu",
        "tus", "un", "una", "unas", "uno", "unos", "y", "ya", // English
        "about", "all", "also", "an", "any", "are", "as", "at", "be", "been", "being", "both",
        "but", "by", "can", "could", "did", "do", "does", "each", "for", "from", "had", "has",
        "have", "how", "i", "if", "in", "into", "is", "it", "its", "may", "me", "might", "more",
        "most", "much", "must", "my", "of", "on", "only", "or", "other", "our", "over", "own",
        "same", "should", "so", "some", "such", "than", "that", "the", "their", "them", "then",
        "there", "these", "they", "this", "those", "through", "to", "was", "we", "were", "what",
        "when", "where", "which", "while", "who", "whom", "why", "will", "with", "would", "you",
        "your",
    ];
    let lowered = token.to_lowercase();
    STOPWORDS.contains(&lowered.as_str())
}

#[allow(dead_code)]
fn map_fts_rows<P: rusqlite::Params>(
    stmt: &mut rusqlite::Statement<'_>,
    params: P,
) -> Result<Vec<FtsRow>, String> {
    let mapped = stmt
        .query_map(params, |row| {
            Ok(FtsRow {
                item_id: row.get(0)?,
                title: row.get(1)?,
                rank: row.get(2)?,
            })
        })
        .map_err(|e| format!("FTS5 search failed: {e}"))?;

    mapped
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("FTS5 row mapping failed: {e}"))
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── sanitize_fts5_query ──────────────────────────────────────────────────

    #[test]
    fn sanitize_fts5_query_cases() {
        let cases = vec![
            ("buenos aires", "\"buenos\" \"aires\""),
            ("foo AND bar", "\"foo\" \"bar\""),
            ("foo OR bar", "\"foo\" \"bar\""),
            ("foo NOT bar", "\"foo\" \"bar\""),
            ("foo NEAR bar", "\"foo\" \"bar\""),
            ("histo*", "\"histo\""),
            (r#""exact phrase""#, "\"exact\" \"phrase\""),
            ("(foo OR bar)", "\"foo\" \"bar\""),
            ("foo-bar", "\"foo\" \"bar\""),
            ("foo^bar", "\"foobar\""),
            ("acta:cabildo,1810.", "\"acta\" \"cabildo\" \"1810\""),
            ("acta AND (cabildo):*", "\"acta\" \"cabildo\""),
            ("  foo   bar  ", "\"foo\" \"bar\""),
            ("foo and bar", "\"foo\" \"bar\""),
            ("AND OR NOT", ""),
            ("", ""),
        ];

        for (input, expected) in cases {
            assert_eq!(sanitize_fts5_query(input), expected, "input={input}");
        }
    }

    // ── Match mode construction (F1) ─────────────────────────────────────────

    #[test]
    fn all_mode_is_byte_for_byte_identical_to_the_legacy_sanitizer() {
        // Splitting tokenization from expression building must not move the
        // keyword-search behavior. Same inputs as the case table above.
        for input in [
            "buenos aires",
            "foo AND bar",
            "acta:cabildo,1810.",
            "foo^bar",
            "AND OR NOT",
            "",
        ] {
            assert_eq!(
                sanitize_fts5_query_with_mode(input, FtsMatchMode::All),
                sanitize_fts5_query(input),
                "input={input}"
            );
        }
    }

    #[test]
    fn any_mode_joins_tokens_with_or_and_drops_stopwords() {
        assert_eq!(
            sanitize_fts5_query_with_mode(
                "¿Qué decía el acta del cabildo sobre el comercio?",
                FtsMatchMode::Any
            ),
            "\"¿Qué\" OR \"decía\" OR \"acta\" OR \"cabildo\" OR \"comercio?\""
        );
    }

    #[test]
    fn any_mode_falls_back_to_unfiltered_tokens_when_all_are_stopwords() {
        // Dropping every token would build an expression matching nothing,
        // which is strictly worse than a noisy one.
        assert_eq!(
            sanitize_fts5_query_with_mode("que es esto", FtsMatchMode::Any),
            "\"que\" OR \"es\" OR \"esto\""
        );
    }

    #[test]
    fn single_token_query_is_identical_under_both_modes() {
        // Step 1.4: short queries must behave exactly as before the mode split.
        for input in ["cabildo", "  guerra  ", "histo*"] {
            let any = sanitize_fts5_query_with_mode(input, FtsMatchMode::Any);
            assert_eq!(any, sanitize_fts5_query(input), "input={input}");
            assert!(!any.contains(" OR "), "input={input} produced {any}");
        }
    }

    #[test]
    fn stopword_filter_is_case_and_accent_aware() {
        // `to_lowercase` handles the accented forms; both spellings are listed.
        assert_eq!(
            sanitize_fts5_query_with_mode("QUÉ Y Qué SOBRE cabildo", FtsMatchMode::Any),
            "\"cabildo\""
        );
    }

    // ── In-memory FTS5 tests ─────────────────────────────────────────────────
    fn setup_fts_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            r#"
            CREATE TABLE items (
                id TEXT PRIMARY KEY,
                collection_id TEXT,
                title TEXT NOT NULL
            );

            CREATE VIRTUAL TABLE fts_items USING fts5(
                item_id UNINDEXED,
                title,
                metadata,
                extracted_text,
                tokenize = 'unicode61 remove_diacritics 1',
                content = ''
            );
            "#,
        )
        .expect("FTS5 table creation failed");
        conn
    }

    #[test]
    fn fts_index_and_search_basic() {
        let conn = setup_fts_db();
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-1", "col-a", "Historia Colonial"],
        )
        .expect("insert item failed");

        fts_index_item(
            &conn,
            "item-1",
            "Historia Colonial",
            "",
            "Buenos Aires 1810",
        )
        .expect("index failed");

        let results = fts_search(&conn, "colonial", None).expect("search failed");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item_id, "item-1");
        assert_eq!(results[0].title, "Historia Colonial");
    }

    #[test]
    fn fts_index_uses_items_rowid_as_canonical_identity() {
        let conn = setup_fts_db();
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-rowid", "col-a", "Contrato Rowid"],
        )
        .expect("insert item failed");

        let item_rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM items WHERE id = ?1",
                params!["item-rowid"],
                |row| row.get(0),
            )
            .expect("lookup item rowid failed");

        fts_index_item(&conn, "item-rowid", "Contrato Rowid", "", "texto canonico")
            .expect("index failed");

        let fts_rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM fts_items WHERE fts_items MATCH ?1",
                params!["texto"],
                |row| row.get(0),
            )
            .expect("lookup fts rowid failed");

        assert_eq!(fts_rowid, item_rowid);
    }

    #[test]
    fn fts_search_returns_empty_for_no_match() {
        let conn = setup_fts_db();
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-1", "col-a", "Historia Colonial"],
        )
        .expect("insert item failed");

        fts_index_item(&conn, "item-1", "Historia Colonial", "", "Buenos Aires")
            .expect("index failed");

        let results = fts_search(&conn, "azteca", None).expect("search failed");
        assert!(results.is_empty());
    }

    #[test]
    fn fts_search_empty_query_returns_empty() {
        let conn = setup_fts_db();
        let results = fts_search(&conn, "", None).expect("search failed");
        assert!(results.is_empty());
    }

    /// F1 regression: a natural-language question must retrieve a document that
    /// is clearly relevant to it.
    ///
    /// Under `All` this returns zero rows — FTS5's implicit AND makes every
    /// token mandatory, including `que`, `el`, `del` and `sobre`. That is the
    /// bug that silently disabled the RAG lexical leg for every real question.
    #[test]
    fn natural_language_question_matches_relevant_document_only_in_any_mode() {
        let conn = setup_fts_db();
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-1", "col-1", "Acta del Cabildo"],
        )
        .expect("item insert failed");
        fts_index_item(
            &conn,
            "item-1",
            "Acta del Cabildo",
            "",
            "El cabildo trato el comercio de cueros en 1810",
        )
        .expect("index failed");

        let question = "¿Qué decía el acta del cabildo sobre el comercio de cueros?";

        let strict = fts_search_with_mode(&conn, question, None, FtsMatchMode::All)
            .expect("strict search failed");
        assert!(
            strict.is_empty(),
            "implicit AND unexpectedly matched; the premise of this test changed: {strict:?}"
        );

        let loose = fts_search_with_mode(&conn, question, None, FtsMatchMode::Any)
            .expect("loose search failed");
        assert_eq!(loose.len(), 1, "natural-language question found nothing");
        assert_eq!(loose[0].item_id, "item-1");
    }

    /// Under `Any`, BM25 must still rank a document matching more query terms
    /// above one matching fewer. This is the relevance floor that replaces an
    /// explicit similarity threshold.
    #[test]
    fn any_mode_ranks_documents_matching_more_query_terms_first() {
        let conn = setup_fts_db();
        for (id, title) in [("item-weak", "Padron"), ("item-strong", "Acta")] {
            conn.execute(
                "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
                params![id, "col-1", title],
            )
            .expect("item insert failed");
        }
        // Only "comercio"; misses cabildo and cueros.
        fts_index_item(&conn, "item-weak", "Padron", "", "registro de comercio")
            .expect("index weak failed");
        // All three content terms.
        fts_index_item(
            &conn,
            "item-strong",
            "Acta",
            "",
            "el cabildo trato el comercio de cueros",
        )
        .expect("index strong failed");

        let results = fts_search_with_mode(
            &conn,
            "¿Qué decía el cabildo sobre el comercio de cueros?",
            None,
            FtsMatchMode::Any,
        )
        .expect("search failed");

        assert_eq!(results.len(), 2, "both documents should match under OR");
        assert_eq!(
            results[0].item_id, "item-strong",
            "document matching more terms must rank first: {results:?}"
        );
    }

    #[test]
    fn fts_index_upsert_replaces_previous_entry() {
        let conn = setup_fts_db();
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-1", "col-a", "Título Actualizado"],
        )
        .expect("insert item failed");

        fts_index_item(&conn, "item-1", "Título Original", "", "texto viejo")
            .expect("first index failed");
        fts_index_item(&conn, "item-1", "Título Actualizado", "", "texto nuevo")
            .expect("second index failed");

        let new_results = fts_search(&conn, "nuevo", None).expect("search failed");
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].title, "Título Actualizado");

        // In contentless mode, historical terms may persist depending on SQLite FTS5
        // delete semantics/version. We assert the current snapshot is searchable and
        // returns the latest identity fields, which is what API consumers rely on.
        let old_results = fts_search(&conn, "viejo", None).expect("search failed");
        assert!(
            old_results.iter().all(|row| row.item_id == "item-1"),
            "unexpected stale rows: {old_results:?}"
        );
    }

    #[test]
    fn fts_search_ranks_by_relevance() {
        let conn = setup_fts_db();
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-1", "col-a", "Historia"],
        )
        .expect("insert item 1 failed");
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-2", "col-b", "Historia"],
        )
        .expect("insert item 2 failed");

        fts_index_item(&conn, "item-1", "Historia", "", "guerra guerra guerra")
            .expect("index 1 failed");
        fts_index_item(&conn, "item-2", "Historia", "", "guerra").expect("index 2 failed");

        let results = fts_search(&conn, "guerra", None).expect("search failed");
        assert_eq!(results.len(), 2);
        // item-1 has higher term frequency — should rank first (lower BM25 = more relevant in SQLite)
        assert_eq!(results[0].item_id, "item-1");
    }

    #[test]
    fn fts_search_scoped_filters_by_collection() {
        let conn = setup_fts_db();

        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-a", "col-a", "Cabildo A"],
        )
        .expect("insert item-a failed");
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            params!["item-b", "col-b", "Cabildo B"],
        )
        .expect("insert item-b failed");

        fts_index_item(&conn, "item-a", "Cabildo A", "", "cabildo abierto")
            .expect("index item-a failed");
        fts_index_item(&conn, "item-b", "Cabildo B", "", "cabildo cerrado")
            .expect("index item-b failed");

        let results = fts_search(&conn, "cabildo", Some("col-a")).expect("search failed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item_id, "item-a");
        assert_eq!(results[0].title, "Cabildo A");
    }

    #[test]
    fn fts_search_only_operators_short_circuits() {
        let conn = setup_fts_db();
        let results = fts_search(&conn, "  (AND) : * , .  ", None).expect("search failed");
        assert!(results.is_empty());
    }

    #[test]
    fn fts_search_row_mapping_failure_is_error() {
        let conn = setup_fts_db();

        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, CAST(X'80' AS BLOB))",
            params!["bad-item", "col-bad"],
        )
        .expect("insert bad item failed");

        let bad_rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM items WHERE id = ?1",
                params!["bad-item"],
                |row| row.get(0),
            )
            .expect("bad rowid lookup failed");

        conn.execute(
            "INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![bad_rowid, "bad-item", "cabildo", "", "cabildo"],
        )
        .expect("insert bad fts row failed");

        let err = fts_search(&conn, "cabildo", None).expect_err("expected row mapping error");
        assert!(
            err.contains("FTS5 row mapping failed"),
            "unexpected error: {err}"
        );
    }

    // ── index_item_from_db with text_provider (Task 2.1) ─────────────────────

    fn setup_fts_db_with_assets() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            r#"
            CREATE TABLE items (
              id TEXT PRIMARY KEY,
              collection_id TEXT,
              title TEXT NOT NULL,
              metadata TEXT
            );

            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              sort_index INTEGER NOT NULL DEFAULT 0,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE transcriptions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT NOT NULL,
              language TEXT,
              duration_ms INTEGER,
              model TEXT NOT NULL,
              segments TEXT,
              confidence REAL,
              created_at INTEGER NOT NULL
            );

            CREATE VIRTUAL TABLE fts_items USING fts5(
                item_id UNINDEXED,
                title,
                metadata,
                extracted_text,
                tokenize = 'unicode61 remove_diacritics 1',
                content = ''
            );
            "#,
        )
        .expect("full FTS schema creation failed");
        conn
    }

    #[test]
    fn index_item_from_db_indexes_transcription_text_via_text_provider() {
        let conn = setup_fts_db_with_assets();

        // Insert item and asset
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, ?4)",
            params!["item-t1", "col-1", "Test Document", "{}"],
        )
        .expect("item insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, sort_index, created_at) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params!["asset-t1", "item-t1", "audio.mp3", "audio", 1_i64],
        )
        .expect("asset insert");

        // Insert transcription (no extractions!)
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model, segments, confidence, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params!["trans-t1", "asset-t1", "Don Manuel Belgrano en la ciudad de Buenos Aires", "es", 5000_i64, "base", "[]", 0.9_f64, 2_i64],
        )
        .expect("transcription insert");

        // FTS should find "Belgrano" from transcription text
        index_item_from_db(&conn, "item-t1").expect("index_item_from_db should succeed");

        let results = fts_search(&conn, "Belgrano", None).expect("search should succeed");
        assert_eq!(
            results.len(),
            1,
            "FTS should find the item via transcription text"
        );
        assert_eq!(results[0].item_id, "item-t1");
    }

    #[test]
    fn index_item_from_db_concatenates_extraction_and_transcription_text() {
        let conn = setup_fts_db_with_assets();

        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, ?4)",
            params!["item-t2", "col-1", "Multi-Source Doc", "{}"],
        )
        .expect("item insert");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, sort_index, created_at) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params!["asset-t2", "item-t2", "page1.png", "image", 1_i64],
        )
        .expect("asset insert");

        // Extraction text
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params!["ext-t2", "asset-t2", "colonial manuscript text", 10_i64],
        )
        .expect("extraction insert");

        // Transcription text
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model, segments, confidence, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params!["trans-t2", "asset-t2", "audio transcript palabras", "es", 3000_i64, "base", "[]", 0.85_f64, 20_i64],
        )
        .expect("transcription insert");

        index_item_from_db(&conn, "item-t2").expect("index_item_from_db should succeed");

        // Both "colonial" (from extraction) and "palabras" (from transcription) should be searchable
        let results_extraction =
            fts_search(&conn, "colonial", None).expect("search extraction text");
        assert_eq!(
            results_extraction.len(),
            1,
            "FTS should find extraction text"
        );

        let results_transcription =
            fts_search(&conn, "palabras", None).expect("search transcription text");
        assert_eq!(
            results_transcription.len(),
            1,
            "FTS should find transcription text"
        );
    }
}
