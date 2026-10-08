//! Desktop ownership and scheduling for the durable research engine.
use entropia_agent::{
    cliente_llm::{ClienteLlm, ClienteLlmOpenRouter, TurnoAgente},
    embeddings::ClienteEmbeddings,
    estado::EstadoDb,
    fuente_bibliografica::{FuenteBibliografica, PasajeBibliografico, TipoUbicacion, Ubicacion},
    recuperacion::Recuperador,
    repositorio::RepositorioSqlite,
    rerank::ClienteRerank,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, State};

use crate::rag::scope::RagLibraryRef;
use crate::rag::RagBibliographyLocation;

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

/// Un identificador de biblioteca tal como lo manda la interfaz: «user:123» o
/// «group:456». Un único formato en los dos lados: es el mismo string que el
/// job congela y que después trae cada cita bibliográfica.
fn ref_de_biblioteca(texto: &str) -> Result<RagLibraryRef, String> {
    let (tipo, id) = texto.split_once(':').ok_or_else(|| {
        format!("Biblioteca mal nombrada «{texto}»: se espera «user:123» o «group:456»")
    })?;
    if tipo.trim().is_empty() || id.trim().is_empty() {
        return Err(format!(
            "Biblioteca mal nombrada «{texto}»: se espera «user:123» o «group:456»"
        ));
    }
    Ok(RagLibraryRef {
        library_type: tipo.trim().to_string(),
        library_id: id.trim().to_string(),
    })
}

/// El pasaje de la biblioteca como lo entiende el motor de investigación: la
/// ubicación del chat (páginas o párrafos, `from..to`) se mapea a `Ubicacion`
/// y la biblioteca se nombra con el mismo string que el job congeló.
fn pasaje_del_hallazgo(hallazgo: &crate::rag::scope::PassageResult) -> PasajeBibliografico {
    PasajeBibliografico {
        chunk_id: hallazgo.chunk_id.clone(),
        item_key: hallazgo.item_key.clone(),
        titulo: hallazgo.title.clone(),
        autores: hallazgo.authors.clone(),
        anio: hallazgo.year,
        biblioteca: format!("{}:{}", hallazgo.library_type, hallazgo.library_native_id),
        texto: hallazgo.snippet.clone(),
        ubicacion: hallazgo.location.as_ref().map(ubicacion_del_chat),
    }
}

fn ubicacion_del_chat(location: &RagBibliographyLocation) -> Ubicacion {
    Ubicacion {
        tipo: if location.kind == "paragraphs" {
            TipoUbicacion::Parrafos
        } else {
            TipoUbicacion::Paginas
        },
        desde: location.from,
        hasta: location.to,
    }
}

/// La pierna bibliográfica de una investigación: la MISMA búsqueda de pasajes
/// que usa el chat de Recuperar (mismas bibliotecas, piso de similitud,
/// snippet y ubicaciones), acotada a las bibliotecas que el job congeló.
struct FuenteBiblioteca {
    corpus: PathBuf,
    bibliotecas: Vec<RagLibraryRef>,
}

impl FuenteBiblioteca {
    fn nuevo(corpus: PathBuf, bibliotecas: &[String]) -> Result<Self, String> {
        let bibliotecas = bibliotecas
            .iter()
            .map(|texto| ref_de_biblioteca(texto))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            corpus,
            bibliotecas,
        })
    }
}

impl FuenteBibliografica for FuenteBiblioteca {
    /// Cada llamada abre su conexión propia: el motor la hace desde el hilo del
    /// workflow y embeber la consulta puede ser una llamada de red. Todo error
    /// viaja como `Err(String)` y el motor lo declara como degradación.
    fn buscar(&self, consulta: &str, limite: usize) -> Result<Vec<PasajeBibliografico>, String> {
        use crate::bibliography::processing::ProfileEmbedder;

        let conn = crate::db::open::open_archive_connection(&self.corpus)?;
        let contrato = crate::processing::eligibility::resolve_effective_embedding_contract(&conn)?;
        let mut params = crate::rag::params::rag_params_from_settings(&conn);
        params.top_k = limite.clamp(1, 50);
        let embedder =
            crate::bibliography::processing::EngineProfileEmbedder::new(self.corpus.clone());
        let fuzzy = crate::nlp::fuzzy::fuzzy_enabled(&conn);
        let buscados = crate::rag::scope::passage_search(
            &conn,
            &contrato.hash,
            consulta,
            &self.bibliotecas,
            &params,
            fuzzy,
            &|text| embedder.embed(text),
        );
        if let Some(notice) = buscados.notice {
            let detalle = buscados.detail.unwrap_or_default();
            return Err(if detalle.is_empty() {
                notice.code().to_string()
            } else {
                format!("{}: {}", notice.code(), detalle)
            });
        }
        Ok(buscados.passages.iter().map(pasaje_del_hallazgo).collect())
    }
}

/// Las bibliotecas que el trabajo congeló, o `None` cuando su alcance es solo
/// corpus (el motor entonces ni consulta la fuente). En `create` se lee del
/// pedido; en los pasos siguientes, del workflow que el motor guardó.
fn bibliotecas_del_trabajo(
    op: &str,
    request: &Value,
    plan_json: Option<&str>,
) -> Result<Option<Vec<String>>, String> {
    let (alcance, crudas) = if op == "create" {
        (
            request["alcance"].as_str().unwrap_or("corpus").to_string(),
            request.get("bibliotecas").cloned().unwrap_or(Value::Null),
        )
    } else {
        let plan: Value = plan_json
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or(Value::Null);
        (
            plan["alcance"].as_str().unwrap_or("corpus").to_string(),
            plan.get("bibliotecas").cloned().unwrap_or(Value::Null),
        )
    };
    let alcance = alcance.trim();
    if matches!(alcance, "" | "corpus") {
        return Ok(None);
    }
    let bibliotecas: Vec<String> = serde_json::from_value(crudas).map_err(|e| e.to_string())?;
    if bibliotecas.is_empty() {
        return Err(
            "El alcance bibliográfico necesita al menos una biblioteca declarada (por ejemplo «user:123»)"
                .into(),
        );
    }
    Ok(Some(bibliotecas))
}

/// El workflow congelado del job, tal como lo guarda el motor en `plan_json`.
fn plan_json_del_job(state: &Path, job_id: &str) -> Option<String> {
    let conn =
        rusqlite::Connection::open_with_flags(state, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    conn.query_row(
        "SELECT plan_json FROM jobs WHERE id=?1 AND modo='research'",
        [job_id],
        |r| r.get(0),
    )
    .ok()
}

/// El modelo con el que este pedido habla con el motor.
///
/// Un trabajo ya creado lleva su modelo congelado en `plan_json.model` y ese
/// es el único que acepta el motor («El modelo cambió respecto del snapshot
/// del job»). En `create` manda el `modelo` del pedido: el motor lo valida y
/// lo congela de inmediato, y el desktop no lo toca. Sin ninguno de los dos
/// vale el ajuste de siempre: `rag_model`, después `openrouter_model`, y por
/// último el default del cliente.
fn modelo_del_trabajo(
    op: &str,
    request: &Value,
    plan_json: Option<&str>,
    rag_model: Option<String>,
    openrouter_model: Option<String>,
) -> String {
    let no_vacio = |m: &str| !m.trim().is_empty();
    if op != "create" {
        let congelado = plan_json
            .and_then(|plan| serde_json::from_str::<Value>(plan).ok())
            .and_then(|plan| plan["model"].as_str().map(str::to_string))
            .filter(|modelo| no_vacio(modelo));
        if let Some(modelo) = congelado {
            return modelo;
        }
    }
    if let Some(modelo) = request["modelo"]
        .as_str()
        .map(str::to_string)
        .filter(|modelo| no_vacio(modelo))
    {
        return modelo;
    }
    rag_model
        .filter(|modelo| no_vacio(modelo))
        .or_else(|| openrouter_model.filter(|modelo| no_vacio(modelo)))
        .unwrap_or_else(|| ClienteLlmOpenRouter::MODELO_DEFAULT.into())
}

/// La fuente bibliográfica que este pedido necesita, o `None` cuando el
/// alcance del trabajo es solo corpus. Es `procesar_con` el único que la
/// recibe: con `None` el motor declara la degradación si el alcance la pedía.
fn fuente_del_trabajo(
    request: &Value,
    corpus: &Path,
    state: &Path,
) -> Result<Option<FuenteBiblioteca>, String> {
    let op = request["op"].as_str().unwrap_or("");
    let plan = if op == "create" {
        None
    } else {
        plan_json_del_job(state, request["job_id"].as_str().unwrap_or(""))
    };
    bibliotecas_del_trabajo(op, request, plan.as_deref())?
        .map(|bibliotecas| FuenteBiblioteca::nuevo(corpus.to_path_buf(), &bibliotecas))
        .transpose()
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
            // La pierna bibliográfica, solo cuando el alcance del trabajo la
            // incluye. El motor la consulta únicamente en ese caso.
            let fuente = fuente_del_trabajo(&request, &inner.corpus, &inner.state)?;
            let bib = fuente.as_ref().map(|f| f as &dyn FuenteBibliografica);
            if model {
                let conn = rusqlite::Connection::open_with_flags(
                    &inner.corpus,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .map_err(|e| e.to_string())?;
                let key = crate::settings::get_setting(&conn, crate::settings::OPENROUTER_API_KEY)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("Configura la credencial OpenRouter en Configuración")?;
                let model = {
                    // El modelo de este pedido: el que el trabajo congeló, el
                    // que el create eligió, o el ajuste de siempre.
                    let op = request["op"].as_str().unwrap_or("");
                    let plan = if op == "create" {
                        None
                    } else {
                        plan_json_del_job(&inner.state, request["job_id"].as_str().unwrap_or(""))
                    };
                    modelo_del_trabajo(
                        op,
                        &request,
                        plan.as_deref(),
                        crate::settings::get_setting(&conn, "rag_model"),
                        crate::settings::get_setting(&conn, "openrouter_model"),
                    )
                };
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
                entropia_agent::investigacion::procesar_con(
                    &db,
                    &repo,
                    &llm,
                    Some(recuperador.as_ref()),
                    bib,
                    &inner.artifacts,
                    request,
                )
            } else {
                entropia_agent::investigacion::procesar_con(
                    &db,
                    &repo,
                    &NoModel,
                    None,
                    bib,
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
    use super::{
        bibliotecas_del_trabajo, capture_terminal_job, modelo_del_trabajo, pasaje_del_hallazgo,
        plan_json_del_job, ref_de_biblioteca, terminal_job_id, uses_model,
    };
    use crate::rag::scope::PassageResult;
    use crate::rag::RagBibliographyLocation;
    use entropia_agent::fuente_bibliografica::{TipoUbicacion, Ubicacion};
    use serde_json::json;
    use std::path::Path;

    fn hallazgo(location: Option<RagBibliographyLocation>) -> PassageResult {
        PassageResult {
            chunk_id: "chunk-1".into(),
            item_id: "item-1".into(),
            item_key: "ABCD1234".into(),
            title: "Historia de los vencidos".into(),
            authors: "Bloch, Febvre".into(),
            year: Some(1949),
            library_name: "Mi biblioteca".into(),
            library_type: "user".into(),
            library_native_id: "0".into(),
            csl_json: "{}".into(),
            snippet: "fragmento citable del pasaje".into(),
            location,
            score: 0.8,
            match_kind: "meaning".into(),
            match_terms: Vec::new(),
        }
    }

    #[test]
    fn el_pasaje_conserva_autores_anio_biblioteca_y_pagina() {
        let pasaje = pasaje_del_hallazgo(&hallazgo(Some(RagBibliographyLocation {
            kind: "pages".into(),
            from: 3,
            to: 4,
        })));
        assert_eq!(pasaje.chunk_id, "chunk-1");
        assert_eq!(pasaje.item_key, "ABCD1234");
        assert_eq!(pasaje.titulo, "Historia de los vencidos");
        assert_eq!(pasaje.autores, "Bloch, Febvre");
        assert_eq!(pasaje.anio, Some(1949));
        // La biblioteca se nombra con el MISMO string que el job congela.
        assert_eq!(pasaje.biblioteca, "user:0");
        assert_eq!(pasaje.texto, "fragmento citable del pasaje");
        assert_eq!(
            pasaje.ubicacion,
            Some(Ubicacion {
                tipo: TipoUbicacion::Paginas,
                desde: 3,
                hasta: 4
            })
        );
    }

    #[test]
    fn los_parrafos_del_chat_se_mapean_a_parrafos_y_la_ubicacion_ausente_no_se_inventa() {
        let parrafos = pasaje_del_hallazgo(&hallazgo(Some(RagBibliographyLocation {
            kind: "paragraphs".into(),
            from: 2,
            to: 2,
        })));
        assert_eq!(
            parrafos.ubicacion,
            Some(Ubicacion {
                tipo: TipoUbicacion::Parrafos,
                desde: 2,
                hasta: 2
            })
        );
        let sin_ubicacion = pasaje_del_hallazgo(&hallazgo(None));
        assert_eq!(sin_ubicacion.ubicacion, None);
    }

    #[test]
    fn las_referencias_de_biblioteca_son_tipo_dos_puntos_id() {
        let ref_user = ref_de_biblioteca("user:123").expect("user:123");
        assert_eq!(ref_user.library_type, "user");
        assert_eq!(ref_user.library_id, "123");
        let ref_group = ref_de_biblioteca("group:456").expect("group:456");
        assert_eq!(ref_group.library_type, "group");
        assert_eq!(ref_group.library_id, "456");
        // Un identificador sin tipo no se adivina: se rechaza.
        assert!(ref_de_biblioteca("solo-id").is_err());
        assert!(ref_de_biblioteca(":123").is_err());
        assert!(ref_de_biblioteca("user:").is_err());
    }

    #[test]
    fn el_create_congela_alcance_y_bibliotecas_del_pedido() {
        // El alcance bibliográfico del pedido lleva sus bibliotecas al motor.
        assert_eq!(
            bibliotecas_del_trabajo(
                "create",
                &json!({"op":"create","alcance":"biblioteca","bibliotecas":["user:123"]}),
                None
            )
            .expect("alcance biblioteca"),
            Some(vec!["user:123".to_string()])
        );
        assert_eq!(
            bibliotecas_del_trabajo(
                "create",
                &json!({"op":"create","alcance":"ambos","bibliotecas":["group:456","user:1"]}),
                None
            )
            .expect("alcance ambos"),
            Some(vec!["group:456".to_string(), "user:1".to_string()])
        );
        // «both» es el alias inglés que el motor también acepta.
        assert_eq!(
            bibliotecas_del_trabajo(
                "create",
                &json!({"op":"create","alcance":"both","bibliotecas":["user:123"]}),
                None
            )
            .expect("alias both"),
            Some(vec!["user:123".to_string()])
        );
    }

    #[test]
    fn un_trabajo_de_corpus_no_pide_fuente_bibliografica() {
        assert_eq!(
            bibliotecas_del_trabajo("create", &json!({"op":"create","alcance":"corpus"}), None)
                .expect("corpus"),
            None
        );
        // Sin alcance no hay alcance bibliográfico: es el comportamiento de siempre.
        assert_eq!(
            bibliotecas_del_trabajo("create", &json!({"op":"create"}), None).expect("sin alcance"),
            None
        );
    }

    #[test]
    fn los_pasos_siguientes_leen_el_alcance_congelado_en_el_workflow() {
        let plan = r#"{"step":3,"collections":["c1"],"model":null,"alcance":"ambos","bibliotecas":["group:9"]}"#;
        assert_eq!(
            bibliotecas_del_trabajo(
                "advance",
                &json!({"op":"advance","job_id":"job-1"}),
                Some(plan)
            )
            .expect("workflow ambos"),
            Some(vec!["group:9".to_string()])
        );
        // Un workflow anterior al alcance bibliográfico es de corpus.
        let viejo = r#"{"step":0,"collections":["c1"],"model":null}"#;
        assert_eq!(
            bibliotecas_del_trabajo(
                "advance",
                &json!({"op":"advance","job_id":"job-1"}),
                Some(viejo)
            )
            .expect("workflow viejo"),
            None
        );
        // Y sin workflow legible no se inventa alcance: se pasa sin fuente y
        // el motor declara la degradación si el job la necesitaba.
        assert_eq!(
            bibliotecas_del_trabajo("advance", &json!({"op":"advance","job_id":"job-1"}), None)
                .expect("sin workflow"),
            None
        );
    }

    #[test]
    fn el_alcance_bibliografico_sin_bibliotecas_es_un_error_del_pedido() {
        assert!(bibliotecas_del_trabajo(
            "create",
            &json!({"op":"create","alcance":"biblioteca","bibliotecas":[]}),
            None
        )
        .is_err());
    }

    #[test]
    fn el_workflow_se_lee_del_estado_del_job() {
        let root = temp_workspace("workflow-scope");
        let state = root.path().join("estado.sqlite");
        entropia_agent::estado::EstadoDb::abrir(state.to_str().expect("utf-8 path"))
            .expect("estado schema");
        let conn = rusqlite::Connection::open(&state).expect("state connection");
        conn.execute(
            "INSERT INTO jobs (id, modo, pregunta, status, close_reason, config_snapshot,
                              corpus_snapshot_id, project, corpus, plan_json, created_at, updated_at)
             VALUES ('job-1', 'research', '¿Pregunta?', 'running', NULL, '{}',
                     'snap-1', 'demo', 'desktop',
                     '{\"step\":0,\"collections\":[\"c1\"],\"model\":null,\"alcance\":\"biblioteca\",\"bibliotecas\":[\"user:7\"]}',
                     100, 200)",
            [],
        )
        .expect("insert job");
        drop(conn);

        let plan = plan_json_del_job(&state, "job-1").expect("workflow del job");
        assert_eq!(
            bibliotecas_del_trabajo(
                "advance",
                &json!({"op":"advance","job_id":"job-1"}),
                Some(&plan)
            )
            .expect("alcance del workflow"),
            Some(vec!["user:7".to_string()])
        );
        assert_eq!(plan_json_del_job(&state, "job-inexistente"), None);
    }

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

    #[test]
    fn el_modelo_de_un_trabajo_es_el_congelado_en_su_workflow() {
        // El motor rechaza cualquier modelo que no sea el del snapshot del
        // job («El modelo cambió respecto del snapshot del job»): el cliente
        // se arma SIEMPRE con el congelado.
        let workflow = r#"{"step":3,"collections":["c1"],"model":"meta/llama-3.3-70b"}"#;
        assert_eq!(
            modelo_del_trabajo(
                "advance",
                &json!({"op":"advance","job_id":"job-1"}),
                Some(workflow),
                Some("ajuste/rag".into()),
                Some("ajuste/openrouter".into())
            ),
            "meta/llama-3.3-70b"
        );
        // La reescritura de una sección es el mismo trabajo y el mismo modelo.
        assert_eq!(
            modelo_del_trabajo(
                "rewrite_section",
                &json!({"op":"rewrite_section","job_id":"job-1"}),
                Some(workflow),
                Some("ajuste/rag".into()),
                None
            ),
            "meta/llama-3.3-70b"
        );
    }

    #[test]
    fn el_create_usa_el_modelo_del_pedido() {
        // En create manda el `modelo` del pedido: el motor lo valida y lo
        // congela de inmediato. El desktop no lo toca.
        assert_eq!(
            modelo_del_trabajo(
                "create",
                &json!({"op":"create","modelo":"google/gemma-4-26b-a4b-it"}),
                None,
                Some("ajuste/rag".into()),
                Some("ajuste/openrouter".into())
            ),
            "google/gemma-4-26b-a4b-it"
        );
    }

    #[test]
    fn sin_modelo_congelado_ni_de_pedido_manda_el_ajuste_de_siempre() {
        // Un workflow con `model: null` todavía no congeló ninguno.
        let workflow = r#"{"step":0,"collections":["c1"],"model":null}"#;
        assert_eq!(
            modelo_del_trabajo(
                "advance",
                &json!({"op":"advance","job_id":"job-1"}),
                Some(workflow),
                Some("ajuste/rag".into()),
                Some("ajuste/openrouter".into())
            ),
            "ajuste/rag"
        );
        // Sin rag_model sigue openrouter_model…
        assert_eq!(
            modelo_del_trabajo(
                "create",
                &json!({"op":"create"}),
                None,
                Some("  ".into()),
                Some("ajuste/openrouter".into())
            ),
            "ajuste/openrouter"
        );
        // …y sin ninguno de los dos, el default del cliente.
        assert_eq!(
            modelo_del_trabajo(
                "advance",
                &json!({"op":"advance","job_id":"job-1"}),
                Some(workflow),
                None,
                Some("".into())
            ),
            entropia_agent::cliente_llm::ClienteLlmOpenRouter::MODELO_DEFAULT
        );
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
