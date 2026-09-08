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
use tauri::State;

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
    fn schedule(&self, id: String) -> Result<(), String> {
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
                    .execute(json!({"op":op,"job_id":id}), op == "advance")
                    .await;
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
                            let _ = this
                                .execute(json!({"op":"cancel","job_id":id}), false)
                                .await;
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
    async fn request(&self, request: Value) -> Result<Value, String> {
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
        let _lock = self.0.mutation.lock().await;
        let response = self.execute(request, false).await?;
        if response["job"]["status"] == "running" {
            let id = response["job"]["id"]
                .as_str()
                .ok_or("Motor devolvió un job sin ID")?
                .to_owned();
            self.schedule(id)?;
        }
        Ok(response)
    }
}
#[tauri::command]
pub async fn research_request(
    request: Value,
    state: State<'_, ResearchState>,
) -> Result<Value, String> {
    state.request(request).await
}
