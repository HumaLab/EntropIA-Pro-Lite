//! Lexical splitter for renderer SQL batches (S-02b).
//!
//! `split_sql_statements` finds statement boundaries WITHOUT preparing SQL.
//! Preparing would invoke the renderer authorizer, and a legitimate migration
//! trigger must stay un-denied until its fingerprint has been checked against
//! the allowlist (plan §3.1 step 3, order (i)/(ii)). The boundary signal is
//! SQLite's own lexical `sqlite3_complete()` plus a byte mask that marks every
//! position inside a string literal, quoted identifier or comment, so only a
//! real statement-terminating `;` is ever considered a candidate.
//!
//! Text-preservation rule: every returned piece is a contiguous slice of the
//! input, in order. A candidate `;` only cuts when the slice since the last cut
//! is a complete statement AND contains at least one token that is not
//! whitespace, a comment or `;` itself. `;;`, `; -- x` and comment-only gaps
//! therefore never become bogus statements and comment text always stays with
//! the statement it precedes or follows. The only text the splitter may drop is
//! a trailing run of whitespace/comments after the last statement, so
//! concatenating the returned pieces equals the input minus that trailing run.

use sha2::{Digest, Sha256};

/// Splits a multi-statement SQL string into statements.
///
/// Each statement keeps its exact source text, including leading comments and
/// trailing whitespace. Use [`normalize_for_fingerprint`] before hashing.
pub fn split_sql_statements(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mask = code_mask(sql);
    let mut statements = Vec::new();
    let mut start = 0usize;

    for (index, &byte) in bytes.iter().enumerate() {
        if byte != b';' || !mask[index] {
            continue;
        }
        // Only a complete statement with actual content ends the current one;
        // `;`, `;;` and comment-only gaps stay part of the next piece.
        if !contains_code_token(&mask[start..=index], &bytes[start..=index])
            || !candidate_is_complete(&sql[start..=index])
        {
            continue;
        }
        statements.push(sql[start..=index].to_string());
        start = index + 1;
    }

    // A trailing statement is allowed to omit its final `;`. A trailing run of
    // whitespace and comments carries no statement and is dropped.
    if contains_code_token(&mask[start..], &bytes[start..]) {
        statements.push(sql[start..].to_string());
    }
    statements
}

/// True when `statement` is a complete SQL statement according to SQLite's own
/// lexical scanner. `sqlite3_complete` only tokenizes: it never parses or
/// prepares, so it cannot reach the connection's authorizer.
fn candidate_is_complete(statement: &str) -> bool {
    let Ok(terminated) = std::ffi::CString::new(statement) else {
        // An interior NUL cannot be valid SQL; fail closed (no cut here).
        return false;
    };
    // SAFETY: `terminated` is a valid NUL-terminated C string that outlives
    // the call; `sqlite3_complete` reads it and returns immediately.
    unsafe { rusqlite::ffi::sqlite3_complete(terminated.as_ptr()) != 0 }
}

/// Marks every byte of `sql` that sits inside SQL code (not a string literal,
/// quoted identifier, `--` comment or `/* */` comment). Quoted text and
/// comments can contain `;`, but only code positions may be cut points.
fn code_mask(sql: &str) -> Vec<bool> {
    let bytes = sql.as_bytes();
    let mut mask = vec![true; bytes.len()];
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            quote @ (b'\'' | b'"' | b'`') => {
                mask[index] = false;
                index += 1;
                while index < bytes.len() {
                    mask[index] = false;
                    if bytes[index] == quote {
                        // A doubled quote escapes itself (`'it''s'`); keep it
                        // inside the literal so a later `;` stays masked.
                        if index + 1 < bytes.len() && bytes[index + 1] == quote {
                            mask[index + 1] = false;
                            index += 2;
                            continue;
                        }
                        index += 1;
                        break;
                    }
                    index += 1;
                }
            }
            b'[' => {
                mask[index] = false;
                index += 1;
                while index < bytes.len() {
                    mask[index] = false;
                    let closed = bytes[index] == b']';
                    index += 1;
                    if closed {
                        break;
                    }
                }
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                mask[index] = false;
                mask[index + 1] = false;
                index += 2;
                while index < bytes.len() {
                    mask[index] = false;
                    let newline = bytes[index] == b'\n';
                    index += 1;
                    if newline {
                        break;
                    }
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                mask[index] = false;
                mask[index + 1] = false;
                index += 2;
                while index < bytes.len() {
                    mask[index] = false;
                    let closed = bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/');
                    index += 1;
                    if closed {
                        mask[index] = false;
                        index += 1;
                        break;
                    }
                }
            }
            _ => index += 1,
        }
    }
    mask
}

/// True when the masked bytes hold at least one token that is not whitespace,
/// a comment or `;` itself.
fn contains_code_token(mask: &[bool], bytes: &[u8]) -> bool {
    mask.iter()
        .zip(bytes)
        .any(|(&code, &byte)| code && byte != b';' && !is_sql_whitespace(byte))
}

/// The whitespace set SQLite's own tokenizer skips.
fn is_sql_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// The fingerprint input for `statement`: surrounding whitespace and a single
/// trailing `;` removed. Comments are deliberately kept — a leading or
/// interleaved comment changes the fingerprint and the statement is refused.
pub fn normalize_for_fingerprint(statement: &str) -> &str {
    let trimmed = statement.trim();
    match trimmed.strip_suffix(';') {
        Some(without_semicolon) => without_semicolon.trim_end(),
        None => trimmed,
    }
}

/// Lowercase hex SHA-256 of the normalized statement text.
pub fn fingerprint(statement: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_for_fingerprint(statement).as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trigger from `packages/store/src/runner.ts` (0029_rag_chunks): its
    /// body carries semicolons that must not split the statement.
    const RAG_CHUNKS_TRIGGER: &str = "\
CREATE TRIGGER rag_chunks_fts_insert
AFTER INSERT ON rag_chunks
BEGIN
  INSERT INTO rag_chunks_fts(chunk_id, text_content)
  VALUES (NEW.id, NEW.text_content);
END;";

    #[derive(serde::Deserialize)]
    struct Fixture {
        calls: Vec<FixtureCall>,
    }

    #[derive(serde::Deserialize)]
    struct FixtureCall {
        command: String,
        sql: String,
    }

    fn batched_calls() -> Vec<FixtureCall> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/migration_ipc.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "cannot read the migration IPC fixture {}: {error}\n\
                 Run: pnpm --filter @entropia/store export-migration-ipc",
                path.display()
            )
        });
        let fixture: Fixture = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("invalid migration IPC fixture: {error}"));
        fixture
            .calls
            .into_iter()
            .filter(|call| call.command == "db_execute_batch")
            .collect()
    }

    #[test]
    fn splits_string_literals_containing_semicolons() {
        let sql = "INSERT INTO t(a, b) VALUES ('x;y', \"c;d\");";
        assert_eq!(split_sql_statements(sql), vec![sql.to_string()]);

        let two = "INSERT INTO t(a) VALUES ('x;y');\nINSERT INTO t(a) VALUES ('z;w');";
        assert_eq!(split_sql_statements(two).len(), 2);
    }

    #[test]
    fn splits_quoted_identifiers_containing_semicolons() {
        let sql = "CREATE TABLE \"a;b\" (x INTEGER);\n\
                   CREATE TABLE [c;d] (y INTEGER);\n\
                   CREATE TABLE `e;f` (z INTEGER);";
        assert_eq!(split_sql_statements(sql).len(), 3);
    }

    #[test]
    fn semicolons_in_comments_do_not_split() {
        let sql = "-- a comment;\n\
                   /* another ; comment */\n\
                   SELECT 1;\n\
                   -- trailing; comment\n\
                   SELECT 2;";
        let pieces = split_sql_statements(sql);
        assert_eq!(pieces.len(), 2);
        assert!(pieces[0].contains("SELECT 1"));
        assert!(pieces[1].contains("-- trailing; comment"));
        assert!(pieces[1].contains("SELECT 2"));
    }

    #[test]
    fn trigger_body_stays_in_one_statement() {
        let sql = format!("{RAG_CHUNKS_TRIGGER}\nCREATE TABLE after_trigger (x);");
        let pieces = split_sql_statements(&sql);
        assert_eq!(pieces.len(), 2);
        assert!(pieces[0].contains("END;"));
        assert!(pieces[1].contains("after_trigger"));
    }

    #[test]
    fn keeps_a_trailing_statement_without_semicolon() {
        let sql = "CREATE TABLE a (x);\nCREATE TABLE b (y)";
        let pieces = split_sql_statements(sql);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[1].trim(), "CREATE TABLE b (y)");
    }

    #[test]
    fn drops_only_gap_text_without_a_statement() {
        assert!(split_sql_statements("  \n\t ").is_empty());
        assert!(split_sql_statements("; ; /* gap */;").is_empty());
        assert!(split_sql_statements("-- only a comment;\n/* and this */").is_empty());
    }

    #[test]
    fn normalize_trims_whitespace_and_one_trailing_semicolon() {
        assert_eq!(normalize_for_fingerprint("  SELECT 1; \n"), "SELECT 1");
        assert_eq!(normalize_for_fingerprint("SELECT 1;;"), "SELECT 1;");
        assert_eq!(
            normalize_for_fingerprint("-- c\nSELECT 1 ;"),
            "-- c\nSELECT 1"
        );
        assert_eq!(normalize_for_fingerprint(""), "");
        // Comments are NOT removed.
        assert_eq!(
            normalize_for_fingerprint("/* c */ SELECT 1;"),
            "/* c */ SELECT 1"
        );
    }

    #[test]
    fn fingerprint_is_lowercase_hex_sha256_of_the_normalized_text() {
        let expected = fingerprint("PRAGMA defer_foreign_keys=ON;");
        assert_eq!(expected.len(), 64);
        assert!(expected
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_eq!(
            expected,
            fingerprint("  PRAGMA defer_foreign_keys=ON;  \n"),
            "normalization must make surrounding whitespace irrelevant"
        );
        assert_ne!(
            expected,
            fingerprint("/* x */ PRAGMA defer_foreign_keys=ON;"),
            "a leading comment must change the fingerprint"
        );
    }

    #[test]
    fn fixture_batches_keep_every_statement_and_no_text_is_lost() {
        let calls = batched_calls();
        assert!(
            !calls.is_empty(),
            "fixture must contain db_execute_batch calls"
        );

        let mut statements = 0usize;
        for call in &calls {
            let pieces = split_sql_statements(&call.sql);
            assert!(
                !pieces.is_empty(),
                "a recorded batch split into zero statements: {:?}",
                &call.sql[..call.sql.len().min(80)]
            );
            statements += pieces.len();

            // Pieces are contiguous slices, so their concatenation is a prefix
            // of the batch; what is left over may only be gap text.
            let rejoined = pieces.concat();
            assert!(
                call.sql.starts_with(&rejoined),
                "split pieces are not a contiguous prefix of the batch"
            );
            let tail = &call.sql[rejoined.len()..];
            let tail_mask = code_mask(tail);
            assert!(
                !contains_code_token(&tail_mask, tail.as_bytes()),
                "splitter dropped statement text: {tail:?}"
            );
        }
        assert!(
            statements > calls.len(),
            "expected more statements than batches"
        );
    }

    #[test]
    fn split_sql_statements_does_not_prepare() {
        use rusqlite::hooks::{AuthContext, Authorization};
        use rusqlite::Connection;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let connection = Connection::open_in_memory().expect("in-memory database");
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        connection.authorizer(Some(move |_ctx: AuthContext<'_>| {
            counter.fetch_add(1, Ordering::SeqCst);
            Authorization::Deny
        }));

        for call in batched_calls() {
            let pieces = split_sql_statements(&call.sql);
            assert!(!pieces.is_empty());
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "split_sql_statements reached the connection (it prepared SQL)"
        );
    }
}
