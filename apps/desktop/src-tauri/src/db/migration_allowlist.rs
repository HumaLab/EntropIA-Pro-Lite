//! Committed allowlist of the migration DDL statements the renderer may run
//! (S-02b; consumed by the S-02c migration window).
//!
//! SQLite invokes the authorizer while a statement is prepared, so a legitimate
//! `CREATE TRIGGER` from the TypeScript migration runner must be recognized
//! before the prepare happens. Recognition is by exact fingerprint: the SHA-256
//! of the statement text with surrounding whitespace and one trailing `;`
//! removed (comments kept, see [`crate::db::sql_split`]). This module loads the
//! committed fingerprints and re-derives the set from the recorded migration
//! IPC fixture, so editing `runner.ts` without regenerating the list fails the
//! consistency test. The committed file also carries the `# last-migration:`
//! directive the backend uses to auto-close the one-shot window.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::db::sql_split::fingerprint;

/// The DDL kinds the migration allowlist covers. Every other statement (tables,
/// indexes, DML, transactions) is handled by the ordinary authorizer rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DdlKind {
    CreateTrigger,
    CreateTempTrigger,
    CreateView,
    CreateTempView,
    DropTrigger,
    DropView,
    Pragma,
}

/// Classifies a statement by the first keywords after leading whitespace and
/// comments. The fingerprint itself is taken over the un-stripped text, so a
/// leading comment does not change the classification but does change the hash.
pub fn classify(statement: &str) -> Option<DdlKind> {
    let words = leading_keywords(statement, 4);
    match words.first().map(String::as_str)? {
        "pragma" => Some(DdlKind::Pragma),
        "drop" => match words.get(1).map(String::as_str) {
            Some("trigger") => Some(DdlKind::DropTrigger),
            Some("view") => Some(DdlKind::DropView),
            _ => None,
        },
        "create" => {
            let (temp, index) = match words.get(1).map(String::as_str) {
                Some("temp" | "temporary") => (true, 2),
                _ => (false, 1),
            };
            match (words.get(index).map(String::as_str), temp) {
                (Some("trigger"), true) => Some(DdlKind::CreateTempTrigger),
                (Some("trigger"), false) => Some(DdlKind::CreateTrigger),
                (Some("view"), true) => Some(DdlKind::CreateTempView),
                (Some("view"), false) => Some(DdlKind::CreateView),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The first `limit` alphabetic word tokens of a statement, skipping
/// whitespace and `--`/`/* */` comments.
fn leading_keywords(statement: &str, limit: usize) -> Vec<String> {
    let bytes = statement.as_bytes();
    let mut words = Vec::new();
    let mut index = 0usize;

    while index < bytes.len() && words.len() < limit {
        match bytes[index] {
            byte if byte.is_ascii_alphabetic() => {
                let start = index;
                while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                    index += 1;
                }
                words.push(statement[start..index].to_ascii_lowercase());
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index < bytes.len()
                    && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            _ => index += 1,
        }
    }
    words
}

/// A `db_*` call recorded by `packages/store/src/migration-ipc-recorder.ts`.
/// Test-only: production code never parses the fixture.
#[cfg(test)]
#[derive(Debug, serde::Deserialize)]
pub struct RecordedMigrationCall {
    pub command: String,
    pub sql: String,
    #[serde(default)]
    pub params: Vec<serde_json::Value>,
}

/// The recorded migration IPC fixture (fresh install + 0032 repair drops).
#[cfg(test)]
#[derive(Debug, serde::Deserialize)]
pub struct MigrationIpcFixture {
    pub calls: Vec<RecordedMigrationCall>,
    #[serde(default)]
    pub repair_drops: Vec<String>,
}

/// The fingerprint of every `CREATE`/`DROP TRIGGER`, `CREATE`/`DROP VIEW` and
/// `PRAGMA` statement reached by splitting the fixture's `db_execute_batch`
/// SQL (plus the repair drops), sorted and unique.
#[cfg(test)]
pub fn collect_ddl_fingerprints(fixture: &MigrationIpcFixture) -> BTreeSet<String> {
    let mut fingerprints = BTreeSet::new();
    for call in &fixture.calls {
        if call.command == "db_execute_batch" {
            collect_from_sql(&call.sql, &mut fingerprints);
        }
    }
    for repair_drop in &fixture.repair_drops {
        collect_from_sql(repair_drop, &mut fingerprints);
    }
    fingerprints
}

#[cfg(test)]
fn collect_from_sql(sql: &str, fingerprints: &mut BTreeSet<String>) {
    for statement in crate::db::sql_split::split_sql_statements(sql) {
        if classify(&statement).is_some() {
            fingerprints.insert(fingerprint(&statement));
        }
    }
}

/// The name of the last migration the fixture records in `_migrations`
/// (fresh install: `0058_processing_ner_tasks`). Read from the recorded IPC
/// calls, never hardcoded, so the directive in the committed allowlist tracks
/// `runner.ts`. Migration batches insert the name inline; the remaining
/// migrations insert it through `db_execute` with the name as `params[0]`.
#[cfg(test)]
pub fn last_recorded_migration(fixture: &MigrationIpcFixture) -> Option<String> {
    let mut last = None;
    for call in &fixture.calls {
        match call.command.as_str() {
            "db_execute" if call.sql.contains("INSERT INTO _migrations") => {
                if let Some(serde_json::Value::String(name)) = call.params.first() {
                    last = Some(name.clone());
                }
            }
            "db_execute_batch" => {
                if let Some(name) = batch_migration_insert_name(&call.sql) {
                    last = Some(name);
                }
            }
            _ => {}
        }
    }
    last
}

/// The name of the LAST migration row inserted inline by a batch, if any.
#[cfg(test)]
fn batch_migration_insert_name(sql: &str) -> Option<String> {
    const MARKER: &str = "INSERT INTO _migrations (name, applied_at) VALUES ('";
    let start = sql.rfind(MARKER)? + MARKER.len();
    let rest = &sql[start..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

/// The object name a listed DDL statement declares: the trigger or view name
/// for CREATE/DROP, the pragma name for PRAGMA. Lowercased, without quotes or
/// a schema qualifier. `None` when the leading tokens do not name one — the
/// fingerprint check stays the authority, this only narrows the exception.
pub fn declared_object(statement: &str) -> Option<String> {
    let tokens = name_tokens(statement);
    match tokens.first().map(String::as_str)? {
        "pragma" => qualified_name_at(&tokens, 1),
        "drop" => {
            let mut index = 1;
            if !matches!(
                tokens.get(index).map(String::as_str),
                Some("trigger" | "view")
            ) {
                return None;
            }
            index += 1;
            skip_if(&tokens, &mut index, &["if", "exists"]);
            qualified_name_at(&tokens, index)
        }
        "create" => {
            let mut index = 1;
            if matches!(
                tokens.get(index).map(String::as_str),
                Some("temp" | "temporary")
            ) {
                index += 1;
            }
            if !matches!(
                tokens.get(index).map(String::as_str),
                Some("trigger" | "view")
            ) {
                return None;
            }
            index += 1;
            skip_if(&tokens, &mut index, &["if", "not", "exists"]);
            qualified_name_at(&tokens, index)
        }
        _ => None,
    }
}

/// The table a `DROP TABLE` or `ALTER TABLE` statement targets, if any.
/// SQLite drops the triggers attached to a table it is itself dropping while
/// rebuilding it, so the executor scopes exactly that internal drop to this
/// table name (`DROP TABLE processing_tasks` must not fail because of its own
/// `processing_tasks_settle_dependents` trigger).
pub fn declared_table(statement: &str) -> Option<String> {
    let tokens = name_tokens(statement);
    match tokens.first().map(String::as_str)? {
        "drop" => {
            let mut index = 1;
            if tokens.get(index).map(String::as_str) != Some("table") {
                return None;
            }
            index += 1;
            skip_if(&tokens, &mut index, &["if", "exists"]);
            qualified_name_at(&tokens, index)
        }
        "alter" => {
            let index = 1;
            if tokens.get(index).map(String::as_str) != Some("table") {
                return None;
            }
            qualified_name_at(&tokens, index + 1)
        }
        _ => None,
    }
}

/// Advances `index` past the fixed keyword run when it is present.
fn skip_if(tokens: &[String], index: &mut usize, keywords: &[&str]) {
    if tokens
        .iter()
        .skip(*index)
        .zip(keywords)
        .all(|(token, keyword)| token == keyword)
    {
        *index += keywords.len();
    }
}

/// The (possibly `schema.`)`name` at `index`, returning the bare name.
fn qualified_name_at(tokens: &[String], index: usize) -> Option<String> {
    let mut name = tokens.get(index)?;
    if name == "." {
        return None;
    }
    let mut next = index + 1;
    while tokens.get(next).map(String::as_str) == Some(".") {
        match tokens.get(next + 1) {
            Some(part) if part != "." => {
                name = part;
                next += 2;
            }
            _ => break,
        }
    }
    Some(name.clone())
}

/// Lowercased name tokens of a statement, skipping whitespace and comments.
/// Bare words, `"…"`/`` `…` ``/`[…]` quoted identifiers and `.` separators
/// become tokens; everything else (punctuation, operators, string literals)
/// is skipped.
fn name_tokens(statement: &str) -> Vec<String> {
    let bytes = statement.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index < bytes.len()
                    && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'.' => {
                tokens.push(".".to_string());
                index += 1;
            }
            quote @ (b'"' | b'`') => {
                index += 1;
                let mut name = String::new();
                while index < bytes.len() {
                    if bytes[index] == quote {
                        if bytes.get(index + 1) == Some(&quote) {
                            name.push(quote as char);
                            index += 2;
                            continue;
                        }
                        index += 1;
                        break;
                    }
                    let Some(ch) = statement[index..].chars().next() else {
                        break;
                    };
                    name.push(ch);
                    index += ch.len_utf8();
                }
                tokens.push(name.to_ascii_lowercase());
            }
            b'[' => {
                index += 1;
                let start = index;
                while index < bytes.len() && bytes[index] != b']' {
                    index += 1;
                }
                tokens.push(statement[start..index].to_ascii_lowercase());
                index = (index + 1).min(bytes.len());
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' => {
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric()
                        || bytes[index] == b'_'
                        || bytes[index] == b'$')
                {
                    index += 1;
                }
                tokens.push(statement[start..index].to_ascii_lowercase());
            }
            _ => index += 1,
        }
    }
    tokens
}

/// The committed allowlist, parsed once from `migration_ddl_allowlist.txt`.
pub struct MigrationDdlAllowlist {
    fingerprints: BTreeSet<String>,
    last_migration: Option<String>,
}

impl MigrationDdlAllowlist {
    /// Parses the list. `#` comment lines and blanks are skipped except the
    /// `# last-migration: <name>` directive; every other non-comment line must
    /// be one lowercase 64-character hex SHA-256. A malformed line panics: a
    /// silently shrinking allowlist would not fail safe.
    pub fn from_text(text: &str) -> Self {
        let mut fingerprints = BTreeSet::new();
        let mut last_migration = None;
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix("# last-migration:") {
                let name = name.trim();
                assert!(
                    !name.is_empty(),
                    "empty last-migration directive on line {} of migration_ddl_allowlist.txt",
                    index + 1
                );
                last_migration = Some(name.to_string());
                continue;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            assert!(
                line.len() == 64
                    && line
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "invalid fingerprint on line {} of migration_ddl_allowlist.txt: {line:?}",
                index + 1
            );
            fingerprints.insert(line.to_string());
        }
        Self {
            fingerprints,
            last_migration,
        }
    }

    /// The allowlist compiled into the binary.
    pub fn embedded() -> &'static Self {
        static EMBEDDED: OnceLock<MigrationDdlAllowlist> = OnceLock::new();
        EMBEDDED.get_or_init(|| Self::from_text(include_str!("migration_ddl_allowlist.txt")))
    }

    /// True when `fingerprint` (any hex case) is a listed migration statement.
    pub fn contains(&self, fingerprint: &str) -> bool {
        self.fingerprints
            .contains(&fingerprint.to_ascii_lowercase())
    }

    /// True when `statement` is one of the listed migration DDL statements:
    /// a `CREATE`/`DROP TRIGGER`, `CREATE`/`DROP VIEW` or `PRAGMA` whose exact
    /// fingerprint is committed. Everything else is not covered by the
    /// exception, whatever the migration window or the command kind.
    pub fn allows_statement(&self, statement: &str) -> bool {
        classify(statement).is_some() && self.contains(&fingerprint(statement))
    }

    /// The last migration name the committed list was generated for; the
    /// backend closes the one-shot window once `_migrations` holds it.
    pub fn last_migration(&self) -> Option<&str> {
        self.last_migration.as_deref()
    }

    /// The listed fingerprint count (tests assemble and compare the list).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.fingerprints.len()
    }
}

/// Header of the committed file, including the regeneration recipe.
#[cfg(test)]
pub const ALLOWLIST_HEADER: &str = "\
# SHA-256 fingerprints (lowercase hex) of every CREATE/DROP TRIGGER, CREATE/DROP VIEW
# and PRAGMA statement emitted by the TypeScript migration runner, plus the 0032
# partial-state repair DROP TRIGGER statements.
#
# The fingerprinted text is the split statement with surrounding whitespace and one
# trailing ';' removed. Comments are NOT stripped: a leading or interleaved comment
# changes the fingerprint and the statement is refused.
#
# The `# last-migration:` directive below is the last `_migrations` row the
# recorded fixture inserts; the backend closes the one-shot migration window
# once the database holds it.
#
# Regenerate after any runner.ts change:
#   pnpm --filter @entropia/store export-migration-ipc
#   cargo test --lib regenerate_migration_ddl_allowlist -- --ignored
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sql_split::split_sql_statements;
    use std::path::{Path, PathBuf};

    fn fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migration_ipc.json")
    }

    fn load_fixture() -> MigrationIpcFixture {
        let path = fixture_path();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "cannot read the migration IPC fixture {}: {error}\n\
                 Run: pnpm --filter @entropia/store export-migration-ipc",
                path.display()
            )
        });
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("invalid migration IPC fixture: {error}"))
    }

    fn committed_lines() -> Vec<String> {
        include_str!("migration_ddl_allowlist.txt")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn classifies_only_the_allowlisted_ddl_kinds() {
        assert_eq!(
            classify("CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END;"),
            Some(DdlKind::CreateTrigger)
        );
        assert_eq!(
            classify("/* c */ create TEMPORARY trigger t AFTER INSERT ON x BEGIN SELECT 1; END;"),
            Some(DdlKind::CreateTempTrigger)
        );
        assert_eq!(
            classify("CREATE VIEW v AS SELECT 1;"),
            Some(DdlKind::CreateView)
        );
        assert_eq!(
            classify("-- c\nCREATE TEMP VIEW v AS SELECT 1;"),
            Some(DdlKind::CreateTempView)
        );
        assert_eq!(
            classify("DROP TRIGGER IF EXISTS trg_x;"),
            Some(DdlKind::DropTrigger)
        );
        assert_eq!(classify("drop view if exists v;"), Some(DdlKind::DropView));
        assert_eq!(
            classify("PRAGMA defer_foreign_keys=ON;"),
            Some(DdlKind::Pragma)
        );

        assert_eq!(classify("CREATE TABLE t (x);"), None);
        assert_eq!(classify("CREATE INDEX i ON t(x);"), None);
        assert_eq!(classify("CREATE VIRTUAL TABLE v USING fts5(x);"), None);
        assert_eq!(classify("DROP TABLE t;"), None);
        assert_eq!(classify("ALTER TABLE t ADD COLUMN x;"), None);
        assert_eq!(classify("INSERT INTO t VALUES (1);"), None);
    }

    #[test]
    fn loader_skips_comments_and_parses_the_last_migration_directive() {
        let known = "0".repeat(64);
        let list = MigrationDdlAllowlist::from_text(&format!(
            "# header comment\n# last-migration: 0058_processing_ner_tasks\n\n{known}\n# trailing comment\n"
        ));
        assert_eq!(list.len(), 1);
        assert!(list.contains(&known));
        assert!(list.contains(&known.to_ascii_uppercase()));
        assert!(!list.contains(&"f".repeat(64)));
        assert_eq!(list.last_migration(), Some("0058_processing_ner_tasks"));
    }

    #[test]
    #[should_panic(expected = "empty last-migration directive")]
    fn loader_rejects_an_empty_last_migration_directive() {
        MigrationDdlAllowlist::from_text(&format!("# last-migration:   \n{}\n", "0".repeat(64)));
    }

    #[test]
    fn allows_statement_covers_only_listed_ddl_fingerprints() {
        let fixture = load_fixture();
        let allowlist = MigrationDdlAllowlist::from_text(&format!(
            "# last-migration: {}\n{}\n",
            last_recorded_migration(&fixture).expect("fixture records migrations"),
            fingerprint("PRAGMA defer_foreign_keys=ON;"),
        ));
        assert!(allowlist.allows_statement("PRAGMA defer_foreign_keys=ON;"));
        assert!(allowlist.allows_statement("  PRAGMA defer_foreign_keys=ON;  \n"));
        // A comment changes the fingerprint, a different statement never matches,
        // and non-DDL statements are never covered by the exception.
        assert!(!allowlist.allows_statement("/* x */ PRAGMA defer_foreign_keys=ON;"));
        assert!(!allowlist.allows_statement("PRAGMA writable_schema=ON;"));
        assert!(
            !allowlist.allows_statement("CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END;")
        );
    }

    #[test]
    fn declared_object_reads_the_trigger_view_and_pragma_names() {
        assert_eq!(
            declared_object("CREATE TRIGGER rag_chunks_fts_insert AFTER INSERT ON rag_chunks BEGIN SELECT 1; END;"),
            Some("rag_chunks_fts_insert".to_string())
        );
        assert_eq!(
            declared_object("CREATE TRIGGER IF NOT EXISTS processing_tasks_settle_dependents AFTER UPDATE ON t BEGIN SELECT 1; END;"),
            Some("processing_tasks_settle_dependents".to_string())
        );
        assert_eq!(
            declared_object("/* c */ create TEMPORARY view \"v_quoted\" AS SELECT 1;"),
            Some("v_quoted".to_string())
        );
        assert_eq!(
            declared_object("DROP TRIGGER IF EXISTS trg_processing_extractions_ai;"),
            Some("trg_processing_extractions_ai".to_string())
        );
        assert_eq!(
            declared_object("drop view main.my_view;"),
            Some("my_view".to_string())
        );
        assert_eq!(
            declared_object("PRAGMA defer_foreign_keys=ON;"),
            Some("defer_foreign_keys".to_string())
        );
        assert_eq!(
            declared_object("PRAGMA main.defer_foreign_keys=ON;"),
            Some("defer_foreign_keys".to_string())
        );
        assert_eq!(declared_object("CREATE TABLE t (x);"), None);
    }

    #[test]
    fn declared_table_reads_drop_and_alter_targets() {
        assert_eq!(
            declared_table("DROP TABLE processing_tasks;"),
            Some("processing_tasks".to_string())
        );
        assert_eq!(
            declared_table("DROP TABLE IF EXISTS main.\"processing_tasks\";"),
            Some("processing_tasks".to_string())
        );
        assert_eq!(
            declared_table("ALTER TABLE processing_tasks ADD COLUMN x TEXT;"),
            Some("processing_tasks".to_string())
        );
        assert_eq!(declared_table("CREATE TABLE t (x);"), None);
        assert_eq!(declared_table("DROP TRIGGER trg; "), None);
    }

    #[test]
    fn last_recorded_migration_comes_from_the_fixture_order() {
        let fixture = load_fixture();
        assert_eq!(
            last_recorded_migration(&fixture).as_deref(),
            Some("0058_processing_ner_tasks")
        );
    }

    #[test]
    #[should_panic(expected = "invalid fingerprint")]
    fn loader_rejects_malformed_lines() {
        MigrationDdlAllowlist::from_text("not-a-fingerprint\n");
    }

    #[test]
    fn embedded_allowlist_matches_the_recorded_migration_ipc() {
        let expected: Vec<String> = collect_ddl_fingerprints(&load_fixture())
            .into_iter()
            .collect();
        let actual = committed_lines();

        assert_eq!(
            actual, expected,
            "the committed allowlist is stale; run `pnpm --filter @entropia/store \
             export-migration-ipc` and then \
             `cargo test --lib regenerate_migration_ddl_allowlist -- --ignored`"
        );

        let embedded = MigrationDdlAllowlist::embedded();
        assert_eq!(embedded.len(), expected.len());
        for fingerprint in &expected {
            assert!(
                embedded.contains(fingerprint),
                "embedded allowlist lost {fingerprint}"
            );
        }

        // The directive the window auto-close reads must be the last migration
        // the fixture records, not a hand-kept name.
        let expected_last = last_recorded_migration(&load_fixture())
            .expect("fixture must record at least one _migrations insert");
        assert_eq!(
            embedded.last_migration(),
            Some(expected_last.as_str()),
            "the committed # last-migration directive drifted from the fixture"
        );
    }

    #[test]
    fn fixture_ddl_coverage_covers_triggers_pragmas_and_repair_drops() {
        let fixture = load_fixture();
        let mut statements = 0usize;
        let mut kinds: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();

        let mut count = |sql: &str, statements: &mut usize| {
            for statement in split_sql_statements(sql) {
                *statements += 1;
                if let Some(kind) = classify(&statement) {
                    *kinds.entry(format!("{kind:?}")).or_default() += 1;
                }
            }
        };
        for call in &fixture.calls {
            if call.command == "db_execute_batch" {
                count(&call.sql, &mut statements);
            }
        }
        for repair_drop in &fixture.repair_drops {
            count(repair_drop, &mut statements);
        }

        eprintln!("fixture statements: {statements}, allowlisted kinds: {kinds:?}");
        assert!(statements > 0);
        assert_eq!(kinds.get("Pragma").copied(), Some(5));
        assert!(kinds.get("CreateTrigger").copied().unwrap_or(0) > 0);
        assert!(kinds.get("DropTrigger").copied().unwrap_or(0) >= 6);
        assert_eq!(kinds.get("CreateView").copied().unwrap_or(0), 0);
    }

    #[test]
    #[ignore = "regenerates the committed allowlist; run after export-migration-ipc"]
    fn regenerate_migration_ddl_allowlist() {
        let fixture = load_fixture();
        let fingerprints = collect_ddl_fingerprints(&fixture);
        let last_migration = last_recorded_migration(&fixture)
            .expect("fixture must record at least one _migrations insert");
        let mut content = String::from(ALLOWLIST_HEADER);
        content.push_str(&format!("# last-migration: {last_migration}\n"));
        for fingerprint in &fingerprints {
            content.push_str(fingerprint);
            content.push('\n');
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db/migration_ddl_allowlist.txt");
        std::fs::write(&path, content)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
        eprintln!(
            "wrote {} fingerprints (last migration {last_migration}) to {}",
            fingerprints.len(),
            path.display()
        );
    }
}
