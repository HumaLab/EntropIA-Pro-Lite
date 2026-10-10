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
//! consistency test.
#![allow(dead_code)] // S-02c wires this into `db_execute_batch`; kept compiled until then.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::db::sql_split::{fingerprint, split_sql_statements};

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
#[derive(Debug, serde::Deserialize)]
pub struct RecordedMigrationCall {
    pub command: String,
    pub sql: String,
    #[serde(default)]
    pub params: Vec<serde_json::Value>,
}

/// The recorded migration IPC fixture (fresh install + 0032 repair drops).
#[derive(Debug, serde::Deserialize)]
pub struct MigrationIpcFixture {
    pub calls: Vec<RecordedMigrationCall>,
    #[serde(default)]
    pub repair_drops: Vec<String>,
}

/// The fingerprint of every `CREATE`/`DROP TRIGGER`, `CREATE`/`DROP VIEW` and
/// `PRAGMA` statement reached by splitting the fixture's `db_execute_batch`
/// SQL (plus the repair drops), sorted and unique.
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

fn collect_from_sql(sql: &str, fingerprints: &mut BTreeSet<String>) {
    for statement in split_sql_statements(sql) {
        if classify(&statement).is_some() {
            fingerprints.insert(fingerprint(&statement));
        }
    }
}

/// The committed allowlist, parsed once from `migration_ddl_allowlist.txt`.
pub struct MigrationDdlAllowlist {
    fingerprints: BTreeSet<String>,
}

impl MigrationDdlAllowlist {
    /// Parses the list. `#` comment lines and blanks are skipped; every other
    /// line must be one lowercase 64-character hex SHA-256. A malformed line
    /// panics: a silently shrinking allowlist would not fail safe.
    pub fn from_text(text: &str) -> Self {
        let mut fingerprints = BTreeSet::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
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
        Self { fingerprints }
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

    /// The listed fingerprints in sorted order.
    pub fn fingerprints(&self) -> impl Iterator<Item = &str> {
        self.fingerprints.iter().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.fingerprints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fingerprints.is_empty()
    }
}

/// Header of the committed file, including the regeneration recipe.
pub const ALLOWLIST_HEADER: &str = "\
# SHA-256 fingerprints (lowercase hex) of every CREATE/DROP TRIGGER, CREATE/DROP VIEW
# and PRAGMA statement emitted by the TypeScript migration runner, plus the 0032
# partial-state repair DROP TRIGGER statements.
#
# The fingerprinted text is the split statement with surrounding whitespace and one
# trailing ';' removed. Comments are NOT stripped: a leading or interleaved comment
# changes the fingerprint and the statement is refused.
#
# Regenerate after any runner.ts change:
#   pnpm --filter @entropia/store export-migration-ipc
#   cargo test --lib regenerate_migration_ddl_allowlist -- --ignored
";

#[cfg(test)]
mod tests {
    use super::*;
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
    fn loader_skips_comments_and_rejects_unknown_fingerprints() {
        let known = "0".repeat(64);
        let list = MigrationDdlAllowlist::from_text(&format!(
            "# header comment\n\n{known}\n# trailing comment\n"
        ));
        assert_eq!(list.len(), 1);
        assert!(list.contains(&known));
        assert!(list.contains(&known.to_ascii_uppercase()));
        assert!(!list.contains(&"f".repeat(64)));
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
        let fingerprints = collect_ddl_fingerprints(&load_fixture());
        let mut content = String::from(ALLOWLIST_HEADER);
        for fingerprint in &fingerprints {
            content.push_str(fingerprint);
            content.push('\n');
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db/migration_ddl_allowlist.txt");
        std::fs::write(&path, content)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
        eprintln!(
            "wrote {} fingerprints to {}",
            fingerprints.len(),
            path.display()
        );
    }
}
