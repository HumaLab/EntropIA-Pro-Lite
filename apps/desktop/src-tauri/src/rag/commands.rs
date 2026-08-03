//! Comandos Tauri del chat RAG: `rag_ask` + gestión de conversaciones
//! persistidas (`rag_list_conversations`, `rag_get_conversation`,
//! `rag_delete_conversation`).
//!
//! Pipeline de `rag_ask`: validación → settings + historial → recuperación
//! híbrida (en `spawn_blocking` con la conexión worker) → prompt de
//! fragmentos numerados → LLM LOCAL (Gemma) → persistencia del intercambio.
//!
//! Pro es 100% LOCAL: tanto el embedding de la consulta como la generación de
//! la respuesta corren en el equipo del usuario. El embedding usa el proveedor
//! local (BGE-M3 ONNX) vía `crate::nlp::embeddings`; la respuesta usa el motor
//! Gemma local (`crate::llm::get_or_init_local_gemma_engine` +
//! `LlmEngine::generate`), siguiendo el MISMO patrón que `run_local_gemma_ner`.
//! El branch OpenRouter existe solo cuando `llm_mode` es `openrouter`/`auto` Y
//! hay API key configurada; si no, SIEMPRE cae al motor local. El camino por
//! defecto funciona sin ninguna API key.

#[cfg(feature = "local-ml")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
#[cfg(feature = "local-ml")]
use tauri::Emitter;

use super::params::{rag_params_from_settings, RagParams, TOP_K_MAX, TOP_K_MIN};
use super::{retrieval, store};
use super::{RagAnswer, RagChatTurn, RagConversation, RagConversationSummary, RagSource};

const QUESTION_MAX_CHARS: usize = 4000;
#[cfg(feature = "local-ml")]
static LOCAL_RERANKER_DOWNLOAD_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Modo de generación de la respuesta del chat RAG. El camino por defecto es
/// `Local` (Gemma en el equipo); `OpenRouter` solo se selecciona cuando
/// `llm_mode` lo pide explícitamente Y hay credenciales (mismo idioma que
/// `ner_fallback_config`).
enum RagAnswerMode {
    #[cfg(feature = "local-ml")]
    Local,
    OpenRouter {
        api_key: String,
        model: String,
    },
}

/// Resultado de la fase bloqueante (settings + historial + recuperación).
struct RetrievalPhase {
    mode: RagAnswerMode,
    model: String,
    /// Modelo del reranker Lite (`rag_reranker_model`, ver
    /// `reranker::resolve_reranker_model`). Solo se usa en la variante
    /// OpenRouter: Pro rerankea siempre con el cross-encoder local, sin
    /// selección de modelo posible, por eso queda vacío y sin leer en ese build.
    #[cfg_attr(feature = "local-ml", allow(dead_code))]
    reranker_model: String,
    candidates: Vec<retrieval::RrfCandidate>,
    history: Vec<RagChatTurn>,
    params: RagParams,
    trace: bool,
}

/// Traza por etapa del pipeline RAG, activada con el setting `rag_trace`.
///
/// Existe porque el pipeline era una caja negra: sin duración ni cardinalidad
/// por etapa no se puede afirmar que un cambio de recuperación o de reranking
/// mejoró algo. Apagada por defecto — no queremos ruido en los logs de un
/// usuario que no está diagnosticando.
///
/// Formato de línea estable y grepeable: `[rag][trace] stage=... ms=... n=...`.
fn trace_stage(enabled: bool, stage: &str, elapsed: std::time::Duration, detail: &str) {
    if !enabled {
        return;
    }
    eprintln!(
        "[rag][trace] stage={stage} ms={} {detail}",
        elapsed.as_millis()
    );
}

/// Lee el flag de traza. Ausente o basura = apagado (mismo idioma que el resto
/// de los settings: nunca falla).
fn rag_trace_from_settings(conn: &Connection) -> bool {
    crate::settings::get_setting(conn, "rag_trace")
        .map(|value| matches!(value.trim(), "1" | "true" | "on"))
        .unwrap_or(false)
}

fn local_reranker_model_dir(db: &crate::db::state::AppDbState) -> std::path::PathBuf {
    super::reranker::resolve_local_reranker_model_dir(db.db_path.parent())
}

#[tauri::command]
pub async fn rag_reranker_model_info(
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<super::reranker::LocalRerankerModelInfo, String> {
    let model_dir = local_reranker_model_dir(&db);
    tokio::task::spawn_blocking(move || super::reranker::get_local_reranker_model_info(model_dir))
        .await
        .map_err(|_| "Local reranker model inspection failed".to_string())
}

#[tauri::command]
pub async fn rag_reranker_open_models_dir(
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<(), String> {
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = db;
        return Err("Local reranker models are unavailable in this build.".to_string());
    }

    #[cfg(feature = "local-ml")]
    {
        let models_dir = local_reranker_model_dir(&db);
        std::fs::create_dir_all(&models_dir)
            .map_err(|_| "Failed to create local reranker model directory".to_string())?;

        #[cfg(target_os = "linux")]
        std::process::Command::new("xdg-open")
            .arg(&models_dir)
            .spawn()
            .map_err(|_| "Failed to open local reranker model directory".to_string())?;
        #[cfg(target_os = "macos")]
        std::process::Command::new("open")
            .arg(&models_dir)
            .spawn()
            .map_err(|_| "Failed to open local reranker model directory".to_string())?;
        #[cfg(target_os = "windows")]
        std::process::Command::new("explorer")
            .arg(&models_dir)
            .spawn()
            .map_err(|_| "Failed to open local reranker model directory".to_string())?;
        Ok(())
    }
}

#[tauri::command]
pub async fn rag_reranker_download_model(
    db: tauri::State<'_, crate::db::state::AppDbState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let model_dir = local_reranker_model_dir(&db);

    #[cfg(not(feature = "local-ml"))]
    return super::reranker::download_local_reranker_model_files(&model_dir, &app_handle)
        .map(|_| "started".to_string());

    #[cfg(feature = "local-ml")]
    {
        if LOCAL_RERANKER_DOWNLOAD_ACTIVE.swap(true, Ordering::AcqRel) {
            return Ok("in_progress".to_string());
        }
        let download_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                super::reranker::download_local_reranker_model_files(&model_dir, &download_handle)
            })
            .await;
            LOCAL_RERANKER_DOWNLOAD_ACTIVE.store(false, Ordering::Release);

            match result {
                Ok(Ok(())) => crate::app_logs::info(
                    &app_handle,
                    "reranker/download",
                    "Local reranker installation completed",
                ),
                Ok(Err(error)) => {
                    crate::app_logs::error(
                        &app_handle,
                        "reranker/download",
                        "Local reranker installation failed",
                    );
                    let _ = app_handle.emit(
                        "reranker:download_error",
                        super::reranker::RerankerDownloadErrorPayload { error },
                    );
                }
                Err(_) => {
                    let error = "Local reranker download task failed".to_string();
                    crate::app_logs::error(
                        &app_handle,
                        "reranker/download",
                        "Local reranker download task failed",
                    );
                    let _ = app_handle.emit(
                        "reranker:download_error",
                        super::reranker::RerankerDownloadErrorPayload { error },
                    );
                }
            }
        });
        Ok("started".to_string())
    }
}

/// Responde una pregunta con RAG híbrido (vector + FTS5 fusionados con RRF)
/// sobre la base de transcripciones, citando las fuentes con `[n]`. El
/// historial se deriva de la conversación persistida (`conversation_id`) y
/// cada intercambio exitoso se guarda en SQLite; la respuesta devuelve el id
/// real de la conversación (fresco si no existía o fue borrada en vuelo).
/// Si la persistencia falla DESPUÉS de una respuesta exitosa del LLM, la
/// respuesta se devuelve igual con `conversation_id: None` — los errores de
/// validación y del LLM sí se propagan como `Err`.
///
/// La generación es LOCAL por defecto (motor Gemma): funciona sin API key.
#[tauri::command]
pub async fn rag_ask(
    question: String,
    conversation_id: Option<String>,
    top_k: Option<u8>,
    app_handle: tauri::AppHandle,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<RagAnswer, String> {
    let question = validate_question(&question)?;
    let requested_top_k = top_k;
    let db_path = db.db_path.clone();

    // Fase de recuperación: settings + embedding + SQL corren en el pool
    // bloqueante con la conexión worker (nunca en el hilo del event loop).
    let conn_arc = db.worker_conn.clone();
    let retrieval_question = question.clone();
    let history_conversation_id = conversation_id.clone();
    let embed_db_path = db_path.clone();
    let started = std::time::Instant::now();
    let phase = tokio::task::spawn_blocking(move || -> Result<RetrievalPhase, String> {
        // Paso 1: lecturas de settings + historial persistido con el lock,
        // soltándolo antes de cualquier I/O pesado (embedding/inferencia).
        let settings_started = std::time::Instant::now();
        let (mode, model, reranker_model, history, params, trace, retrieval_unit) = {
            let conn = conn_arc.lock().map_err(|e| e.to_string())?;
            let trace = rag_trace_from_settings(&conn);

            // Modo de respuesta: local por defecto (mismo idioma que
            // `ner_fallback_config`). OpenRouter solo si se seleccionó Y hay
            // clave; si falta la clave, degradamos a local en vez de fallar.
            let (mode, model) = resolve_answer_mode(&conn);

            // Modelo del reranker Lite (`rag_reranker_model`); Pro ignora este
            // valor porque siempre rerankea localmente.
            #[cfg(not(feature = "local-ml"))]
            let reranker_model = super::reranker::resolve_reranker_model(&conn);
            #[cfg(feature = "local-ml")]
            let reranker_model = String::new();

            // Parámetros RAG runtime (rag_top_k, rag_min_similarity, etc.);
            // el argumento `top_k` del comando pisa al setting si vino.
            let mut params = rag_params_from_settings(&conn);
            params.top_k = resolve_top_k(requested_top_k, params.top_k);
            let retrieval_unit = retrieval::RetrievalUnit::from_setting(
                crate::settings::get_setting(&conn, "rag_retrieval_unit").as_deref(),
            )?;

            // Historial desde la conversación persistida (vacío si el id no
            // existe o no vino); presupuesto de turnos/chars configurable.
            let history = match history_conversation_id.as_deref() {
                Some(id) => store::load_history(&conn, id, params.history_turns)?,
                None => Vec::new(),
            };

            (
                mode,
                model,
                reranker_model,
                history,
                params,
                trace,
                retrieval_unit,
            )
        };
        trace_stage(
            trace,
            "settings_history",
            settings_started.elapsed(),
            &format!("history_turns={}", history.len()),
        );

        // Paso 2 (sin lock): pierna vectorial LOCAL con degradación elegante;
        // si la config o el embedding fallan (modelo ONNX ausente, etc.),
        // seguimos solo con FTS.
        let embed_started = std::time::Instant::now();
        let query_embedding = embed_query_local(&embed_db_path, &retrieval_question);
        trace_stage(
            trace,
            "query_embedding",
            embed_started.elapsed(),
            // `vector_leg=off` es la señal de que la recuperación quedó FTS-only.
            if query_embedding.is_some() {
                "vector_leg=on"
            } else {
                "vector_leg=off"
            },
        );

        // Paso 3: re-adquirir el lock solo para la recuperación SQL.
        let retrieval_started = std::time::Instant::now();
        let conn = conn_arc.lock().map_err(|e| e.to_string())?;
        let candidates = retrieval::hybrid_retrieve_candidates(
            &conn,
            &retrieval_question,
            query_embedding.as_deref(),
            &params,
            retrieval_unit,
        )?;
        trace_stage(
            trace,
            "hybrid_retrieval",
            retrieval_started.elapsed(),
            &format!(
                "materialized={} rerank_depth={} fusion_limit={}",
                candidates.len(),
                params.rerank_depth,
                params.fusion_candidate_limit
            ),
        );

        Ok(RetrievalPhase {
            mode,
            model,
            reranker_model,
            candidates,
            history,
            params,
            trace,
        })
    })
    .await
    .map_err(|e| format!("RAG retrieval task panicked: {e}"))??;

    // Profundidad de reranking IDÉNTICA en ambas variantes (`rag_rerank_depth`,
    // recortado a top_k..=fusion_candidate_limit): antes Lite mandaba `top_k`
    // como `top_n` y Pro puntuaba la lista fusionada completa, así que el mismo
    // corpus y la misma pregunta podían devolver órdenes distintos por build.
    let rerank_depth = phase.params.rerank_depth;
    // Copiados antes del rerank: la rama local-ml mueve `phase.candidates`
    // dentro del closure.
    let trace = phase.trace;
    let materialized = phase.candidates.len();
    let rerank_started = std::time::Instant::now();

    #[cfg(not(feature = "local-ml"))]
    let candidates = {
        let RagAnswerMode::OpenRouter { api_key, .. } = &phase.mode;
        super::reranker::rerank_candidates(
            &question,
            phase.candidates,
            api_key,
            &phase.reranker_model,
            rerank_depth,
        )
        .await
    };
    #[cfg(feature = "local-ml")]
    let candidates = {
        let fallback = phase.candidates.clone();
        let rerank_question = question.clone();
        let model_dir = super::reranker::resolve_local_reranker_model_dir(db_path.parent());
        match tokio::task::spawn_blocking(move || {
            super::reranker::rerank_candidates_local(
                &rerank_question,
                phase.candidates,
                &model_dir,
                rerank_depth,
            )
        })
        .await
        {
            Ok(ranked) => ranked,
            Err(_) => {
                eprintln!(
                    "[rag] Local reranking unavailable (worker failed); preserving RRF order"
                );
                fallback
            }
        }
    };

    trace_stage(
        trace,
        "rerank",
        rerank_started.elapsed(),
        &format!(
            "in={materialized} out={} depth={rerank_depth}",
            candidates.len()
        ),
    );

    let sources_started = std::time::Instant::now();
    let sources = retrieval::build_sources(
        candidates,
        &question,
        phase.params.top_k,
        phase.params.snippet_max_chars,
        phase.params.context_max_chars,
    );
    trace_stage(
        trace,
        "build_sources",
        sources_started.elapsed(),
        &format!(
            "sources={} top_k={} context_max_chars={}",
            sources.len(),
            phase.params.top_k,
            phase.params.context_max_chars
        ),
    );

    // Sin contenido relevante: no llamamos al LLM; el frontend muestra su
    // propio mensaje de "sin resultados". El intercambio vacío también se
    // persiste para que la conversación quede completa.
    if sources.is_empty() {
        trace_stage(trace, "total", started.elapsed(), "outcome=no_sources");
        let conversation_id = persist_exchange_or_warn(
            db.worker_conn.clone(),
            conversation_id,
            question,
            String::new(),
            Vec::new(),
            phase.model.clone(),
        )
        .await;
        return Ok(empty_answer(phase.model, conversation_id));
    }

    let generation_started = std::time::Instant::now();
    let answer = generate_answer(
        &app_handle,
        &db_path,
        &phase.mode,
        &question,
        &sources,
        &phase.history,
        &phase.params,
    )
    .await?;
    trace_stage(
        trace,
        "generation",
        generation_started.elapsed(),
        &format!("answer_chars={}", answer.chars().count()),
    );

    let cited_from = sources.len();
    let sources = filter_cited_sources(sources, &answer);
    trace_stage(
        trace,
        "total",
        started.elapsed(),
        &format!("sources_built={cited_from} sources_cited={}", sources.len()),
    );

    // Paso 4: persistencia del intercambio en un cuarto scope de lock corto,
    // SIEMPRE después de la generación del LLM. Si el LLM falló, el `?` de
    // arriba ya propagó el error sin persistir. Si la PERSISTENCIA falla, la
    // respuesta ya computada no se descarta: se devuelve con
    // `conversation_id: None`.
    let conversation_id = persist_exchange_or_warn(
        db.worker_conn.clone(),
        conversation_id,
        question,
        answer.clone(),
        sources.clone(),
        phase.model.clone(),
    )
    .await;

    Ok(RagAnswer {
        answer,
        sources,
        model: phase.model,
        conversation_id,
    })
}

/// Genera la respuesta del LLM según el modo resuelto. El camino por defecto
/// (`Local`) usa el motor Gemma del equipo siguiendo el patrón de
/// `run_local_gemma_ner`: abre su propia conexión desde `db_path`, obtiene el
/// engine cacheado, lockea su mutex, presupuesta el contexto contra `n_ctx`
/// (truncado/chunking) y genera. El branch `OpenRouter` solo corre cuando se
/// seleccionó explícitamente y hay clave.
// `app_handle` and `db_path` are consumed only by the local-ml RagAnswerMode::Local
// arm. In lean only the OpenRouter arm runs, so allow them to be unused rather than
// renaming the parameters (keeps the signature and call sites stable).
#[cfg_attr(not(feature = "local-ml"), allow(unused_variables))]
async fn generate_answer(
    app_handle: &tauri::AppHandle,
    db_path: &std::path::Path,
    mode: &RagAnswerMode,
    question: &str,
    sources: &[RagSource],
    history: &[RagChatTurn],
    params: &RagParams,
) -> Result<String, String> {
    match mode {
        RagAnswerMode::OpenRouter { api_key, model } => {
            // Branch opcional gateado por `llm_mode`. El prompt remoto usa el
            // texto crudo (sin wrapping Gemma).
            let context = format_fragments(sources);
            let history_block =
                format_history(history, params.history_turns, params.history_turn_max_chars);
            let prompt = crate::llm::prompt::raw_rag_answer(question, &context, &history_block);
            let client =
                crate::llm::openrouter::OpenRouterClient::new(api_key.clone(), model.clone());
            let generation = crate::llm::generation::OpenRouterGenerationParams::provider_defaults(
                params.max_tokens,
                params.temperature,
            );
            client.generate(&prompt, &generation).await
        }
        #[cfg(feature = "local-ml")]
        RagAnswerMode::Local => {
            // Camino local de Pro: motor LLM en el equipo. Mismo patrón que
            // `run_local_gemma_ner`.
            let app_handle = app_handle.clone();
            let db_path = db_path.to_path_buf();
            let question = question.to_string();
            let sources = sources.to_vec();
            let history = history.to_vec();
            let params = *params;
            tokio::task::spawn_blocking(move || -> Result<String, String> {
                let conn = Connection::open(&db_path).map_err(|error| {
                    format!("Failed to open DB for local RAG generation: {error}")
                })?;
                let engine =
                    crate::llm::get_or_init_local_gemma_engine(&conn, &db_path, &app_handle)?;
                // Ventana local del chat RAG: 131K tokens por defecto (setting
                // `local_rag_n_ctx`), con reserva de salida y margen para el
                // presupuesto de evidencia. Ventanas chicas conservan el
                // presupuesto legacy.
                let n_ctx = crate::llm::local_rag_n_ctx_from_settings(&conn);
                let max_tokens = if n_ctx >= crate::llm::LOCAL_RAG_LARGE_CTX_THRESHOLD {
                    params
                        .max_tokens
                        .min(crate::llm::LOCAL_RAG_MAX_OUTPUT_TOKENS)
                } else {
                    params.max_tokens
                };
                let engine = engine
                    .lock()
                    .map_err(|error| format!("Local LLM engine lock poisoned: {error}"))?;

                // Presupuesto de contexto contra el `n_ctx` efectivo: los
                // fragmentos recuperados pueden ser grandes y desbordar la
                // ventana del modelo local. Construimos el prompt completo y,
                // como red de seguridad final, lo truncamos al presupuesto de
                // tokens (evidencia + historial ≈ n_ctx - salida - margen).
                let prompt = build_local_rag_prompt(
                    n_ctx, max_tokens, &question, &sources, &history, &params,
                );
                engine.generate_chat_with_ctx(
                    &prompt,
                    max_tokens,
                    n_ctx,
                    params.temperature,
                    "[rag][local]",
                )
            })
            .await
            .map_err(|e| format!("Local RAG generation task panicked: {e}"))?
        }
    }
}

/// Igual que `persist_exchange_blocking`, pero NUNCA propaga el error: una
/// respuesta ya obtenida del LLM no se descarta porque falló la persistencia.
/// Loguea el error y devuelve `None` (el frontend no adopta ningún id).
async fn persist_exchange_or_warn(
    conn_arc: Arc<Mutex<Connection>>,
    conversation_id: Option<String>,
    question: String,
    answer: String,
    sources: Vec<RagSource>,
    model: String,
) -> Option<String> {
    match persist_exchange_blocking(conn_arc, conversation_id, question, answer, sources, model)
        .await
    {
        Ok(id) => Some(id),
        Err(error) => {
            eprintln!(
                "[rag] No se pudo persistir el intercambio (la respuesta se devuelve igual): {error}"
            );
            None
        }
    }
}

/// Persiste el intercambio pregunta/respuesta en el pool bloqueante con un
/// lock corto sobre la conexión worker. Devuelve el id real de la
/// conversación (fresco si no existía).
async fn persist_exchange_blocking(
    conn_arc: Arc<Mutex<Connection>>,
    conversation_id: Option<String>,
    question: String,
    answer: String,
    sources: Vec<RagSource>,
    model: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut conn = conn_arc.lock().map_err(|e| e.to_string())?;
        store::persist_exchange(
            &mut conn,
            conversation_id.as_deref(),
            &question,
            &answer,
            &sources,
            &model,
            store::now_millis(),
        )
    })
    .await
    .map_err(|e| format!("RAG persistence task panicked: {e}"))?
}

/// Lista las conversaciones RAG persistidas, más reciente primero.
#[tauri::command]
pub async fn rag_list_conversations(
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<Vec<RagConversationSummary>, String> {
    let conn_arc = db.worker_conn.clone();
    tokio::task::spawn_blocking(move || -> Result<Vec<RagConversationSummary>, String> {
        let conn = conn_arc.lock().map_err(|e| e.to_string())?;
        store::list_conversations(&conn)
    })
    .await
    .map_err(|e| format!("RAG list task panicked: {e}"))?
}

/// Carga una conversación persistida completa, con mensajes y fuentes.
#[tauri::command]
pub async fn rag_get_conversation(
    conversation_id: String,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<RagConversation, String> {
    let conn_arc = db.worker_conn.clone();
    tokio::task::spawn_blocking(move || -> Result<RagConversation, String> {
        let conn = conn_arc.lock().map_err(|e| e.to_string())?;
        store::get_conversation(&conn, &conversation_id)
    })
    .await
    .map_err(|e| format!("RAG get task panicked: {e}"))?
}

/// Elimina una conversación persistida y sus mensajes. Borrar un id
/// inexistente es un no-op exitoso.
#[tauri::command]
pub async fn rag_delete_conversation(
    conversation_id: String,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<(), String> {
    let conn_arc = db.worker_conn.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut conn = conn_arc.lock().map_err(|e| e.to_string())?;
        store::delete_conversation(&mut conn, &conversation_id)
    })
    .await
    .map_err(|e| format!("RAG delete task panicked: {e}"))?
}

/// Embedding LOCAL de la consulta del usuario, con degradación elegante a
/// FTS-only. Abre una conexión propia desde `db_path` para leer la config de
/// embeddings, inicializa el proveedor LOCAL (BGE-M3 ONNX) y embebe el texto.
/// Cualquier fallo (modelo ausente, proveedor API sin clave, error de
/// inferencia) se loguea y devuelve `None` — la recuperación sigue solo con
/// FTS. NUNCA contacta la nube en el camino por defecto.
fn embed_query_local(db_path: &std::path::Path, question: &str) -> Option<Vec<f32>> {
    let result = (|| -> Result<Vec<f32>, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open DB for RAG query embedding: {e}"))?;
        let config = crate::nlp::embeddings::config_from_settings(&conn)?;
        // Engine CACHEADO: reconstruir la sesión ONNX de BGE-M3 en cada pregunta
        // era el costo fijo más grande del camino de recuperación, y además
        // tiraba el cache interno de texto→embedding del engine.
        let engine = crate::nlp::embeddings::get_or_init_engine(config)?;
        engine.embed_text(question)
    })();

    match result {
        Ok(embedding) => Some(embedding),
        Err(error) => {
            eprintln!("[rag] Pierna vectorial deshabilitada (se usa solo FTS): {error}");
            None
        }
    }
}

/// Resuelve el modo de generación de la respuesta desde `llm_mode` (mismo
/// idioma que `ner_fallback_config`). Por defecto `Local`. `openrouter`/`auto`
/// piden OpenRouter, pero SOLO si hay clave configurada; si falta, se degrada
/// a `Local` para que el chat siga funcionando sin la nube.
///
/// Devuelve `(modo, model_string)`: el `model_string` es el que se persiste y
/// se devuelve al frontend (filename local o id del modelo remoto).
///
/// Sin el feature `local-ml` no hay motor local: el chat RAG es solo OpenRouter
/// (igual que EntropIA Lite). Devuelve siempre `OpenRouter`; una api key vacía
/// la captura aguas abajo `OpenRouterClient` con un error claro.
#[cfg(not(feature = "local-ml"))]
fn resolve_answer_mode(conn: &Connection) -> (RagAnswerMode, String) {
    let api_key = crate::settings::get_setting(conn, "openrouter_api_key")
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let model = resolve_answer_model(conn);
    (
        RagAnswerMode::OpenRouter {
            api_key,
            model: model.clone(),
        },
        model,
    )
}

/// Devuelve `(modo, model_string)`: el `model_string` es el que se persiste y
/// se devuelve al frontend (filename local o id del modelo remoto).
#[cfg(feature = "local-ml")]
fn resolve_answer_mode(conn: &Connection) -> (RagAnswerMode, String) {
    let wants_openrouter = matches!(
        crate::settings::get_setting(conn, "llm_mode")
            .unwrap_or_else(|| "local".to_string())
            .as_str(),
        "openrouter" | "auto"
    );

    if wants_openrouter {
        if let Some(api_key) = crate::settings::get_setting(conn, "openrouter_api_key")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            let model = resolve_answer_model(conn);
            return (
                RagAnswerMode::OpenRouter {
                    api_key,
                    model: model.clone(),
                },
                model,
            );
        }
        eprintln!(
            "[rag] llm_mode pide OpenRouter pero no hay API key; usando el motor Gemma local"
        );
    }

    (RagAnswerMode::Local, crate::llm::MODEL_FILENAME.to_string())
}

/// Modelo OpenRouter del chat RAG, con herencia explícita en tres niveles:
///
/// 1. `rag_model` — override propio de la solapa RAG Params.
/// 2. `openrouter_model` — el modelo general de Configuración (herencia por
///    defecto: sin override, el chat usa el mismo modelo que el resto de la app).
/// 3. `settings::DEFAULT_OPENROUTER_MODEL` — el default del producto.
///
/// Valores vacíos o solo-espacios en cualquiera de los dos settings NO cuentan
/// como elección: caen al nivel siguiente. Por eso borrar el campo en la UI
/// vuelve a heredar el modelo general en vez de mandar un id vacío al proveedor.
fn resolve_answer_model(conn: &Connection) -> String {
    ["rag_model", "openrouter_model"]
        .into_iter()
        .find_map(|key| {
            crate::settings::get_setting(conn, key)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| crate::settings::DEFAULT_OPENROUTER_MODEL.to_string())
}

/// Valida la pregunta del usuario: trim, no vacía y máximo 4000 caracteres
/// (conteo por chars, no bytes).
fn validate_question(question: &str) -> Result<String, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err(
            "La pregunta no puede estar vacía. Escribí una consulta para buscar en tus documentos."
                .to_string(),
        );
    }
    if question.chars().count() > QUESTION_MAX_CHARS {
        return Err(format!(
            "La pregunta es demasiado larga (máximo {QUESTION_MAX_CHARS} caracteres)."
        ));
    }
    Ok(question)
}

/// top_k final: el argumento del comando (clamp 1..=20) pisa el setting
/// `rag_top_k`; sin argumento queda el valor del setting (ya validado por
/// `rag_params_from_settings`).
fn resolve_top_k(requested: Option<u8>, settings_top_k: usize) -> usize {
    match requested {
        Some(value) => usize::from(value).clamp(TOP_K_MIN, TOP_K_MAX),
        None => settings_top_k,
    }
}

/// Respuesta vacía cuando la recuperación no encontró fuentes (sin LLM).
/// `conversation_id` es `None` si la persistencia del intercambio falló.
fn empty_answer(model: String, conversation_id: Option<String>) -> RagAnswer {
    RagAnswer {
        answer: String::new(),
        sources: Vec::new(),
        model,
        conversation_id,
    }
}

/// Prompt completo para el motor Gemma LOCAL, presupuestado contra `n_ctx`.
///
/// Construimos el prompt crudo con TODOS los fragmentos recuperados (ya
/// limitados por `context_max_chars` en retrieval) y, como red de seguridad
/// final, lo truncamos al presupuesto de tokens del modelo. En ventanas
/// grandes (>= 64K) el presupuesto descuenta además el margen de
/// sistema/estimación (`LOCAL_RAG_MARGIN_TOKENS`), dejando
/// evidencia + historial ≈ n_ctx - salida - margen. Devuelve instrucciones
/// crudas; el motor aplica el chat template provisto por el modelo.
#[cfg(feature = "local-ml")]
fn build_local_rag_prompt(
    n_ctx: u32,
    max_tokens: i32,
    question: &str,
    sources: &[RagSource],
    history: &[RagChatTurn],
    params: &RagParams,
) -> String {
    let context = format_fragments(sources);
    let history_block =
        format_history(history, params.history_turns, params.history_turn_max_chars);
    let budget_ctx = if n_ctx >= crate::llm::LOCAL_RAG_LARGE_CTX_THRESHOLD {
        n_ctx - crate::llm::LOCAL_RAG_MARGIN_TOKENS
    } else {
        n_ctx
    };
    fit_local_rag_instruction(budget_ctx, max_tokens, question, &context, &history_block)
}

/// Fits only optional RAG material. The instruction/citation scaffold and the
/// complete current question are mandatory; if the requested output reserve
/// leaves no room for them, generation reduces that reserve after tokenization.
#[cfg(feature = "local-ml")]
fn fit_local_rag_instruction(
    n_ctx: u32,
    max_tokens: i32,
    question: &str,
    context: &str,
    history: &str,
) -> String {
    const CHARS_PER_TOKEN_ESTIMATE: usize = 3;
    const TEMPLATE_OVERHEAD_TOKENS: i64 = 128;

    let mandatory = crate::llm::prompt::raw_rag_answer(question, "", "");
    let input_tokens = i64::from(n_ctx) - i64::from(max_tokens) - TEMPLATE_OVERHEAD_TOKENS;
    if input_tokens <= 0 {
        return mandatory;
    }

    let input_budget_chars = input_tokens as usize * CHARS_PER_TOKEN_ESTIMATE;
    let complete = crate::llm::prompt::raw_rag_answer(question, context, history);
    if complete.chars().count() <= input_budget_chars {
        return complete;
    }

    let mandatory_chars = mandatory.chars().count();
    if mandatory_chars >= input_budget_chars {
        return mandatory;
    }

    let context_budget = input_budget_chars - mandatory_chars;
    let fitted_context: String = context.chars().take(context_budget).collect();
    let without_history = crate::llm::prompt::raw_rag_answer(question, &fitted_context, "");
    let remaining_chars = input_budget_chars.saturating_sub(without_history.chars().count());
    if history.trim().is_empty() || remaining_chars == 0 {
        return without_history;
    }

    let one_history_char = crate::llm::prompt::raw_rag_answer(question, &fitted_context, "x");
    let history_wrapper_chars = one_history_char
        .chars()
        .count()
        .saturating_sub(without_history.chars().count() + 1);
    let history_budget = remaining_chars.saturating_sub(history_wrapper_chars);
    if history_budget == 0 {
        return without_history;
    }

    let fitted_history: String = history.chars().take(history_budget).collect();
    crate::llm::prompt::raw_rag_answer(question, &fitted_context, &fitted_history)
}

/// Fragmentos con el formato `[n] «item_title» (collection_name):\n{snippet}`.
fn format_fragments(sources: &[RagSource]) -> String {
    sources
        .iter()
        .map(|source| {
            format!(
                "[{}] «{}» ({}):\n{}",
                source.index, source.item_title, source.collection_name, source.snippet
            )
        })
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// Últimos `max_turns` turnos, cada uno truncado a `turn_max_chars` (por
/// chars, no bytes), con prefijo Usuario:/Asistente:.
fn format_history(history: &[RagChatTurn], max_turns: usize, turn_max_chars: usize) -> String {
    history
        .iter()
        .skip(history.len().saturating_sub(max_turns))
        .filter(|turn| !turn.content.trim().is_empty())
        .map(|turn| {
            let prefix = if turn.role == "assistant" {
                "Asistente"
            } else {
                "Usuario"
            };
            let content: String = turn.content.trim().chars().take(turn_max_chars).collect();
            format!("{prefix}: {content}")
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Filtra las fuentes para incluir SOLO las que el LLM citó en la respuesta
/// (las que aparecen como `[n]` en el texto). Los índices inválidos o que no
/// corresponden a una fuente recuperada no exponen ninguna fuente.
fn filter_cited_sources(sources: Vec<RagSource>, answer: &str) -> Vec<RagSource> {
    let cited: std::collections::HashSet<u32> = extract_citation_indices(answer);
    sources
        .into_iter()
        .filter(|source| cited.contains(&source.index))
        .collect()
}

/// Extrae los índices `[n]` presentes en el texto de la respuesta.
fn extract_citation_indices(text: &str) -> std::collections::HashSet<u32> {
    let mut indices = std::collections::HashSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end].is_ascii_digit() {
                end += 1;
            }
            if end > start && end < chars.len() && chars[end] == ']' {
                let num_str: String = chars[start..end].iter().collect();
                if let Ok(n) = num_str.parse::<u32>() {
                    if n > 0 {
                        indices.insert(n);
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    indices
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, content: &str) -> RagChatTurn {
        RagChatTurn {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    // ── Traza por etapa (F11) ────────────────────────────────────────────────

    fn trace_conn(value: Option<&str>) -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("app_settings schema creation failed");
        if let Some(value) = value {
            conn.execute(
                "INSERT INTO app_settings(key, value) VALUES ('rag_trace', ?1)",
                rusqlite::params![value],
            )
            .expect("setting insert failed");
        }
        conn
    }

    #[test]
    fn rag_trace_is_off_unless_explicitly_enabled() {
        // Un usuario que no está diagnosticando no debería ver ruido.
        assert!(!rag_trace_from_settings(&trace_conn(None)));
        assert!(!rag_trace_from_settings(&trace_conn(Some("0"))));
        assert!(!rag_trace_from_settings(&trace_conn(Some("false"))));
        assert!(!rag_trace_from_settings(&trace_conn(Some(""))));
        // Basura tampoco enciende (mismo idioma que el resto de los settings).
        assert!(!rag_trace_from_settings(&trace_conn(Some("quizás"))));
    }

    #[test]
    fn rag_trace_accepts_the_documented_truthy_values() {
        for value in ["1", "true", "on", "  true  "] {
            assert!(
                rag_trace_from_settings(&trace_conn(Some(value))),
                "value={value} should enable tracing"
            );
        }
    }

    #[test]
    fn rag_trace_defaults_off_when_settings_table_is_missing() {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        assert!(!rag_trace_from_settings(&conn));
    }

    #[test]
    fn trace_stage_is_a_noop_when_disabled() {
        // No podemos capturar stderr acá; lo que este test protege es que la
        // ruta apagada no entre en pánico ni formatee nada.
        trace_stage(false, "stage", std::time::Duration::from_millis(5), "n=1");
        trace_stage(true, "stage", std::time::Duration::from_millis(5), "n=1");
    }

    #[cfg(feature = "local-ml")]
    fn source(index: u32, title: &str, collection: &str, snippet: &str) -> RagSource {
        RagSource {
            index,
            asset_id: format!("asset-{index}"),
            item_id: format!("item-{index}"),
            item_title: title.to_string(),
            collection_id: "col-1".to_string(),
            collection_name: collection.to_string(),
            snippet: snippet.to_string(),
            score: 1.0 / f64::from(index),
            start_seconds: None,
            end_seconds: None,
        }
    }

    #[test]
    fn resolve_top_k_defaults_and_clamps() {
        // Sin argumento: pasa el valor del setting tal cual.
        assert_eq!(resolve_top_k(None, 6), 6);
        assert_eq!(resolve_top_k(None, 13), 13);
        // Con argumento: pisa el setting, clamp 1..=20.
        assert_eq!(resolve_top_k(Some(0), 6), 1);
        assert_eq!(resolve_top_k(Some(3), 6), 3);
        assert_eq!(resolve_top_k(Some(15), 6), 15);
        assert_eq!(resolve_top_k(Some(20), 6), 20);
        assert_eq!(resolve_top_k(Some(200), 6), 20);
    }

    #[test]
    fn validate_question_rejects_empty_and_whitespace() {
        assert!(validate_question("").is_err());
        assert!(validate_question("   \n\t ").is_err());
    }

    #[test]
    fn validate_question_trims_and_accepts_normal_input() {
        assert_eq!(
            validate_question("  ¿Qué pasó en mayo?  ").as_deref(),
            Ok("¿Qué pasó en mayo?")
        );
    }

    #[test]
    fn validate_question_caps_at_4000_chars_not_bytes() {
        // Multibyte char: 4000 chars son 8000 bytes — el límite es por chars.
        let exactly_max = "á".repeat(4000);
        assert!(validate_question(&exactly_max).is_ok());

        let over_max = "á".repeat(4001);
        let error = validate_question(&over_max).expect_err("4001 chars must be rejected");
        assert_eq!(
            error,
            "La pregunta es demasiado larga (máximo 4000 caracteres)."
        );
    }

    fn conn_with_settings(pairs: &[(&str, &str)]) -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("app_settings schema creation failed");
        for (key, value) in pairs {
            conn.execute(
                "INSERT INTO app_settings(key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .expect("setting insert failed");
        }
        conn
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn resolve_answer_mode_defaults_to_local_without_settings() {
        // Sin `llm_mode`: camino LOCAL, model = filename del modelo Gemma.
        let conn = conn_with_settings(&[]);
        let (mode, model) = resolve_answer_mode(&conn);
        assert!(matches!(mode, RagAnswerMode::Local));
        assert_eq!(model, crate::llm::MODEL_FILENAME);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn resolve_answer_mode_local_is_default_explicit() {
        let conn = conn_with_settings(&[("llm_mode", "local")]);
        let (mode, model) = resolve_answer_mode(&conn);
        assert!(matches!(mode, RagAnswerMode::Local));
        assert_eq!(model, crate::llm::MODEL_FILENAME);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn resolve_answer_mode_openrouter_without_key_degrades_to_local() {
        // `llm_mode=openrouter` pero SIN api key: debe degradar a LOCAL,
        // nunca fallar ni mandar requests con clave vacía.
        let conn = conn_with_settings(&[("llm_mode", "openrouter")]);
        let (mode, model) = resolve_answer_mode(&conn);
        assert!(
            matches!(mode, RagAnswerMode::Local),
            "missing key must fall back to local"
        );
        assert_eq!(model, crate::llm::MODEL_FILENAME);
    }

    #[test]
    fn resolve_answer_mode_openrouter_with_key_selects_remote() {
        let conn = conn_with_settings(&[
            ("llm_mode", "auto"),
            ("openrouter_api_key", "  sk-or-123  "),
            ("openrouter_model", "  vendor/model-x  "),
        ]);
        let (mode, model) = resolve_answer_mode(&conn);
        match mode {
            RagAnswerMode::OpenRouter { api_key, model: m } => {
                assert_eq!(api_key, "sk-or-123", "api key is trimmed");
                assert_eq!(m, "vendor/model-x", "model is trimmed");
            }
            #[cfg(feature = "local-ml")]
            RagAnswerMode::Local => panic!("configured OpenRouter mode must be selected"),
        }
        assert_eq!(model, "vendor/model-x");
    }

    #[test]
    fn resolve_answer_mode_openrouter_with_key_uses_default_model_when_unset() {
        let conn = conn_with_settings(&[
            ("llm_mode", "openrouter"),
            ("openrouter_api_key", "sk-or-999"),
        ]);
        let (mode, model) = resolve_answer_mode(&conn);
        assert!(matches!(mode, RagAnswerMode::OpenRouter { .. }));
        assert_eq!(model, crate::settings::DEFAULT_OPENROUTER_MODEL);
    }

    #[test]
    fn resolve_answer_model_inherits_the_general_model_without_rag_override() {
        let conn = conn_with_settings(&[("openrouter_model", "  vendor/general  ")]);
        assert_eq!(resolve_answer_model(&conn), "vendor/general");
    }

    #[test]
    fn resolve_answer_model_prefers_the_rag_override_over_the_general_model() {
        let conn = conn_with_settings(&[
            ("openrouter_model", "vendor/general"),
            ("rag_model", "  vendor/rag-only  "),
        ]);
        assert_eq!(resolve_answer_model(&conn), "vendor/rag-only");
    }

    #[test]
    fn resolve_answer_model_blank_override_falls_back_to_the_general_model() {
        let conn =
            conn_with_settings(&[("openrouter_model", "vendor/general"), ("rag_model", "   ")]);
        assert_eq!(resolve_answer_model(&conn), "vendor/general");
    }

    #[test]
    fn resolve_answer_model_without_settings_uses_the_product_default() {
        let conn = conn_with_settings(&[]);
        assert_eq!(
            resolve_answer_model(&conn),
            crate::settings::DEFAULT_OPENROUTER_MODEL
        );
    }

    #[test]
    fn resolve_answer_mode_openrouter_uses_the_rag_model_override() {
        let conn = conn_with_settings(&[
            ("llm_mode", "openrouter"),
            ("openrouter_api_key", "sk-or-999"),
            ("openrouter_model", "vendor/general"),
            ("rag_model", "vendor/rag-only"),
        ]);
        let (mode, model) = resolve_answer_mode(&conn);
        match mode {
            RagAnswerMode::OpenRouter { model: m, .. } => assert_eq!(m, "vendor/rag-only"),
            #[cfg(feature = "local-ml")]
            RagAnswerMode::Local => panic!("configured OpenRouter mode must be selected"),
        }
        assert_eq!(model, "vendor/rag-only");
    }

    #[test]
    fn format_history_keeps_last_six_turns_and_truncates_content() {
        let mut history = Vec::new();
        for i in 0..8 {
            history.push(turn(
                if i % 2 == 0 { "user" } else { "assistant" },
                &format!("turno {i}"),
            ));
        }
        history.push(turn("user", &"x".repeat(600)));

        let formatted = format_history(&history, 6, 500);
        let lines: Vec<&str> = formatted.lines().collect();
        assert_eq!(lines.len(), 6, "only the last 6 turns survive");
        assert!(!formatted.contains("turno 0"));
        assert!(!formatted.contains("turno 2"));
        assert!(formatted.contains("Usuario: turno 4"));
        assert!(formatted.contains("Asistente: turno 7"));

        let last = lines.last().expect("history should have lines");
        assert!(last.starts_with("Usuario: "));
        assert_eq!(
            last.chars().count(),
            "Usuario: ".chars().count() + 500,
            "content is truncated to 500 chars"
        );
    }

    #[test]
    fn format_history_empty_returns_empty_string() {
        assert!(format_history(&[], 6, 500).is_empty());
    }

    #[test]
    fn format_history_respects_configured_turns_and_chars() {
        let history = vec![
            turn("user", "primer turno"),
            turn("assistant", "segundo turno"),
            turn("user", &"y".repeat(200)),
        ];
        let formatted = format_history(&history, 2, 100);
        let lines: Vec<&str> = formatted.lines().collect();
        assert_eq!(lines.len(), 2, "only the last 2 turns survive");
        assert!(!formatted.contains("primer turno"));
        let last = lines.last().expect("history should have lines");
        assert_eq!(last.chars().count(), "Usuario: ".chars().count() + 100);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn build_local_rag_prompt_contains_numbered_fragments_history_and_question() {
        let sources = vec![
            source(1, "Acta del Cabildo", "Archivo General", "fragmento uno"),
            source(2, "Crónica", "Hemeroteca", "fragmento dos"),
        ];
        let history = vec![turn("user", "hola"), turn("assistant", "buenas")];
        let prompt = build_local_rag_prompt(
            4096,
            1500,
            "¿Qué pasó en mayo?",
            &sources,
            &history,
            &RagParams::default(),
        );

        assert!(prompt.contains("[1] «Acta del Cabildo» (Archivo General):\nfragmento uno"));
        assert!(prompt.contains("[2] «Crónica» (Hemeroteca):\nfragmento dos"));
        assert!(prompt.contains("Usuario: hola"));
        assert!(prompt.contains("Asistente: buenas"));
        assert!(prompt.contains("Pregunta: ¿Qué pasó en mayo?"));
        assert!(prompt.contains("[n]"), "citation instructions present");
        assert!(
            !prompt.contains("<start_of_turn>"),
            "RAG must pass raw instructions to the model-provided chat template"
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn build_local_rag_prompt_without_history_omits_history_block() {
        let sources = vec![source(1, "Acta", "Archivo", "fragmento")];
        let prompt =
            build_local_rag_prompt(4096, 1500, "pregunta", &sources, &[], &RagParams::default());
        assert!(!prompt.contains("Conversación previa"));
        assert!(prompt.contains("Pregunta: pregunta"));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn build_local_rag_prompt_large_ctx_keeps_evidence_beyond_legacy_28k_prefilter() {
        // Con la ventana de 131K ya no existe el prefilter de 28K chars: un
        // fragmento de 40K chars entra completo al prompt (cabe en el
        // presupuesto de evidencia de ~116K tokens).
        let big_snippet = "e".repeat(40_000);
        let sources = vec![source(1, "Acta", "Archivo", &big_snippet)];
        let prompt = build_local_rag_prompt(
            crate::llm::DEFAULT_LOCAL_RAG_N_CTX,
            crate::llm::LOCAL_RAG_MAX_OUTPUT_TOKENS,
            "pregunta",
            &sources,
            &[],
            &RagParams::default(),
        );
        assert!(prompt.contains(&big_snippet));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn build_local_rag_prompt_large_ctx_truncates_to_evidence_budget() {
        // Presupuesto: 131072 - 6144 (margen) - 8192 (salida) - 128 (template)
        // = 116608 tokens ≈ 349824 chars. Un contexto de 500K chars se trunca
        // por debajo de ese techo, char-safe.
        let big_snippet = "ñ".repeat(500_000);
        let sources = vec![source(1, "Acta", "Archivo", &big_snippet)];
        let prompt = build_local_rag_prompt(
            crate::llm::DEFAULT_LOCAL_RAG_N_CTX,
            crate::llm::LOCAL_RAG_MAX_OUTPUT_TOKENS,
            "pregunta",
            &sources,
            &[],
            &RagParams::default(),
        );
        let expected_budget_chars = (crate::llm::DEFAULT_LOCAL_RAG_N_CTX
            - crate::llm::LOCAL_RAG_MARGIN_TOKENS) as i32
            - crate::llm::LOCAL_RAG_MAX_OUTPUT_TOKENS
            - 128;
        let expected_budget_chars = expected_budget_chars as usize * 3;
        assert!(prompt.chars().count() <= expected_budget_chars + 64);
        assert!(std::str::from_utf8(prompt.as_bytes()).is_ok());
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn build_local_rag_prompt_small_ctx_keeps_legacy_budget_without_margin() {
        // Ventana chica (4096): sin margen de 131K — el presupuesto legacy es
        // n_ctx - max_tokens - 128. Un contexto de 20K chars se trunca a eso.
        let big_snippet = "a".repeat(20_000);
        let sources = vec![source(1, "Acta", "Archivo", &big_snippet)];
        let prompt =
            build_local_rag_prompt(4096, 1500, "pregunta", &sources, &[], &RagParams::default());
        // (4096 - 1500 - 128) tokens * 3 chars/token = 7404 chars + wrap.
        assert!(prompt.chars().count() <= 7404 + 64);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn build_local_rag_prompt_preserves_complete_question_when_output_reserve_fills_context() {
        let question = "¿Qué relación tuvo José Álvarez con la comisión de 1961?";
        let sources = vec![source(1, "Acta", "Archivo", &"ñ".repeat(20_000))];
        let history = vec![turn("assistant", &"á".repeat(4_000))];

        let prompt = build_local_rag_prompt(
            4096,
            4096,
            question,
            &sources,
            &history,
            &RagParams::default(),
        );

        assert!(prompt.contains("Reglas obligatorias:"));
        assert!(prompt.contains("usando el formato [n]"));
        assert!(prompt.contains(&format!("Pregunta: {question}")));
        assert!(std::str::from_utf8(prompt.as_bytes()).is_ok());
    }

    #[test]
    fn empty_answer_skips_llm_and_returns_empty_payload() {
        let answer = empty_answer("modelo-x".to_string(), Some("conv-1".to_string()));
        assert!(answer.answer.is_empty());
        assert!(answer.sources.is_empty());
        assert_eq!(answer.model, "modelo-x");
        assert_eq!(answer.conversation_id.as_deref(), Some("conv-1"));
    }

    #[test]
    fn empty_answer_carries_none_when_persistence_failed() {
        let answer = empty_answer("modelo-x".to_string(), None);
        assert!(answer.answer.is_empty());
        assert_eq!(answer.conversation_id, None);
    }

    /// Conexión SIN las tablas RAG: fuerza el fallo de persistencia.
    fn conn_without_rag_tables() -> Arc<Mutex<Connection>> {
        Arc::new(Mutex::new(
            Connection::open_in_memory().expect("in-memory DB failed"),
        ))
    }

    #[tokio::test]
    async fn persist_failure_after_llm_answer_returns_none_instead_of_error() {
        // La respuesta del LLM ya está computada: si la persistencia falla
        // (acá, tablas ausentes), el intercambio se pierde pero la respuesta
        // se devuelve igual con `None` — nunca un `Err`.
        let conversation_id = persist_exchange_or_warn(
            conn_without_rag_tables(),
            None,
            "pregunta".to_string(),
            "respuesta".to_string(),
            Vec::new(),
            "modelo-x".to_string(),
        )
        .await;
        assert_eq!(conversation_id, None);
    }

    #[tokio::test]
    async fn persist_success_returns_the_real_conversation_id() {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            "CREATE TABLE rag_conversations (
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE rag_messages (
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES rag_conversations(id) ON DELETE CASCADE,
               sort_index INTEGER NOT NULL,
               role TEXT NOT NULL CHECK(role IN ('user','assistant')),
               content TEXT NOT NULL,
               sources TEXT,
               model TEXT,
               created_at INTEGER NOT NULL
             );",
        )
        .expect("RAG chat schema creation failed");

        let conversation_id = persist_exchange_or_warn(
            Arc::new(Mutex::new(conn)),
            None,
            "pregunta".to_string(),
            "respuesta".to_string(),
            Vec::new(),
            "modelo-x".to_string(),
        )
        .await;
        assert!(
            conversation_id.is_some(),
            "successful persistence keeps returning Some(id)"
        );
    }

    // ── Citation filtering ────────────────────────────────────────────────────

    fn test_source(index: u32) -> RagSource {
        RagSource {
            index,
            asset_id: format!("asset-{index}"),
            item_id: format!("item-{index}"),
            item_title: format!("Doc {index}"),
            collection_id: "col".to_string(),
            collection_name: "Col".to_string(),
            snippet: format!("snippet {index}"),
            score: 1.0,
            start_seconds: None,
            end_seconds: None,
        }
    }

    #[test]
    fn extract_citation_indices_finds_all_bracketed_numbers() {
        let text = "Según [1] y [3], el cabildo [2] sesionó. Ver también [1].";
        let indices = extract_citation_indices(text);
        assert_eq!(indices.len(), 3);
        assert!(indices.contains(&1));
        assert!(indices.contains(&2));
        assert!(indices.contains(&3));
    }

    #[test]
    fn extract_citation_indices_ignores_zero_and_non_citations() {
        let text = "El año [0] no es cita. Tampoco [abc] ni [] ni [12x].";
        let indices = extract_citation_indices(text);
        assert!(indices.is_empty());
    }

    #[test]
    fn extract_citation_indices_empty_text() {
        assert!(extract_citation_indices("").is_empty());
        assert!(extract_citation_indices("sin citas acá").is_empty());
    }

    #[test]
    fn filter_cited_sources_keeps_only_cited() {
        let sources = vec![test_source(1), test_source(2), test_source(3)];
        let answer = "Según [1] y [3], algo pasó.";
        let filtered = filter_cited_sources(sources, answer);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].index, 1);
        assert_eq!(filtered[1].index, 3);
    }

    #[test]
    fn filter_cited_sources_returns_none_when_no_citations_detected() {
        let sources = vec![test_source(1), test_source(2), test_source(3)];
        let answer = "No encontré información relevante.";
        let filtered = filter_cited_sources(sources, answer);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_cited_sources_ignores_invalid_and_out_of_range_numbers() {
        let sources = vec![test_source(1), test_source(2), test_source(3)];
        let answer = "Referencias inválidas: [0], [abc], [99].";
        let filtered = filter_cited_sources(sources, answer);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_cited_sources_handles_single_citation() {
        let sources = vec![test_source(1), test_source(2)];
        let answer = "Solo [2] menciona esto.";
        let filtered = filter_cited_sources(sources, answer);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].index, 2);
    }
}
