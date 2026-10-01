//! Desktop ownership and scheduling for the durable research engine.
use entropia_agent::{
    cliente_llm::{ClienteLlm, ClienteLlmOpenRouter, TurnoAgente},
    embeddings::ClienteEmbeddings,
    estado::EstadoDb,
    recuperacion::Recuperador,
    repositorio::RepositorioSqlite,
    rerank::ClienteRerank,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, State};

#[derive(Clone)]
pub struct ResearchState(Arc<Inner>);
struct Inner {
    corpus: PathBuf,
    state: PathBuf,
    artifacts: PathBuf,
    mutation: tokio::sync::Mutex<()>,
    active: Mutex<HashMap<String, Arc<AtomicU8>>>,
    /// Hybrid retrieval, built once per credential. `Recuperador` caches the
    /// corpus with its embeddings: building it per call would reload every
    /// chunk on every step of the job.
    retrieval: Mutex<Option<(String, Arc<Recuperador>)>>,
}
struct NoModel;
impl ClienteLlm for NoModel {
    fn turno_agente(&self, _: &[Value], _: &[Value]) -> Result<TurnoAgente, String> {
        Err("Operación sin autorización de modelo".into())
    }
    fn modelo(&self) -> &str {
        "unconfigured"
    }
}
/// El id del job solo cuando una mutación exitosa lo cierra en terminal
/// (`done`/`failed`). Lecturas (`list`/`get`/`source`), trabajo activo y
/// estados intermedios nunca capturan.
fn terminal_job_id(response: &Value) -> Option<String> {
    let job = response.get("job")?;
    match job.get("status").and_then(Value::as_str)? {
        "done" | "failed" => {}
        _ => return None,
    }
    let id = job.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}
/// Encola un job terminal en la outbox de sync abriendo la conexión de sync
/// existente y el estado de research de solo lectura. Sin esquema de sync o con
/// la captura deshabilitada es un no-op (contrato de captura existente); un
/// error real de estado de captura se informa, nunca se descarta en silencio.
fn capture_terminal_job(
    corpus: &std::path::Path,
    state: &std::path::Path,
    job_id: &str,
) -> Result<(), String> {
    let sync = crate::sync::open_sync_connection(corpus)?;
    let state = crate::sync::research_envelope::open_research_state_read_only(state)
        .map_err(|error| error.message)?;
    crate::sync::research_capture::enqueue_terminal_job(&sync, &state, job_id)
        .map_err(|error| error.message)
}
/// Ops that reach the model and therefore need the credential wired in.
fn uses_model(op: &str) -> bool {
    matches!(op, "advance" | "rewrite_section")
}
impl ResearchState {
    pub fn new(app_dir: PathBuf, corpus: PathBuf) -> Result<Self, String> {
        let root = app_dir.join("research");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let state = root.join("estado.sqlite");
        let db = EstadoDb::abrir(state.to_str().ok_or("Ruta no UTF-8")?)?;
        // Recovery is explicit: interrupted work never silently replays a billable call.
        let motor = entropia_agent::trabajos::MotorTrabajos::nuevo(&db);
        for id in motor.listar_ids_jobs() {
            if motor.obtener_job(&id).is_some_and(|j| {
                j.modo == "research" && j.status == entropia_agent::trabajos::EstadoJob::Running
            }) {
                motor.pausar(&id)?;
            }
        }
        Ok(Self(Arc::new(Inner {
            corpus,
            state,
            artifacts: root.join("artifacts"),
            mutation: tokio::sync::Mutex::new(()),
            active: Mutex::new(HashMap::new()),
            retrieval: Mutex::new(None),
        })))
    }
    async fn execute(&self, request: Value, model: bool) -> Result<Value, String> {
        let inner = self.0.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let db = EstadoDb::abrir(inner.state.to_str().ok_or("Ruta no UTF-8")?)?;
            let repo = RepositorioSqlite::abrir(inner.corpus.to_str().ok_or("Ruta no UTF-8")?)?;
            if model {
                let conn = rusqlite::Connection::open_with_flags(
                    &inner.corpus,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .map_err(|e| e.to_string())?;
                let key = crate::settings::get_setting(&conn, crate::settings::OPENROUTER_API_KEY)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("Configura la credencial OpenRouter en Configuración")?;
                let model = crate::settings::get_setting(&conn, "rag_model")
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| {
                        crate::settings::get_setting(&conn, "openrouter_model")
                            .filter(|s| !s.trim().is_empty())
                    })
                    .unwrap_or_else(|| ClienteLlmOpenRouter::MODELO_DEFAULT.into());
                let llm = ClienteLlmOpenRouter::new(key.clone(), model);
                // Same credential drives embeddings and rerank. Without it the
                // engine falls back to lexical search and says so in the report.
                let recuperador = {
                    let mut slot = inner
                        .retrieval
                        .lock()
                        .map_err(|_| "Recuperación no disponible".to_string())?;
                    match slot.as_ref() {
                        Some((actual, rec)) if actual == &key => rec.clone(),
                        _ => {
                            let rec = Arc::new(Recuperador::new(
                                ClienteEmbeddings::new(key.clone()),
                                ClienteRerank::new(key.clone()),
                            ));
                            *slot = Some((key, rec.clone()));
                            rec
                        }
                    }
                };
                entropia_agent::investigacion::procesar(
                    &db,
                    &repo,
                    &llm,
                    Some(recuperador.as_ref()),
                    &inner.artifacts,
                    request,
                )
            } else {
                entropia_agent::investigacion::procesar(
                    &db,
                    &repo,
                    &NoModel,
                    None,
                    &inner.artifacts,
                    request,
                )
            }
        })
        .await
        .map_err(|e| format!("Falló el proceso de investigación: {e}"))?
    }
    fn schedule(&self, id: String, app: AppHandle) -> Result<(), String> {
        let control = Arc::new(AtomicU8::new(0));
        {
            let mut active = self
                .0
                .active
                .lock()
                .map_err(|_| "Control de investigación no disponible")?;
            if active.contains_key(&id) {
                return Ok(());
            }
            active.insert(id.clone(), control.clone());
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let _lock = this.0.mutation.lock().await;
                let requested = control.swap(0, Ordering::AcqRel);
                let op = match requested {
                    1 => "pause",
                    2 => "cancel",
                    _ => "advance",
                };
                let result = this
                    .execute(json!({"op":op,"job_id":id}), uses_model(op))
                    .await;
                // Cierre terminal del avance programado: capturar ANTES de que
                // el resultado se consuma en el match de abajo.
                if let Ok(value) = &result {
                    this.capture_closed_job(&app, value).await;
                }
                match &result {
                    Ok(value) if value["job"]["status"] == "running" && op == "advance" => {}
                    _ => {
                        if let Err(error) = result {
                            // Credential/transport initialization can fail before entering the engine.
                            let _ = this.execute(json!({"op":"pause","job_id":id}), false).await;
                            let path = this.0.state.clone();
                            let job = id.clone();
                            let _ = tauri::async_runtime::spawn_blocking(move || {
                                if let Ok(db) = EstadoDb::abrir(&path.to_string_lossy()) {
                                    let _ = entropia_agent::trabajos::MotorTrabajos::nuevo(&db)
                                        .registrar_evento(
                                            &job,
                                            None,
                                            "research_error",
                                            Some(&json!({"message":error}).to_string()),
                                        );
                                }
                            })
                            .await;
                        }
                        // Apply a queued cancellation even when a stage ended at a gate.
                        if control.swap(0, Ordering::AcqRel) == 2 {
                            if let Ok(value) = this
                                .execute(json!({"op":"cancel","job_id":id}), false)
                                .await
                            {
                                this.capture_closed_job(&app, &value).await;
                            }
                        }
                        break;
                    }
                }
            }
            if let Ok(mut active) = this.0.active.lock() {
                active.remove(&id);
            }
        });
        Ok(())
    }
    async fn request(&self, request: Value, app: &AppHandle) -> Result<Value, String> {
        let op = request["op"].as_str().ok_or("Falta op")?;
        if matches!(op, "list" | "get" | "source") {
            return self.execute(request, false).await;
        }
        let id = request["job_id"].as_str().unwrap_or("").to_owned();
        if matches!(op, "pause" | "cancel") {
            let control = self
                .0
                .active
                .lock()
                .map_err(|_| "Control no disponible")?
                .get(&id)
                .cloned();
            if let Some(control) = control {
                control.fetch_max(if op == "cancel" { 2 } else { 1 }, Ordering::AcqRel);
                let mut response = self.execute(json!({"op":"get","job_id":id}), false).await?;
                response["pause_requested"] = json!(op == "pause");
                response["cancel_requested"] = json!(op == "cancel");
                return Ok(response);
            }
        }
        if op == "advance" {
            return Err("El desktop conduce los pasos; usa reanudar".into());
        }
        if op == "delete" {
            let _lock = self.0.mutation.lock().await;
            return self.delete_job(&id).await;
        }
        let model = uses_model(op);
        let _lock = self.0.mutation.lock().await;
        let response = self.execute(request, model).await?;
        if response["job"]["status"] == "running" {
            let id = response["job"]["id"]
                .as_str()
                .ok_or("Motor devolvió un job sin ID")?
                .to_owned();
            self.schedule(id, app.clone())?;
        } else {
            // Una mutación exitosa que dejó el job en terminal (`done`/`failed`)
            // — ediciones a mano, informes, cierres — queda en la outbox de
            // sync. Trabajo activo nunca se captura acá.
            self.capture_closed_job(app, &response).await;
        }
        Ok(response)
    }
    /// Encola en la outbox de sync un job que acaba de cerrar. No-op cuando la
    /// respuesta no es un cierre terminal; un error de captura real se registra
    /// en los logs de la aplicación en vez de descartarse.
    async fn capture_closed_job(&self, app: &AppHandle, response: &Value) {
        let Some(job_id) = terminal_job_id(response) else {
            return;
        };
        let corpus = self.0.corpus.clone();
        let state = self.0.state.clone();
        let capture = tauri::async_runtime::spawn_blocking(move || {
            capture_terminal_job(&corpus, &state, &job_id)
        })
        .await;
        let error = match capture {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(error) => format!("Falló la captura de cierre: {error}"),
        };
        crate::app_logs::warn(
            app,
            "research",
            format!("No se pudo encolar la investigación cerrada para sincronizar: {error}"),
        );
    }
    /// Borra una investigación dejando la baja registrada para sincronizar.
    ///
    /// La intención de baja es duradera ANTES de la llamada destructiva. Si el
    /// motor borra, el asentamiento deja un tombstone `D` idempotente en la
    /// outbox de research; si el borrado falla, se aborta la intención y nunca
    /// se encola una baja para un pedido fallido. Si el borrado tuvo éxito pero
    /// el asentamiento falla, la intención queda registrada y se puede
    /// asentar después — el error se informa en vez de ocultarse.
    async fn delete_job(&self, id: &str) -> Result<Value, String> {
        self.begin_delete_intent(id).await?;
        match self
            .execute(json!({"op": "delete", "job_id": id}), false)
            .await
        {
            Err(error) => {
                self.abort_delete_intent(id).await.map_err(|intent| {
                    format!("{error}; además falló la limpieza de la intención de baja: {intent}")
                })?;
                Err(error)
            }
            Ok(response) => {
                self.settle_delete_intent(id).await?;
                Ok(response)
            }
        }
    }
    async fn begin_delete_intent(&self, id: &str) -> Result<(), String> {
        let corpus = self.0.corpus.clone();
        let state = self.0.state.clone();
        let id = id.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let sync = crate::sync::open_sync_connection(&corpus)?;
            let state = crate::sync::research_envelope::open_research_state_read_only(&state)
                .map_err(|error| error.message)?;
            crate::sync::research_capture::begin_delete_intent(&sync, &state, &id)
                .map_err(|error| error.message)
        })
        .await
        .map_err(|e| format!("Falló la intención de baja: {e}"))?
    }
    async fn abort_delete_intent(&self, id: &str) -> Result<(), String> {
        let corpus = self.0.corpus.clone();
        let id = id.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let sync = crate::sync::open_sync_connection(&corpus)?;
            crate::sync::research_capture::abort_delete_intent(&sync, &id)
                .map_err(|error| error.message)
        })
        .await
        .map_err(|e| format!("Falló la limpieza de la intención de baja: {e}"))?
    }
    async fn settle_delete_intent(&self, id: &str) -> Result<(), String> {
        let corpus = self.0.corpus.clone();
        let state = self.0.state.clone();
        let id = id.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let sync = crate::sync::open_sync_connection(&corpus)?;
            let state = crate::sync::research_envelope::open_research_state_read_only(&state)
                .map_err(|error| error.message)?;
            crate::sync::research_capture::settle_delete_intent(&sync, &state, &id)
                .map_err(|error| error.message)
        })
        .await
        .map_err(|e| format!("Falló el asentamiento de la baja: {e}"))?
    }
}
#[tauri::command]
pub async fn research_request(
    request: Value,
    app_handle: AppHandle,
    state: State<'_, ResearchState>,
) -> Result<Value, String> {
    state.request(request, &app_handle).await
}

#[cfg(test)]
mod tests {
    use super::{capture_terminal_job, terminal_job_id, uses_model};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn rewriting_a_section_runs_with_the_model() {
        // The writer rewrites the section: without the credential the engine
        // only gets `NoModel` and the call fails before reaching it.
        assert!(uses_model("rewrite_section"));
    }

    #[test]
    fn editing_a_section_by_hand_needs_no_model() {
        assert!(!uses_model("edit_section"));
    }

    fn response(status: &str, id: &str) -> serde_json::Value {
        json!({"job": {"id": id, "status": status}})
    }

    #[test]
    fn only_terminal_job_closures_are_capture_candidates() {
        assert_eq!(
            terminal_job_id(&response("done", "job-1")),
            Some("job-1".to_string())
        );
        assert_eq!(
            terminal_job_id(&response("failed", "job-2")),
            Some("job-2".to_string())
        );
        // Active work and intermediate gates never capture.
        assert_eq!(terminal_job_id(&response("running", "job-3")), None);
        assert_eq!(terminal_job_id(&response("paused", "job-4")), None);
        assert_eq!(terminal_job_id(&response("awaiting_human", "job-5")), None);
        assert_eq!(terminal_job_id(&response("done", "")), None);
        assert_eq!(terminal_job_id(&json!({"op": "get"})), None);
    }

    fn temp_workspace(name: &str) -> tempfile::TempDir {
        tempfile::tempdir().unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    /// Real sync schema fixture at `root/corpus.sqlite` with capture enabled.
    fn corpus_db(root: &Path) -> std::path::PathBuf {
        let path = root.join("corpus.sqlite");
        let conn = rusqlite::Connection::open(&path).expect("corpus db");
        conn.execute_batch(crate::sync::test_support::SCHEMA_FIXTURE)
            .expect("schema fixture");
        crate::sync::schema::ensure_sync_schema(&conn).expect("sync schema");
        conn.execute(
            "INSERT INTO sync_meta(key, value) VALUES('capture_enabled', '1')",
            [],
        )
        .expect("capture enabled");
        path
    }

    /// Real EntropIA-Agent `estado.sqlite` at `root/estado.sqlite` with one job.
    fn research_state(root: &Path, job_id: &str, status: &str) -> std::path::PathBuf {
        let path = root.join("estado.sqlite");
        entropia_agent::estado::EstadoDb::abrir(path.to_str().expect("utf-8 path"))
            .expect("estado schema");
        let conn = rusqlite::Connection::open(&path).expect("state connection");
        conn.execute(
            "INSERT INTO jobs (id, modo, pregunta, status, close_reason, config_snapshot,
                              corpus_snapshot_id, project, corpus, created_at, updated_at)
             VALUES (?1, 'research', '¿Pregunta?', ?2, 'completed', '{}',
                     'snap-1', 'demo', 'desktop', 100, 200)",
            rusqlite::params![job_id, status],
        )
        .expect("insert job");
        path
    }

    #[test]
    fn closing_a_terminal_job_is_captured_into_the_sync_outbox() {
        let root = temp_workspace("capture-close");
        let corpus = corpus_db(root.path());
        let state = research_state(root.path(), "job-1", "done");

        capture_terminal_job(&corpus, &state, "job-1").expect("capture terminal close");

        let conn = rusqlite::Connection::open(&corpus).expect("reopen corpus");
        let entries = crate::sync::research_capture::outbox_entries(&conn).expect("outbox");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].job_id, "job-1");
        assert_eq!(entries[0].op, 'U');

        // Replaying the close capture coalesces instead of duplicating.
        capture_terminal_job(&corpus, &state, "job-1").expect("replay capture");
        let conn = rusqlite::Connection::open(&corpus).expect("reopen corpus");
        let entries = crate::sync::research_capture::outbox_entries(&conn).expect("outbox");
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn an_active_job_is_never_captured_and_the_state_error_is_surfaced() {
        let root = temp_workspace("capture-active");
        let corpus = corpus_db(root.path());
        let state = research_state(root.path(), "job-2", "running");

        let error =
            capture_terminal_job(&corpus, &state, "job-2").expect_err("an active job was captured");
        assert!(
            error.contains("not terminal"),
            "the capture-state error is surfaced, not dropped: {error}"
        );
    }

    #[test]
    fn a_capture_disabled_or_unsynced_session_is_a_no_op() {
        let root = temp_workspace("capture-off");
        let state = research_state(root.path(), "job-3", "done");

        // Sync schema present but capture disabled: silent no-op per contract.
        let path = root.path().join("corpus.sqlite");
        let conn = rusqlite::Connection::open(&path).expect("corpus db");
        conn.execute_batch(crate::sync::test_support::SCHEMA_FIXTURE)
            .expect("schema fixture");
        crate::sync::schema::ensure_sync_schema(&conn).expect("sync schema");
        drop(conn);
        capture_terminal_job(&path, &state, "job-3").expect("capture disabled is a no-op");
        let conn = rusqlite::Connection::open(&path).expect("reopen corpus");
        assert!(crate::sync::research_capture::outbox_entries(&conn)
            .expect("outbox")
            .is_empty());

        // No sync schema at all: also a no-op, never an error.
        let raw = root.path().join("raw.sqlite");
        let conn = rusqlite::Connection::open(&raw).expect("raw db");
        conn.execute("CREATE TABLE items(id TEXT)", [])
            .expect("table");
        drop(conn);
        capture_terminal_job(&raw, &state, "job-3").expect("missing sync schema is a no-op");
    }
}
