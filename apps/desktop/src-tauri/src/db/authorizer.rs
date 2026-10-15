//! Layer 2 of the renderer SQL defence: a SQLite authorizer installed on the
//! UI connection only while a renderer `db_*` command runs, through an RAII
//! guard that always removes it on drop (including on error and panic).
//!
//! Layer 1 (the text validators in `commands.rs`) still runs first and keeps
//! its clear messages; this module is the semantic backstop SQLite enforces
//! itself while statements are prepared and while trigger bodies execute.
//! [`authorize`] is a pure function so the whole policy table is unit-testable.
//!
//! Since Phase 4.4a-2 the renderer only reads and writes rows: every schema
//! statement (CREATE/DROP/ALTER of tables, indexes, triggers, views and
//! virtual tables, plus the schema-maintenance REINDEX/ANALYZE) is denied for
//! every command kind. Migrations run in Rust during backend setup, before the
//! webview loads, so there is no migration window, no fingerprint allowlist
//! and no statement-scoped exception left in the policy.

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::Connection;

/// The FTS5 constructor's pragma, needed by every renderer statement: FTS5
/// queries `PRAGMA data_version` when it connects a virtual table on a
/// connection (the first use, or the first after a schema reset). It takes no
/// value and only reads a counter, so it is the single pragma the renderer may
/// issue.
const FTS5_CONSTRUCTOR_PRAGMA: &str = "data_version";

/// The layer-2 policy, kept pure so every rule is exercised by unit tests.
///
/// Returning [`Authorization::Deny`] (never `Ignore`) makes SQLite reject the
/// statement with an authorization error instead of silently substituting
/// NULL or a default.
///
/// | Action | Verdict |
/// |---|---|
/// | ATTACH / DETACH (incl. `VACUUM`/`VACUUM INTO`) | Deny |
/// | value-less `PRAGMA data_version` | Allow |
/// | any other PRAGMA (values included; `quick_check`, `defer_foreign_keys`) | Deny |
/// | `app_settings` read/write/DDL | Deny |
/// | write `sync_*` unless accessor is `trg_sync_*`; any DDL on `sync_*`/`trg_sync_*` | Deny |
/// | CREATE/DROP TABLE/INDEX/TRIGGER/VIEW/VIRTUAL TABLE (incl. TEMP_*) | Deny |
/// | ALTER TABLE | Deny |
/// | REINDEX / ANALYZE | Deny |
/// | everything else (SELECT, DML on ordinary tables, transactions, functions) | Allow |
pub(crate) fn authorize(ctx: &AuthContext<'_>) -> Authorization {
    match ctx.action {
        // `VACUUM INTO` is implemented as an ATTACH of the target file, so this
        // is what makes it fail. Plain `VACUUM` also goes through the same
        // SQLITE_ATTACH check (with a NULL filename, which rusqlite can only
        // surface as `Unknown`), and is denied here too.
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
        AuthAction::Unknown { code, .. }
            if code == rusqlite::ffi::SQLITE_ATTACH || code == rusqlite::ffi::SQLITE_DETACH =>
        {
            Authorization::Deny
        }

        // Every pragma is denied, `quick_check` and `defer_foreign_keys`
        // included, except the value-less `data_version` the FTS5 constructor
        // needs. Setting a value on `data_version` is denied too.
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => {
            let fts5_constructor =
                pragma_name.eq_ignore_ascii_case(FTS5_CONSTRUCTOR_PRAGMA) && pragma_value.is_none();
            if fts5_constructor {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }

        // `app_settings` holds secrets; no renderer statement may touch it,
        // directly or through a view/trigger.
        _ if action_table(&ctx.action).is_some_and(is_sensitive_table) => Authorization::Deny,

        // Sync bookkeeping is engine-owned (DESIGN §6.2). The capture triggers
        // are the one legitimate writer of `sync_oplog`: SQLite reports their
        // body with the trigger as `accessor`.
        _ if touches_sync_objects(&ctx.action, ctx.accessor) => Authorization::Deny,

        // Schema DDL is backend-owned since Phase 4.4a-2: the renderer has no
        // legitimate CREATE/DROP/ALTER, and layer 1 rejects it textually before
        // SQLite parses it. `AuthAction::CreateVtable`/`DropVtable` cover
        // `CREATE/DROP VIRTUAL TABLE`; the shadow tables FTS5 creates report as
        // ordinary `CreateTable`/`CreateIndex` and are denied with them.
        //
        // REINDEX and ANALYZE are denied alongside: neither is SQL DDL, but
        // both mutate schema-owned state (index b-trees and the `sqlite_stat*`
        // tables) and no renderer feature needs them, so leaving them out would
        // be a hole in an otherwise closed schema surface.
        AuthAction::CreateIndex { .. }
        | AuthAction::CreateTable { .. }
        | AuthAction::CreateTempIndex { .. }
        | AuthAction::CreateTempTable { .. }
        | AuthAction::CreateTempTrigger { .. }
        | AuthAction::CreateTempView { .. }
        | AuthAction::CreateTrigger { .. }
        | AuthAction::CreateView { .. }
        | AuthAction::CreateVtable { .. }
        | AuthAction::DropIndex { .. }
        | AuthAction::DropTable { .. }
        | AuthAction::DropTempIndex { .. }
        | AuthAction::DropTempTable { .. }
        | AuthAction::DropTempTrigger { .. }
        | AuthAction::DropTempView { .. }
        | AuthAction::DropTrigger { .. }
        | AuthAction::DropView { .. }
        | AuthAction::DropVtable { .. }
        | AuthAction::AlterTable { .. }
        | AuthAction::Reindex { .. }
        | AuthAction::Analyze { .. } => Authorization::Deny,

        // Ordinary DML, SELECT (including sqlite_master), transactions,
        // functions and recursive CTEs.
        _ => Authorization::Allow,
    }
}

/// True when a statement under the authorizer touches the sync engine's
/// bookkeeping: any DML on a `sync_*` table (unless the innermost accessor is
/// one of the capture triggers) or DDL over a `sync_*` table / `trg_sync_*`
/// trigger.
fn touches_sync_objects(action: &AuthAction<'_>, accessor: Option<&str>) -> bool {
    match action {
        AuthAction::Insert { table_name }
        | AuthAction::Update { table_name, .. }
        | AuthAction::Delete { table_name } => {
            is_sync_object(table_name, "sync_") && !is_capture_trigger(accessor)
        }
        AuthAction::CreateTable { table_name }
        | AuthAction::CreateTempTable { table_name }
        | AuthAction::DropTable { table_name }
        | AuthAction::DropTempTable { table_name }
        | AuthAction::AlterTable { table_name, .. } => is_sync_object(table_name, "sync_"),
        AuthAction::CreateIndex { table_name, .. }
        | AuthAction::CreateTempIndex { table_name, .. }
        | AuthAction::DropIndex { table_name, .. }
        | AuthAction::DropTempIndex { table_name, .. } => is_sync_object(table_name, "sync_"),
        AuthAction::CreateTrigger {
            trigger_name,
            table_name,
        }
        | AuthAction::CreateTempTrigger {
            trigger_name,
            table_name,
        }
        | AuthAction::DropTrigger {
            trigger_name,
            table_name,
        }
        | AuthAction::DropTempTrigger {
            trigger_name,
            table_name,
        } => is_sync_object(trigger_name, "trg_sync_") || is_sync_object(table_name, "sync_"),
        _ => false,
    }
}

/// The table name an action targets, for actions that name one.
fn action_table<'c>(action: &AuthAction<'c>) -> Option<&'c str> {
    match action {
        AuthAction::CreateIndex { table_name, .. }
        | AuthAction::CreateTable { table_name }
        | AuthAction::CreateTempIndex { table_name, .. }
        | AuthAction::CreateTempTable { table_name }
        | AuthAction::CreateTempTrigger { table_name, .. }
        | AuthAction::CreateTrigger { table_name, .. }
        | AuthAction::Delete { table_name }
        | AuthAction::DropIndex { table_name, .. }
        | AuthAction::DropTable { table_name }
        | AuthAction::DropTempIndex { table_name, .. }
        | AuthAction::DropTempTable { table_name }
        | AuthAction::DropTempTrigger { table_name, .. }
        | AuthAction::DropTrigger { table_name, .. }
        | AuthAction::Insert { table_name }
        | AuthAction::Read { table_name, .. }
        | AuthAction::Update { table_name, .. }
        | AuthAction::AlterTable { table_name, .. }
        | AuthAction::Analyze { table_name }
        | AuthAction::CreateVtable { table_name, .. }
        | AuthAction::DropVtable { table_name, .. } => Some(table_name),
        _ => None,
    }
}

/// True when a bare or schema-qualified object name has `prefix`,
/// case-insensitively. Quotes and a `main.`/`temp.` qualifier are ignored.
fn is_sync_object(name: &str, prefix: &str) -> bool {
    let bare = name
        .trim_matches('"')
        .rsplit('.')
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    bare.starts_with(prefix)
}

fn is_sensitive_table(name: &str) -> bool {
    name.trim_matches('"')
        .rsplit('.')
        .next()
        .unwrap_or(name)
        .eq_ignore_ascii_case("app_settings")
}

/// A capture trigger (`trg_sync_*`) is the only writer allowed to reach the
/// sync tables.
fn is_capture_trigger(accessor: Option<&str>) -> bool {
    accessor.is_some_and(|name| is_sync_object(name, "trg_sync_"))
}

/// Installs the renderer authorizer on `conn` and removes it on drop. The
/// guard borrows the connection mutably-neutrally: SQLite keeps the hook in
/// the connection, so dropping the guard restores the unguarded backend
/// behaviour for every later non-renderer user of the same connection.
pub(crate) struct RendererSqlAuthorizer<'conn> {
    conn: &'conn Connection,
}

impl<'conn> RendererSqlAuthorizer<'conn> {
    pub(crate) fn install(conn: &'conn Connection) -> Self {
        conn.authorizer(Some(|ctx: AuthContext<'_>| authorize(&ctx)));
        Self { conn }
    }
}

impl Drop for RendererSqlAuthorizer<'_> {
    fn drop(&mut self) {
        self.conn
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::ffi;
    use rusqlite::hooks::TransactionOperation;

    fn context(action: AuthAction<'_>) -> AuthContext<'_> {
        AuthContext {
            action,
            database_name: Some("main"),
            accessor: None,
        }
    }

    fn check(action: AuthAction<'_>) -> Authorization {
        authorize(&context(action))
    }

    fn pragma(name: &str) -> AuthAction<'_> {
        AuthAction::Pragma {
            pragma_name: name,
            pragma_value: Some("ON"),
        }
    }

    fn every_ddl_action() -> Vec<AuthAction<'static>> {
        vec![
            AuthAction::CreateIndex {
                index_name: "idx",
                table_name: "items",
            },
            AuthAction::CreateTable { table_name: "t" },
            AuthAction::CreateTempIndex {
                index_name: "idx",
                table_name: "items",
            },
            AuthAction::CreateTempTable { table_name: "t" },
            AuthAction::CreateTempTrigger {
                trigger_name: "trg",
                table_name: "items",
            },
            AuthAction::CreateTempView { view_name: "v" },
            AuthAction::CreateTrigger {
                trigger_name: "trg",
                table_name: "items",
            },
            AuthAction::CreateView { view_name: "v" },
            AuthAction::CreateVtable {
                table_name: "fts",
                module_name: "fts5",
            },
            AuthAction::DropIndex {
                index_name: "idx",
                table_name: "items",
            },
            AuthAction::DropTable {
                table_name: "items",
            },
            AuthAction::DropTempIndex {
                index_name: "idx",
                table_name: "items",
            },
            AuthAction::DropTempTable {
                table_name: "items",
            },
            AuthAction::DropTempTrigger {
                trigger_name: "trg",
                table_name: "items",
            },
            AuthAction::DropTempView { view_name: "v" },
            AuthAction::DropTrigger {
                trigger_name: "trg",
                table_name: "items",
            },
            AuthAction::DropView { view_name: "v" },
            AuthAction::DropVtable {
                table_name: "fts",
                module_name: "fts5",
            },
            AuthAction::AlterTable {
                database_name: "main",
                table_name: "items",
            },
            AuthAction::Reindex {
                index_name: "idx_items_title",
            },
            AuthAction::Analyze {
                table_name: "items",
            },
        ]
    }

    #[test]
    fn every_ddl_action_is_denied() {
        for action in every_ddl_action() {
            assert_eq!(
                check(action),
                Authorization::Deny,
                "DDL action not denied: {action:?}"
            );
        }
    }

    #[test]
    fn attach_detach_and_vacuum_style_attach_are_denied() {
        assert_eq!(
            check(AuthAction::Attach {
                filename: "/tmp/x.db"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Detach { database_name: "x" }),
            Authorization::Deny
        );
        // Plain VACUUM arrives as an ATTACH/DETACH with no filename, which
        // rusqlite can only surface as `Unknown`.
        assert_eq!(
            check(AuthAction::Unknown {
                code: ffi::SQLITE_ATTACH,
                arg1: None,
                arg2: None,
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Unknown {
                code: ffi::SQLITE_DETACH,
                arg1: None,
                arg2: None,
            }),
            Authorization::Deny
        );
    }

    #[test]
    fn pragmas_are_denied_except_the_value_less_data_version() {
        // The FTS5 constructor's read-only pragma is the single exception.
        let data_version = AuthAction::Pragma {
            pragma_name: "data_version",
            pragma_value: None,
        };
        assert_eq!(check(data_version), Authorization::Allow);
        assert_eq!(
            check(AuthAction::Pragma {
                pragma_name: "DATA_VERSION",
                pragma_value: None,
            }),
            Authorization::Allow
        );
        // Setting a value on it, or any other pragma, is denied.
        assert_eq!(
            check(AuthAction::Pragma {
                pragma_name: "data_version",
                pragma_value: Some("ON"),
            }),
            Authorization::Deny
        );
        assert_eq!(check(pragma("defer_foreign_keys")), Authorization::Deny);
        assert_eq!(check(pragma("DEFER_FOREIGN_KEYS")), Authorization::Deny);
        assert_eq!(check(pragma("writable_schema")), Authorization::Deny);
        assert_eq!(check(pragma("foreign_keys")), Authorization::Deny);
        assert_eq!(
            check(AuthAction::Pragma {
                pragma_name: "quick_check",
                pragma_value: None,
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Pragma {
                pragma_name: "quick_check",
                pragma_value: Some("processing_tasks"),
            }),
            Authorization::Deny
        );
    }

    #[test]
    fn app_settings_is_denied_for_every_action() {
        assert_eq!(
            check(AuthAction::Read {
                table_name: "app_settings",
                column_name: "value"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Read {
                table_name: "APP_SETTINGS",
                column_name: "value"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Insert {
                table_name: "app_settings"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Update {
                table_name: "app_settings",
                column_name: "value"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Delete {
                table_name: "app_settings"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::DropTable {
                table_name: "app_settings"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::AlterTable {
                database_name: "main",
                table_name: "app_settings"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Read {
                table_name: "items",
                column_name: "value"
            }),
            Authorization::Allow
        );
    }

    #[test]
    fn sync_writes_are_denied_except_from_capture_triggers() {
        assert_eq!(
            check(AuthAction::Insert {
                table_name: "sync_oplog"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Update {
                table_name: "sync_meta",
                column_name: "value"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::Delete {
                table_name: "sync_pending_rows"
            }),
            Authorization::Deny
        );
        // Reads stay allowed: the renderer may inspect sync status.
        assert_eq!(
            check(AuthAction::Read {
                table_name: "sync_meta",
                column_name: "value"
            }),
            Authorization::Allow
        );

        // The capture trigger body may write the oplog...
        let capture_ctx = AuthContext {
            action: AuthAction::Insert {
                table_name: "sync_oplog",
            },
            database_name: Some("main"),
            accessor: Some("trg_sync_items_i"),
        };
        assert_eq!(authorize(&capture_ctx), Authorization::Allow);
        // ... but no other trigger may.
        let foreign_ctx = AuthContext {
            action: AuthAction::Insert {
                table_name: "sync_oplog",
            },
            database_name: Some("main"),
            accessor: Some("trg_other"),
        };
        assert_eq!(authorize(&foreign_ctx), Authorization::Deny);
    }

    #[test]
    fn sync_ddl_is_denied() {
        assert_eq!(
            check(AuthAction::CreateTable {
                table_name: "sync_new"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::DropTable {
                table_name: "sync_oplog"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::CreateTrigger {
                trigger_name: "trg_sync_items_i",
                table_name: "items"
            }),
            Authorization::Deny
        );
        assert_eq!(
            check(AuthAction::DropTrigger {
                trigger_name: "trg_sync_items_i",
                table_name: "items"
            }),
            Authorization::Deny
        );
        // A trigger with a neutral name planted on a sync table is denied too.
        assert_eq!(
            check(AuthAction::CreateTrigger {
                trigger_name: "trg_copy",
                table_name: "sync_meta"
            }),
            Authorization::Deny
        );
    }

    #[test]
    fn a_single_statement_can_be_the_first_fts5_use_on_a_connection() {
        // FTS5 queries `PRAGMA data_version` when it connects a virtual table
        // on a connection: the first use, or the first after a schema reset.
        // Since migrations run in Rust (Phase 4.2c), that first use on
        // `ui_conn` is an ordinary renderer statement, not a migration batch.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fts.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE VIRTUAL TABLE fts_items USING fts5(body);")
            .unwrap();

        let conn = Connection::open(&path).unwrap();
        let _guard = RendererSqlAuthorizer::install(&conn);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM fts_items", [], |row| row.get(0))
            .expect("the FTS5 constructor's read-only pragma must not be denied");
        assert_eq!(count, 0);
    }

    #[test]
    fn the_guard_removes_the_authorizer_on_panic() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE app_settings (key TEXT, value TEXT);")
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = RendererSqlAuthorizer::install(&conn);
            panic!("simulated command panic");
        }));
        assert!(result.is_err());
        // Unwinding dropped the guard, so the raw backend path works again.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM app_settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn ordinary_sql_is_allowed() {
        assert_eq!(check(AuthAction::Select), Authorization::Allow);
        assert_eq!(
            check(AuthAction::Transaction {
                operation: TransactionOperation::Begin
            }),
            Authorization::Allow
        );
        assert_eq!(
            check(AuthAction::Function {
                function_name: "length"
            }),
            Authorization::Allow
        );
        assert_eq!(
            check(AuthAction::Insert {
                table_name: "items"
            }),
            Authorization::Allow
        );
        assert_eq!(
            check(AuthAction::Update {
                table_name: "items",
                column_name: "title"
            }),
            Authorization::Allow
        );
        assert_eq!(
            check(AuthAction::Delete {
                table_name: "items"
            }),
            Authorization::Allow
        );
        assert_eq!(
            check(AuthAction::Read {
                table_name: "sqlite_master",
                column_name: "name"
            }),
            Authorization::Allow
        );
        assert_eq!(
            check(AuthAction::Savepoint {
                operation: TransactionOperation::Begin,
                savepoint_name: "sp"
            }),
            Authorization::Allow
        );
        assert_eq!(check(AuthAction::Recursive), Authorization::Allow);
    }
}
