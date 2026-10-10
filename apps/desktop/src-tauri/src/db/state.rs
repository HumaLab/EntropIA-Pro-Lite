use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// One-shot migration window phases (S-02c).
///
/// `NotStarted` until the frontend asks the backend to open the window,
/// `Open` while the TypeScript migration runner replays the known migrations,
/// and `Closed` for good afterwards — the window never reopens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationWindowPhase {
    NotStarted,
    Open,
    Closed,
}

const MIGRATION_WINDOW_ALREADY_OPEN: &str = "Migration window is already open for this process";
const MIGRATION_WINDOW_CLOSED: &str = "Migration window is already closed for this process";

/// The backend-owned, one-shot migration window shared by every renderer
/// `db_*` command. Guarded by a mutex so the window transitions are atomic
/// across the blocking IPC tasks; there is no timeout (the TS migrations are
/// not all atomic, so a timer could cut a long migration in half — A-03).
pub struct MigrationWindow {
    phase: Mutex<MigrationWindowPhase>,
}

impl Default for MigrationWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl MigrationWindow {
    pub fn new() -> Self {
        Self {
            phase: Mutex::new(MigrationWindowPhase::NotStarted),
        }
    }

    /// Opens the window. Succeeds only from `NotStarted`: a second `begin`, or
    /// one after `end`/auto-close, is an error.
    pub fn begin(&self) -> Result<(), String> {
        let mut phase = self.phase.lock().map_err(|e| e.to_string())?;
        match *phase {
            MigrationWindowPhase::NotStarted => {
                *phase = MigrationWindowPhase::Open;
                Ok(())
            }
            MigrationWindowPhase::Open => Err(MIGRATION_WINDOW_ALREADY_OPEN.to_string()),
            MigrationWindowPhase::Closed => Err(MIGRATION_WINDOW_CLOSED.to_string()),
        }
    }

    /// Closes the window. Idempotent and never errors.
    pub fn end(&self) {
        if let Ok(mut phase) = self.phase.lock() {
            *phase = MigrationWindowPhase::Closed;
        }
    }

    pub fn phase(&self) -> MigrationWindowPhase {
        self.phase
            .lock()
            .map(|phase| *phase)
            .unwrap_or(MigrationWindowPhase::Closed)
    }

    /// True while the window is `Open`.
    pub fn is_open(&self) -> bool {
        self.phase() == MigrationWindowPhase::Open
    }

    /// A renderer `db_*` call that arrives before `begin` proves the UI is
    /// already operating without migrating, so the window closes and a later
    /// `begin` can never open it.
    pub fn close_if_not_started(&self) {
        if let Ok(mut phase) = self.phase.lock() {
            if *phase == MigrationWindowPhase::NotStarted {
                *phase = MigrationWindowPhase::Closed;
            }
        }
    }
}

#[derive(Clone)]
pub struct AppDbState {
    pub ui_conn: Arc<Mutex<Connection>>,
    #[allow(dead_code)]
    pub worker_conn: Arc<Mutex<Connection>>,
    /// Path to the SQLite file — needed by subsystems that open their own connections.
    pub db_path: PathBuf,
    /// One-shot migration window; see [`MigrationWindow`].
    pub migration_window: Arc<MigrationWindow>,
}

impl AppDbState {
    pub fn new(ui_conn: Connection, worker_conn: Connection, db_path: PathBuf) -> Self {
        Self {
            ui_conn: Arc::new(Mutex::new(ui_conn)),
            worker_conn: Arc::new(Mutex::new(worker_conn)),
            db_path,
            migration_window: Arc::new(MigrationWindow::new()),
        }
    }

    #[allow(dead_code)]
    pub fn worker_conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.worker_conn)
    }
}
