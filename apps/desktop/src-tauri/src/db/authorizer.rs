//! Layer 2 of the renderer SQL defence (S-02a/S-02c): a SQLite authorizer
//! installed on the UI connection only while a renderer `db_*` command runs,
//! through an RAII guard that always removes it on drop (including on error
//! and panic).
//!
//! Layer 1 (the text validators in `commands.rs`) still runs first and keeps
//! its clear messages; this module is the semantic backstop SQLite enforces
//! itself while statements are prepared and while trigger bodies execute.
//! [`authorize`] is a pure function so the whole policy table is unit-testable.
//!
//! Since S-02c the only way to run trigger/view DDL or a `PRAGMA` through the
//! renderer is a single split statement whose SHA-256 fingerprint is in the
//! committed migration allowlist, executed by `db_execute_batch` while the
//! backend-owned migration window is open, with a statement-scoped exception
//! armed for exactly that statement ([`MigrationStatementException`]). Every
//! other command kind denies those actions, window or not.

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::db::migration_allowlist::DdlKind;

/// Which renderer IPC path a guarded statement arrived through. Only
/// `db_execute_batch` may ever run the listed migration DDL: `db_execute`,
/// `db_execute_transaction` and `db_select*` never lift the exception.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RendererSqlKind {
    /// `db_execute_batch` (multi-statement DDL + the TS migration runner).
    Batch,
    /// `db_execute`, `db_execute_transaction`, `db_select`, `db_select_rows`.
    Single,
}

/// Read-only pragmas SQLite itself queries while preparing a renderer
/// statement: `data_version` from the FTS5 vtable constructor, and
/// `quick_check` (with the table name as argument) while rebuilding a table.
/// Both return data and change nothing. The plan's pragma rule explicitly
/// keeps read-only pragmas available; only `db_execute_batch` reaches this
/// list, and every other pragma stays denied outside the migration exception.
const READ_ONLY_PRAGMAS: &[&str] = &["data_version", "quick_check"];

/// The layer-2 policy, kept pure so every rule is exercised by unit tests.
///
/// Returning [`Authorization::Deny`] (never `Ignore`) makes SQLite reject the
/// statement with an authorization error instead of silently substituting
/// NULL or a default.
///
/// | Action | Default | Armed migration exception |
/// |---|---|---|
/// | ATTACH / DETACH (incl. `VACUUM`) | Deny | Deny |
/// | any PRAGMA except read-only `data_version` (Batch) | Deny | Allow `defer_foreign_keys` for `DdlKind::Pragma` |
/// | `app_settings` read/write/DDL | Deny | Deny |
/// | write `sync_*` unless accessor is `trg_sync_*` | Deny | Deny |
/// | DDL on `sync_*` / `trg_sync_*` | Deny | Deny |
/// | CREATE/DROP TRIGGER or VIEW | Deny | Allow for the matching `DdlKind` |
/// | everything else | Allow | Allow |
pub(crate) fn authorize(
    kind: RendererSqlKind,
    exception: Option<&MigrationStatementException>,
    ctx: &AuthContext<'_>,
) -> Authorization {
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

        // Every pragma is denied unless it is either the listed migration
        // statement `PRAGMA defer_foreign_keys` (0045/0048/0050/0052/0058)
        // running under its armed exception, or a read-only pragma SQLite
        // itself issues while preparing a batch statement. The window alone
        // never authorizes a write pragma.
        AuthAction::Pragma { pragma_name, .. } => {
            let listed_migration_pragma = kind == RendererSqlKind::Batch
                && pragma_name.eq_ignore_ascii_case("defer_foreign_keys")
                && exception.is_some_and(|exception| {
                    exception.allows_listed_ddl(DdlKind::Pragma, Some(pragma_name))
                });
            let sqlite_read_only = kind == RendererSqlKind::Batch
                && READ_ONLY_PRAGMAS
                    .iter()
                    .any(|pragma| pragma_name.eq_ignore_ascii_case(pragma));
            if listed_migration_pragma || sqlite_read_only {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }

        // `app_settings` holds secrets; no renderer statement may touch it,
        // directly or through a view/trigger.
        _ if action_table(&ctx.action).is_some_and(is_sensitive_table) => Authorization::Deny,

        // A migration that rebuilds a captured table (`DROP TABLE x`) drops
        // its `trg_sync_x_*` triggers with it: `ensure_capture` installs them at
        // backend setup, before the JS migrations, and `sync_ensure_capture`
        // recreates them afterwards. Only the triggers of the table the armed
        // statement itself drops, and never a `sync_*` table's.
        AuthAction::DropTrigger { table_name, .. }
        | AuthAction::DropTempTrigger { table_name, .. }
            if kind == RendererSqlKind::Batch
                && !is_sync_object(table_name, "sync_")
                && exception
                    .is_some_and(|exception| exception.allows_table_rebuild(table_name)) =>
        {
            Authorization::Allow
        }

        // Sync bookkeeping is engine-owned (DESIGN §6.2). The capture triggers
        // are the one legitimate writer of `sync_oplog`: SQLite reports their
        // body with the trigger as `accessor`.
        _ if touches_sync_objects(&ctx.action, ctx.accessor) => Authorization::Deny,

        // Trigger/view creation is denied for every command kind; only the
        // exact listed migration statement, while its scoped exception is
        // armed, may run it.
        AuthAction::CreateTrigger { .. }
        | AuthAction::CreateTempTrigger { .. }
        | AuthAction::CreateView { .. }
        | AuthAction::CreateTempView { .. } => {
            let ddl_kind = action_ddl_kind(&ctx.action)
                .expect("trigger/view actions always map to a DDL kind");
            let allowed = kind == RendererSqlKind::Batch
                && exception.is_some_and(|exception| {
                    exception.allows_listed_ddl(ddl_kind, action_object(&ctx.action))
                });
            if allowed {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }

        // Dropping a trigger happens explicitly (listed statements) or as an
        // internal step of `DROP TABLE`/`ALTER TABLE` on the table that owns
        // it. The second case is scoped to exactly that table by the executor,
        // and both are still behind the sync/app_settings guards above.
        AuthAction::DropTrigger {
            trigger_name,
            table_name,
        }
        | AuthAction::DropTempTrigger {
            trigger_name,
            table_name,
        } => {
            let listed = exception.is_some_and(|exception| {
                exception.allows_listed_ddl(DdlKind::DropTrigger, Some(trigger_name))
            });
            let rebuilding =
                exception.is_some_and(|exception| exception.allows_table_rebuild(table_name));
            if kind == RendererSqlKind::Batch && (listed || rebuilding) {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }

        AuthAction::DropView { view_name } | AuthAction::DropTempView { view_name } => {
            let listed = exception.is_some_and(|exception| {
                exception.allows_listed_ddl(DdlKind::DropView, Some(view_name))
            });
            if kind == RendererSqlKind::Batch && listed {
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

/// The DDL kind an authorizer action belongs to, for the actions the
/// migration allowlist covers. `None` for every other action.
fn action_ddl_kind(action: &AuthAction<'_>) -> Option<DdlKind> {
    match action {
        AuthAction::CreateTrigger { .. } => Some(DdlKind::CreateTrigger),
        AuthAction::CreateTempTrigger { .. } => Some(DdlKind::CreateTempTrigger),
        AuthAction::CreateView { .. } => Some(DdlKind::CreateView),
        AuthAction::CreateTempView { .. } => Some(DdlKind::CreateTempView),
        // SQLite has no `DROP TEMP TRIGGER/VIEW` syntax; the temp variants map
        // to the same drop kind on purpose, so a temp object can never widen
        // the exception.
        AuthAction::DropTrigger { .. } | AuthAction::DropTempTrigger { .. } => {
            Some(DdlKind::DropTrigger)
        }
        AuthAction::DropView { .. } | AuthAction::DropTempView { .. } => Some(DdlKind::DropView),
        AuthAction::Pragma { .. } => Some(DdlKind::Pragma),
        _ => None,
    }
}

/// The object name an authorizer action declares (trigger, view or pragma).
fn action_object<'a>(action: &'a AuthAction<'a>) -> Option<&'a str> {
    match action {
        AuthAction::CreateTrigger { trigger_name, .. }
        | AuthAction::CreateTempTrigger { trigger_name, .. }
        | AuthAction::DropTrigger { trigger_name, .. }
        | AuthAction::DropTempTrigger { trigger_name, .. } => Some(trigger_name),
        AuthAction::CreateView { view_name }
        | AuthAction::CreateTempView { view_name }
        | AuthAction::DropView { view_name }
        | AuthAction::DropTempView { view_name } => Some(view_name),
        AuthAction::Pragma { pragma_name, .. } => Some(pragma_name),
        _ => None,
    }
}

/// The statement-scoped exception for one listed migration statement. Created
/// disarmed; [`ArmedMigrationException`] arms it for the duration of a single
/// `execute_batch` call and disarms it on drop, including on error.
///
/// The exception is deliberately narrow: it either names the DDL kind (and,
/// when the statement text yields it, the exact object) of a listed migration
/// statement, or it names the exact table a `DROP TABLE`/`ALTER TABLE`
/// statement is rebuilding, so SQLite may drop that table's own triggers. It
/// never authorizes anything else.
enum MigrationExceptionScope {
    ListedDdl {
        kind: DdlKind,
        object: Option<String>,
    },
    TableRebuild {
        table: String,
    },
}

pub(crate) struct MigrationStatementException {
    scope: MigrationExceptionScope,
    armed: AtomicBool,
}

impl MigrationStatementException {
    /// Exception for a listed migration statement, scoped to its DDL kind and
    /// (when readable) the object it declares.
    pub(crate) fn new(kind: DdlKind, object: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            scope: MigrationExceptionScope::ListedDdl { kind, object },
            armed: AtomicBool::new(false),
        })
    }

    /// Exception for a `DROP TABLE`/`ALTER TABLE` statement, scoped to the
    /// triggers SQLite drops internally for exactly that table.
    pub(crate) fn for_table_rebuild(table: String) -> Arc<Self> {
        Arc::new(Self {
            scope: MigrationExceptionScope::TableRebuild { table },
            armed: AtomicBool::new(false),
        })
    }

    pub(crate) fn is_armed(&self) -> bool {
        self.armed.load(Ordering::Acquire)
    }

    /// True while armed for exactly a listed `kind` (and the named object,
    /// when the statement named one).
    fn allows_listed_ddl(&self, kind: DdlKind, object: Option<&str>) -> bool {
        if !self.is_armed() {
            return false;
        }
        let MigrationExceptionScope::ListedDdl {
            kind: scope_kind,
            object: scope_object,
        } = &self.scope
        else {
            return false;
        };
        if *scope_kind != kind {
            return false;
        }
        match (scope_object, object) {
            (Some(expected), Some(actual)) => same_object(expected, actual),
            (Some(_), None) => false,
            // The listed statement's object name could not be read from the
            // text: the fingerprint already proved which statement this is.
            (None, _) => true,
        }
    }

    /// True while armed for a rebuild of exactly `table` (bare-name compare).
    fn allows_table_rebuild(&self, table: &str) -> bool {
        if !self.is_armed() {
            return false;
        }
        match &self.scope {
            MigrationExceptionScope::TableRebuild { table: scoped } => same_object(scoped, table),
            _ => false,
        }
    }
}

/// Arms a [`MigrationStatementException`] for one statement; dropping it (on
/// success, error or panic) clears the exception again.
pub(crate) struct ArmedMigrationException<'a> {
    exception: &'a MigrationStatementException,
}

impl<'a> ArmedMigrationException<'a> {
    pub(crate) fn arm(exception: &'a MigrationStatementException) -> Self {
        exception.armed.store(true, Ordering::Release);
        Self { exception }
    }
}

impl Drop for ArmedMigrationException<'_> {
    fn drop(&mut self) {
        self.exception.armed.store(false, Ordering::Release);
    }
}

/// Bare object-name comparison: quotes and a `main.`/`temp.` qualifier are
/// ignored, case is not significant.
fn same_object(expected: &str, actual: &str) -> bool {
    bare_object_name(expected).eq_ignore_ascii_case(&bare_object_name(actual))
}

fn bare_object_name(name: &str) -> String {
    let bare = name.rsplit('.').next().unwrap_or(name);
    bare.trim_matches(&['"', '`', '[', ']'][..]).to_string()
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
///
/// `exception` is the statement-scoped migration exception the policy reads
/// while it is armed; every non-batch command passes `None`.
pub(crate) struct RendererSqlAuthorizer<'conn> {
    conn: &'conn Connection,
}

impl<'conn> RendererSqlAuthorizer<'conn> {
    pub(crate) fn install(
        conn: &'conn Connection,
        kind: RendererSqlKind,
        exception: Option<Arc<MigrationStatementException>>,
    ) -> Self {
        conn.authorizer(Some(move |ctx: AuthContext<'_>| {
            authorize(kind, exception.as_deref(), &ctx)
        }));
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
        authorize(kind, None, &context(action))
    }

    fn check_with_exception(
        kind: RendererSqlKind,
        exception: Option<&MigrationStatementException>,
        action: AuthAction<'_>,
    ) -> Authorization {
        authorize(kind, exception, &context(action))
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
    fn pragmas_are_denied_without_an_armed_migration_exception() {
        for kind in [RendererSqlKind::Batch, RendererSqlKind::Single] {
            assert_eq!(
                check(kind, pragma("defer_foreign_keys")),
                Authorization::Deny
            );
            assert_eq!(
                check(kind, pragma("DEFER_FOREIGN_KEYS")),
                Authorization::Deny
            );
            assert_eq!(check(kind, pragma("writable_schema")), Authorization::Deny);
            assert_eq!(check(kind, pragma("foreign_keys")), Authorization::Deny);
        }
    }

    #[test]
    fn the_fts5_vtable_constructor_pragma_stays_available_to_batch_only() {
        // `CREATE VIRTUAL TABLE ... fts5` makes the module query
        // `PRAGMA data_version`; it is read-only, so Batch may run it while
        // the renderer paths never lift anything.
        assert_eq!(
            check(RendererSqlKind::Batch, pragma("data_version")),
            Authorization::Allow
        );
        assert_eq!(
            check(RendererSqlKind::Single, pragma("data_version")),
            Authorization::Deny
        );
        assert_eq!(
            check(RendererSqlKind::Batch, pragma("DATA_VERSION")),
            Authorization::Allow
        );
        // SQLite also runs `PRAGMA quick_check(<table>)` while rebuilding one.
        assert_eq!(
            check(
                RendererSqlKind::Batch,
                AuthAction::Pragma {
                    pragma_name: "quick_check",
                    pragma_value: Some("processing_tasks"),
                },
            ),
            Authorization::Allow
        );
    }

    #[test]
    fn pragma_exception_only_allows_the_listed_defer_foreign_keys() {
        let exception =
            MigrationStatementException::new(DdlKind::Pragma, Some("defer_foreign_keys".into()));
        // Disarmed: still denied.
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Batch,
                Some(&exception),
                pragma("defer_foreign_keys")
            ),
            Authorization::Deny
        );
        let armed = ArmedMigrationException::arm(&exception);
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Batch,
                Some(&exception),
                pragma("defer_foreign_keys")
            ),
            Authorization::Allow
        );
        // A different pragma of the same kind does not pass the object check.
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Batch,
                Some(&exception),
                pragma("writable_schema")
            ),
            Authorization::Deny
        );
        // Nor does a different kind with the same object name.
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Single,
                Some(&exception),
                pragma("defer_foreign_keys")
            ),
            Authorization::Deny
        );
        drop(armed);
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Batch,
                Some(&exception),
                pragma("defer_foreign_keys")
            ),
            Authorization::Deny,
            "dropping the armed guard must clear the exception"
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
            authorize(RendererSqlKind::Single, None, &capture_ctx),
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
            authorize(RendererSqlKind::Single, None, &foreign_ctx),
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
    fn trigger_and_view_ddl_needs_the_matching_armed_exception() {
        let trigger = AuthAction::CreateTrigger {
            trigger_name: "rag_chunks_fts_insert",
            table_name: "rag_chunks",
        };
        let view = AuthAction::CreateView { view_name: "v_t" };
        let drop = AuthAction::DropTrigger {
            trigger_name: "rag_chunks_fts_insert",
            table_name: "rag_chunks",
        };

        // Without an exception: denied for every command kind, Batch included.
        for kind in [RendererSqlKind::Batch, RendererSqlKind::Single] {
            assert_eq!(check(kind, trigger), Authorization::Deny);
            assert_eq!(check(kind, view), Authorization::Deny);
            assert_eq!(check(kind, drop), Authorization::Deny);
            assert_eq!(
                check(kind, AuthAction::DropTempView { view_name: "v_t" }),
                Authorization::Deny
            );
        }

        // Batch with an armed exception of the matching kind and object: allow.
        let exception = MigrationStatementException::new(
            DdlKind::CreateTrigger,
            Some("rag_chunks_fts_insert".into()),
        );
        let _armed = ArmedMigrationException::arm(&exception);
        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&exception), trigger),
            Authorization::Allow
        );
        // A different object or a different kind does not pass the scope check.
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Batch,
                Some(&exception),
                AuthAction::CreateTrigger {
                    trigger_name: "trg_extra",
                    table_name: "rag_chunks",
                },
            ),
            Authorization::Deny
        );
        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&exception), view),
            Authorization::Deny
        );
        // Non-batch commands never lift the exception, even when it is armed.
        assert_eq!(
            check_with_exception(RendererSqlKind::Single, Some(&exception), trigger),
            Authorization::Deny
        );
        // A quoted/schema-qualified name still matches the bare name.
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Batch,
                Some(&exception),
                AuthAction::CreateTrigger {
                    trigger_name: "main.\"rag_chunks_fts_insert\"",
                    table_name: "rag_chunks",
                },
            ),
            Authorization::Allow
        );
    }

    #[test]
    fn table_rebuild_exception_drops_that_tables_capture_triggers_only() {
        let exception = MigrationStatementException::for_table_rebuild("annotations".to_string());
        let own_capture = AuthAction::DropTrigger {
            trigger_name: "trg_sync_annotations_i",
            table_name: "annotations",
        };
        let other_capture = AuthAction::DropTrigger {
            trigger_name: "trg_sync_items_i",
            table_name: "items",
        };

        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&exception), own_capture),
            Authorization::Deny
        );
        let _armed = ArmedMigrationException::arm(&exception);
        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&exception), own_capture),
            Authorization::Allow
        );
        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&exception), other_capture),
            Authorization::Deny
        );
        assert_eq!(
            check_with_exception(RendererSqlKind::Single, Some(&exception), own_capture),
            Authorization::Deny
        );

        // Never for a sync table, armed or not.
        let sync_exception =
            MigrationStatementException::for_table_rebuild("sync_meta".to_string());
        let _sync_armed = ArmedMigrationException::arm(&sync_exception);
        let sync_trigger = AuthAction::DropTrigger {
            trigger_name: "trg_sync_meta_guard",
            table_name: "sync_meta",
        };
        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&sync_exception), sync_trigger),
            Authorization::Deny
        );
    }

    #[test]
    fn table_rebuild_exception_only_drops_that_tables_own_triggers() {
        let exception =
            MigrationStatementException::for_table_rebuild("processing_tasks".to_string());
        let own_trigger = AuthAction::DropTrigger {
            trigger_name: "processing_tasks_settle_dependents",
            table_name: "processing_tasks",
        };
        let other_trigger = AuthAction::DropTrigger {
            trigger_name: "collection_activity_items_update",
            table_name: "items",
        };

        // Disarmed: denied.
        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&exception), own_trigger),
            Authorization::Deny
        );
        let _armed = ArmedMigrationException::arm(&exception);
        // Armed: exactly the triggers of the table being rebuilt.
        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&exception), own_trigger),
            Authorization::Allow
        );
        assert_eq!(
            check_with_exception(RendererSqlKind::Batch, Some(&exception), other_trigger),
            Authorization::Deny
        );
        // It never lifts creation, views or pragmas, nor any non-batch kind.
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Batch,
                Some(&exception),
                AuthAction::CreateTrigger {
                    trigger_name: "processing_tasks_settle_dependents",
                    table_name: "processing_tasks",
                },
            ),
            Authorization::Deny
        );
        assert_eq!(
            check_with_exception(
                RendererSqlKind::Batch,
                Some(&exception),
                AuthAction::DropView {
                    view_name: "processing_tasks",
                },
            ),
            Authorization::Deny
        );
        assert_eq!(
            check_with_exception(RendererSqlKind::Single, Some(&exception), own_trigger),
            Authorization::Deny
        );
    }

    #[test]
    fn the_guard_removes_the_authorizer_on_panic() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE app_settings (key TEXT, value TEXT);")
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = RendererSqlAuthorizer::install(&conn, RendererSqlKind::Single, None);
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
