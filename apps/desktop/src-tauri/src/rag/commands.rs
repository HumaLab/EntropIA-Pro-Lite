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

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::params::{rag_params_from_settings, RagParams, TOP_K_MAX, TOP_K_MIN};
use super::{retrieval, store};
use super::{RagAnswer, RagChatTurn, RagConversation, RagConversationSummary, RagSource};

const QUESTION_MAX_CHARS: usize = 4000;

/// Modo de generación de la respuesta del chat RAG. El camino por defecto es
/// `Local` (Gemma en el equipo); `OpenRouter` solo se selecciona cuando
/// `llm_mode` lo pide explícitamente Y hay credenciales (mismo idioma que
/// `ner_fallback_config`).
enum RagAnswerMode {
    #[cfg(feature = "local-ml")]
    Local,
    OpenRouter { api_key: String, model: String },
}

/// Resultado de la fase bloqueante (settings + historial + recuperación).
struct RetrievalPhase {
    mode: RagAnswerMode,
    model: String,
    sources: Vec<RagSource>,
    history: Vec<RagChatTurn>,
    params: RagParams,
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
    let phase = tokio::task::spawn_blocking(move || -> Result<RetrievalPhase, String> {
        // Paso 1: lecturas de settings + historial persistido con el lock,
        // soltándolo antes de cualquier I/O pesado (embedding/inferencia).
        let (mode, model, history, params) = {
            let conn = conn_arc.lock().map_err(|e| e.to_string())?;

            // Modo de respuesta: local por defecto (mismo idioma que
            // `ner_fallback_config`). OpenRouter solo si se seleccionó Y hay
            // clave; si falta la clave, degradamos a local en vez de fallar.
            let (mode, model) = resolve_answer_mode(&conn);

            // Parámetros RAG runtime (rag_top_k, rag_min_similarity, etc.);
            // el argumento `top_k` del comando pisa al setting si vino.
            let mut params = rag_params_from_settings(&conn);
            params.top_k = resolve_top_k(requested_top_k, params.top_k);

            // Historial desde la conversación persistida (vacío si el id no
            // existe o no vino); presupuesto de turnos/chars configurable.
            let history = match history_conversation_id.as_deref() {
                Some(id) => store::load_history(&conn, id, params.history_turns)?,
                None => Vec::new(),
            };

            (mode, model, history, params)
        };

        // Paso 2 (sin lock): pierna vectorial LOCAL con degradación elegante;
        // si la config o el embedding fallan (modelo ONNX ausente, etc.),
        // seguimos solo con FTS.
        let query_embedding = embed_query_local(&embed_db_path, &retrieval_question);

        // Paso 3: re-adquirir el lock solo para la recuperación SQL.
        let conn = conn_arc.lock().map_err(|e| e.to_string())?;
        let sources = retrieval::hybrid_retrieve(
            &conn,
            &retrieval_question,
            query_embedding.as_deref(),
            &params,
        )?;

        Ok(RetrievalPhase {
            mode,
            model,
            sources,
            history,
            params,
        })
    })
    .await
    .map_err(|e| format!("RAG retrieval task panicked: {e}"))??;

    // Sin contenido relevante: no llamamos al LLM; el frontend muestra su
    // propio mensaje de "sin resultados". El intercambio vacío también se
    // persiste para que la conversación quede completa.
    if phase.sources.is_empty() {
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

    let answer = generate_answer(
        &app_handle,
        &db_path,
        &phase.mode,
        &question,
        &phase.sources,
        &phase.history,
        &phase.params,
    )
    .await?;

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
        phase.sources.clone(),
        phase.model.clone(),
    )
    .await;

    Ok(RagAnswer {
        answer,
        sources: phase.sources,
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
            client.generate(&prompt, params.max_tokens).await
        }
        #[cfg(feature = "local-ml")]
        RagAnswerMode::Local => {
            // Camino por defecto: motor Gemma local. Mismo patrón que
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
                let max_tokens = params.max_tokens;
                let engine = engine
                    .lock()
                    .map_err(|error| format!("Local Gemma engine lock poisoned: {error}"))?;

                // Presupuesto de contexto contra el `n_ctx` del modelo local:
                // los fragmentos recuperados pueden ser grandes y desbordar a
                // Gemma. Primero acotamos el bloque de fragmentos con chunking
                // (los más relevantes van primero), luego construimos el prompt
                // completo y, como red de seguridad final, lo truncamos al
                // presupuesto real del modelo.
                let prompt = build_local_rag_prompt(
                    engine.n_ctx(),
                    max_tokens,
                    &question,
                    &sources,
                    &history,
                    &params,
                );
                engine.generate(&prompt, max_tokens, "[rag][local]")
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
        let engine = crate::nlp::embeddings::EmbeddingEngine::init(config)?;
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
    let model = crate::settings::get_setting(conn, "openrouter_model")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_RAG_OPENROUTER_MODEL.to_string());
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
            let model = crate::settings::get_setting(conn, "openrouter_model")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_RAG_OPENROUTER_MODEL.to_string());
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

/// Modelo OpenRouter por defecto para el branch remoto opcional del chat RAG.
const DEFAULT_RAG_OPENROUTER_MODEL: &str = "google/gemma-3-4b-it";

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
/// El contexto de fragmentos puede ser grande y desbordar la ventana del
/// modelo local. Acotamos el bloque de fragmentos con [`chunk_text`] (que
/// devuelve la entrada intacta si es chica y, si no, su primera ventana — los
/// fragmentos más relevantes van primero), construimos el prompt crudo y, como
/// red de seguridad final, truncamos el prompt entero al presupuesto real de
/// tokens del modelo. El resultado se envuelve en el formato de turnos Gemma.
#[cfg(feature = "local-ml")]
fn build_local_rag_prompt(
    n_ctx: u32,
    max_tokens: i32,
    question: &str,
    sources: &[RagSource],
    history: &[RagChatTurn],
    params: &RagParams,
) -> String {
    let context = budget_context_for_local(&format_fragments(sources));
    let history_block =
        format_history(history, params.history_turns, params.history_turn_max_chars);
    let raw = crate::llm::prompt::raw_rag_answer(question, &context, &history_block);
    let truncated = crate::llm::truncate_text_for_context(n_ctx, max_tokens, &raw);
    crate::llm::prompt::gemma_wrap(&truncated)
}

/// Acota el bloque de fragmentos al primer chunk de [`chunk_text`]. Para
/// contextos chicos (la mayoría) es un passthrough sin costo; para contextos
/// enormes evita arrastrar megabytes de texto hacia el truncado final.
#[cfg(feature = "local-ml")]
fn budget_context_for_local(context: &str) -> String {
    crate::nlp::chunking::chunk_text(context)
        .into_iter()
        .next()
        .map(|chunk| chunk.text)
        .unwrap_or_default()
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
        assert_eq!(model, DEFAULT_RAG_OPENROUTER_MODEL);
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
        // Envuelto en el formato de turnos de Gemma para el motor local.
        assert!(prompt.contains("<start_of_turn>user"));
        assert!(prompt.contains("<start_of_turn>model"));
    }

    #[test]
    fn build_local_rag_prompt_without_history_omits_history_block() {
        let sources = vec![source(1, "Acta", "Archivo", "fragmento")];
        let prompt =
            build_local_rag_prompt(4096, 1500, "pregunta", &sources, &[], &RagParams::default());
        assert!(!prompt.contains("Conversación previa"));
        assert!(prompt.contains("Pregunta: pregunta"));
    }

    #[test]
    fn budget_context_for_local_passthrough_for_short_context() {
        let context = "[1] «Acta» (Archivo):\nun fragmento corto";
        assert_eq!(budget_context_for_local(context), context);
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
}
