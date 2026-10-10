//! Layer 2 of the renderer SQL defence (S-02a): a SQLite authorizer installed
//! on the UI connection only while a renderer `db_*` command runs, through an
//! RAII guard that always removes it on drop (including on error and panic).
//!
//! Layer 1 (the text validators in `commands.rs`) still runs first and keeps
//! its clear messages; this module is the semantic backstop SQLite enforces
//! itself while statements are prepared and while trigger bodies execute.
//! [`authorize`] is a pure function so the whole policy table is unit-testable.

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::Connection;

/// Which renderer IPC path a guarded statement arrived through. Only
/// `db_execute_batch` may run `PRAGMA defer_foreign_keys` or trigger/view DDL:
/// the TypeScript migration runner needs both until S-02c replaces this with a
/// per-statement fingerprint allowlist inside a backend-owned migration window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RendererSqlKind {
    /// `db_execute_batch` (multi-statement DDL + the TS migration runner).
    Batch,
    /// `db_execute`, `db_execute_transaction`, `db_select`, `db_select_rows`.
    Single,
}

/// The layer-2 policy, kept pure so every rule is exercised by unit tests.
///
/// Returning [`Authorization::Deny`] (never `Ignore`) makes SQLite reject the
/// statement with an authorization error instead of silently substituting
/// NULL or a default.
///
/// | Action | Batch (`db_execute_batch`) | Single (`db_execute*`/`db_select*`) |
/// |---|---|---|
/// | ATTACH / DETACH (incl. `VACUUM`) | Deny | Deny |
/// | PRAGMA `defer_foreign_keys` | Allow (S-02c narrows it) | Deny |
/// | any other PRAGMA | Deny | Deny |
/// | `app_settings` read/write/DDL | Deny | Deny |
/// | write `sync_*` unless accessor is `trg_sync_*` | Deny | Deny |
/// | DDL on `sync_*` / `trg_sync_*` | Deny | Deny |
/// | CREATE/DROP TRIGGER or VIEW | Allow until S-02c | Deny |
/// | everything else | Allow | Allow |
pub(crate) fn authorize(kind: RendererSqlKind, ctx: &AuthContext<'_>) -> Authorization {
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

        AuthAction::Pragma { pragma_name, .. } => {
            if kind == RendererSqlKind::Batch
                && pragma_name.eq_ignore_ascii_case("defer_foreign_keys")
            {
                // The TS migration runner emits this one write pragma inside
                // its migration batches (0045/0048/0050/0052/0058). S-02c
                // narrows it to the exact listed migration statements inside a
                // backend-owned migration window.
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

        // Trigger/view DDL stays on the batch path until S-02c replaces it with
        // the fingerprint allowlist inside the migration window.
        AuthAction::CreateTrigger { .. }
        | AuthAction::CreateTempTrigger { .. }
        | AuthAction::DropTrigger { .. }
        | AuthAction::DropTempTrigger { .. }
        | AuthAction::CreateView { .. }
        | AuthAction::CreateTempView { .. }
        | AuthAction::DropView { .. }
        | AuthAction::DropTempView { .. } => {
            if kind == RendererSqlKind::Batch {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }

        // Ordinary DML, SELECT (including sqlite_master), CREATE/DROP
        // TABLE/INDEX, ALTER, transactions, functions and recursive CTEs.
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
    pub(crate) fn install(conn: &'conn Connection, kind: RendererSqlKind) -> Self {
        conn.authorizer(Some(move |ctx: AuthContext<'_>| authorize(kind, &ctx)));
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

    fn check(kind: RendererSqlKind, action: AuthAction<'_>) -> Authorization {
        authorize(kind, &context(action))
    }

    fn pragma(name: &str) -> AuthAction<'_> {
        AuthAction::Pragma {
            pragma_name: name,
            pragma_value: Some("ON"),
        }
    }

    #[test]
    fn attach_detach_and_vacuum_style_attach_are_denied() {
        for kind in [RendererSqlKind::Batch, RendererSqlKind::Single] {
            assert_eq!(
                check(
                    kind,
                    AuthAction::Attach {
                        filename: "/tmp/x.db"
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(kind, AuthAction::Detach { database_name: "x" }),
                Authorization::Deny
            );
            // Plain VACUUM arrives as an ATTACH/DETACH with no filename, which
            // rusqlite can only surface as `Unknown`.
            assert_eq!(
                check(
                    kind,
                    AuthAction::Unknown {
                        code: ffi::SQLITE_ATTACH,
                        arg1: None,
                        arg2: None,
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Unknown {
                        code: ffi::SQLITE_DETACH,
                        arg1: None,
                        arg2: None,
                    }
                ),
                Authorization::Deny
            );
        }
    }

    #[test]
    fn pragmas_are_denied_except_defer_foreign_keys_for_batch() {
        assert_eq!(
            check(RendererSqlKind::Batch, pragma("defer_foreign_keys")),
            Authorization::Allow
        );
        assert_eq!(
            check(RendererSqlKind::Batch, pragma("DEFER_FOREIGN_KEYS")),
            Authorization::Allow
        );
        assert_eq!(
            check(RendererSqlKind::Single, pragma("defer_foreign_keys")),
            Authorization::Deny
        );
        assert_eq!(
            check(RendererSqlKind::Batch, pragma("writable_schema")),
            Authorization::Deny
        );
        assert_eq!(
            check(RendererSqlKind::Batch, pragma("foreign_keys")),
            Authorization::Deny
        );
    }

    #[test]
    fn app_settings_is_denied_for_every_action() {
        for kind in [RendererSqlKind::Batch, RendererSqlKind::Single] {
            assert_eq!(
                check(
                    kind,
                    AuthAction::Read {
                        table_name: "app_settings",
                        column_name: "value"
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Read {
                        table_name: "APP_SETTINGS",
                        column_name: "value"
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Insert {
                        table_name: "app_settings"
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Update {
                        table_name: "app_settings",
                        column_name: "value"
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Delete {
                        table_name: "app_settings"
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::DropTable {
                        table_name: "app_settings"
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::AlterTable {
                        database_name: "main",
                        table_name: "app_settings"
                    }
                ),
                Authorization::Deny
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Read {
                        table_name: "items",
                        column_name: "value"
                    }
                ),
                Authorization::Allow
            );
        }
    }

    #[test]
    fn sync_writes_are_denied_except_from_capture_triggers() {
        assert_eq!(
            check(
                RendererSqlKind::Single,
                AuthAction::Insert {
                    table_name: "sync_oplog"
                }
            ),
            Authorization::Deny
        );
        assert_eq!(
            check(
                RendererSqlKind::Single,
                AuthAction::Update {
                    table_name: "sync_meta",
                    column_name: "value"
                }
            ),
            Authorization::Deny
        );
        assert_eq!(
            check(
                RendererSqlKind::Single,
                AuthAction::Delete {
                    table_name: "sync_pending_rows"
                }
            ),
            Authorization::Deny
        );
        // Reads stay allowed: the renderer may inspect sync status.
        assert_eq!(
            check(
                RendererSqlKind::Single,
                AuthAction::Read {
                    table_name: "sync_meta",
                    column_name: "value"
                }
            ),
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
        assert_eq!(
            authorize(RendererSqlKind::Single, &capture_ctx),
            Authorization::Allow
        );
        // ... but no other trigger may.
        let foreign_ctx = AuthContext {
            action: AuthAction::Insert {
                table_name: "sync_oplog",
            },
            database_name: Some("main"),
            accessor: Some("trg_other"),
        };
        assert_eq!(
            authorize(RendererSqlKind::Single, &foreign_ctx),
            Authorization::Deny
        );
    }

    #[test]
    fn sync_ddl_is_denied_even_through_batch() {
        assert_eq!(
            check(
                RendererSqlKind::Batch,
                AuthAction::CreateTable {
                    table_name: "sync_new"
                }
            ),
            Authorization::Deny
        );
        assert_eq!(
            check(
                RendererSqlKind::Batch,
                AuthAction::DropTable {
                    table_name: "sync_oplog"
                }
            ),
            Authorization::Deny
        );
        assert_eq!(
            check(
                RendererSqlKind::Batch,
                AuthAction::CreateTrigger {
                    trigger_name: "trg_sync_items_i",
                    table_name: "items"
                }
            ),
            Authorization::Deny
        );
        assert_eq!(
            check(
                RendererSqlKind::Batch,
                AuthAction::DropTrigger {
                    trigger_name: "trg_sync_items_i",
                    table_name: "items"
                }
            ),
            Authorization::Deny
        );
        // A trigger with a neutral name planted on a sync table is denied too.
        assert_eq!(
            check(
                RendererSqlKind::Batch,
                AuthAction::CreateTrigger {
                    trigger_name: "trg_copy",
                    table_name: "sync_meta"
                }
            ),
            Authorization::Deny
        );
    }

    #[test]
    fn trigger_and_view_ddl_is_batch_only() {
        let trigger = AuthAction::CreateTrigger {
            trigger_name: "trg_t",
            table_name: "items",
        };
        assert_eq!(check(RendererSqlKind::Batch, trigger), Authorization::Allow);
        assert_eq!(check(RendererSqlKind::Single, trigger), Authorization::Deny);
        let view = AuthAction::CreateView { view_name: "v_t" };
        assert_eq!(check(RendererSqlKind::Batch, view), Authorization::Allow);
        assert_eq!(check(RendererSqlKind::Single, view), Authorization::Deny);
        assert_eq!(
            check(
                RendererSqlKind::Single,
                AuthAction::DropTempView { view_name: "v_t" }
            ),
            Authorization::Deny
        );
    }

    #[test]
    fn the_guard_removes_the_authorizer_on_panic() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE app_settings (key TEXT, value TEXT);")
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = RendererSqlAuthorizer::install(&conn, RendererSqlKind::Single);
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
        for kind in [RendererSqlKind::Batch, RendererSqlKind::Single] {
            assert_eq!(check(kind, AuthAction::Select), Authorization::Allow);
            assert_eq!(
                check(
                    kind,
                    AuthAction::Transaction {
                        operation: TransactionOperation::Begin
                    }
                ),
                Authorization::Allow
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Function {
                        function_name: "length"
                    }
                ),
                Authorization::Allow
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Insert {
                        table_name: "items"
                    }
                ),
                Authorization::Allow
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::CreateTable {
                        table_name: "notes"
                    }
                ),
                Authorization::Allow
            );
            assert_eq!(
                check(
                    kind,
                    AuthAction::Read {
                        table_name: "sqlite_master",
                        column_name: "name"
                    }
                ),
                Authorization::Allow
            );
        }
    }
}
