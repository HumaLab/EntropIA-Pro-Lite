use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppDbState {
    pub ui_conn: Arc<Mutex<Connection>>,
    #[allow(dead_code)]
    pub worker_conn: Arc<Mutex<Connection>>,
    /// Path to the SQLite file — needed by subsystems that open their own connections.
    pub db_path: PathBuf,
}

impl AppDbState {
    pub fn new(ui_conn: Connection, worker_conn: Connection, db_path: PathBuf) -> Self {
        Self {
            ui_conn: Arc::new(Mutex::new(ui_conn)),
            worker_conn: Arc::new(Mutex::new(worker_conn)),
            db_path,
        }
    }

    #[allow(dead_code)]
    pub fn worker_conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.worker_conn)
    }
}
