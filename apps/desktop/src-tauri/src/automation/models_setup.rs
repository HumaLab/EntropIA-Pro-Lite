//! Local-model setup for external assistants: one place to see which local
//! models are installed and to queue the missing downloads.
//!
//! The Settings UI drives the same managers through the Tauri commands; this
//! module exposes the shared pieces so the assistant `models_setup` MCP tool
//! offers them over the connector (Claude Code, Codex, OpenCode, Pi, …).
//! Installs are fire-and-forget background tasks with progress events, exactly
//! like the UI buttons: `start_missing` returns what it queued, and the agent
//! polls `status` to follow progress.

use std::path::Path;
#[cfg(feature = "local-ml")]
use std::path::PathBuf;

/// One installable local-model component.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStatus {
    /// Stable id, also used as the `components` filter in the tool call.
    pub id: String,
    /// Short human label.
    pub label: String,
    /// Whether the component is fully installed and usable.
    pub ready: bool,
    /// One-line detail: what is missing, or versions/sizes when ready.
    pub detail: String,
    /// What this component enables in the app.
    pub enables: String,
    /// Set when this machine cannot use it (an NVIDIA-only model on a
    /// computer without one): why, in plain words. Install skips it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

/// Whole-machine snapshot for the tool response.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelsStatus {
    /// `false` in Lite builds (OpenRouter-only, nothing to install).
    pub local_ml: bool,
    pub components: Vec<ComponentStatus>,
}

/// What the `models_setup` tool answers, for `status` and `install` alike:
/// the inventory plus what this call started (empty for `status`).
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelsSetupReport {
    /// The action that ran: `status` or `install`.
    pub action: String,
    /// `false` in Lite builds (OpenRouter-only, nothing to install).
    pub local_ml: bool,
    pub components: Vec<ComponentStatus>,
    /// What `install` queued this call (component ids with their outcome).
    pub started: Vec<String>,
}

impl ModelsSetupReport {
    /// Inventory answer for `action: status`.
    pub fn status(db_path: &Path, app_handle: Option<&tauri::AppHandle>) -> Self {
        let snapshot = status(db_path, app_handle);
        Self {
            action: "status".to_string(),
            local_ml: snapshot.local_ml,
            components: snapshot.components,
            started: Vec::new(),
        }
    }

    /// Answer for `action: install`: fresh inventory plus what was queued.
    /// The agent polls `status` to follow progress.
    /// Answer for `action: install`: the pre-install inventory plus what was
    /// queued. The agent polls `status` to follow progress (note: a full
    /// inventory revalidates installed files, ~10-15 s on a hydrated machine;
    /// poll at most every minute or two while a download runs).
    pub fn install_started(snapshot: ModelsStatus, started: Vec<String>) -> Self {
        Self {
            action: "install".to_string(),
            local_ml: snapshot.local_ml,
            components: snapshot.components,
            started,
        }
    }
}

/// Component ids for the audit trail: the explicit filter when the agent gave
/// one, else the ids `start_missing` reported on.
pub fn only_or_started_ids(only: &[String], started: &[String]) -> Vec<String> {
    if only.is_empty() {
        started
            .iter()
            .filter_map(|line| line.split(':').next().map(str::to_string))
            .collect()
    } else {
        only.to_vec()
    }
}

/// Stable component ids, in install-relevance order.
pub const COMPONENT_RUNTIME: &str = "runtime";
pub const COMPONENT_LLM: &str = "llm";
pub const COMPONENT_GLM_OCR: &str = "glm_ocr";
pub const COMPONENT_QWEN_CORRECT: &str = "qwen_correct";
pub const COMPONENT_EMBEDDINGS: &str = "embeddings";
pub const COMPONENT_RERANKER: &str = "reranker";
pub const COMPONENT_PARAKEET: &str = "parakeet";
pub const COMPONENT_LLAMA_SERVER: &str = "llama_server";

/// Every component id, in inventory order (for the unknown-filter message).
pub const COMPONENT_IDS: &[&str] = &[
    COMPONENT_RUNTIME,
    COMPONENT_LLM,
    COMPONENT_EMBEDDINGS,
    COMPONENT_RERANKER,
    COMPONENT_PARAKEET,
    COMPONENT_LLAMA_SERVER,
    COMPONENT_GLM_OCR,
    COMPONENT_QWEN_CORRECT,
];

// Reporting-only helpers: live only in local-ml builds (status/download),
// but unit tests exercise them in every build.
#[cfg_attr(not(feature = "local-ml"), allow(dead_code))]
fn component(
    id: &str,
    label: &str,
    ready: bool,
    detail: impl Into<String>,
    enables: &str,
) -> ComponentStatus {
    ComponentStatus {
        id: id.to_string(),
        label: label.to_string(),
        ready,
        detail: detail.into(),
        enables: enables.to_string(),
        unavailable_reason: None,
    }
}

/// Approximate size for a detail line ("1,4 GB"), never exact bytes.
#[cfg_attr(not(feature = "local-ml"), allow(dead_code))]
fn human_size(bytes: u64) -> String {
    const GB: f64 = 1_000_000_000.0;
    const MB: f64 = 1_000_000.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB).replace('.', ",")
    } else {
        format!("{:.0} MB", (value / MB).max(1.0))
    }
}

/// Inspect every component without starting anything.
pub fn status(db_path: &Path, app_handle: Option<&tauri::AppHandle>) -> ModelsStatus {
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = (db_path, app_handle);
        ModelsStatus {
            local_ml: false,
            components: Vec::new(),
        }
    }
    #[cfg(feature = "local-ml")]
    {
        status_local_ml(db_path, app_handle)
    }
}

#[cfg(feature = "local-ml")]
fn status_local_ml(db_path: &Path, app_handle: Option<&tauri::AppHandle>) -> ModelsStatus {
    use crate::ocr::local_server::ServerSource;
    use tauri::Manager;
    let mut components = Vec::new();
    let resource_dir = app_handle.and_then(|handle| handle.path().resource_dir().ok());
    let has_gpu = crate::llm::has_nvidia_gpu();
    const NEEDS_GPU: &str = crate::ocr::local_server::NEEDS_NVIDIA_GPU;

    // Runtime pack (Python, venv, Paddle deps, scripts): needed for every
    // local OCR path. Needs the app handle for bundle/data-dir resolution.
    const RUNTIME_ENABLES: &str =
        "OCR local (PaddleOCR, PaddleOCR-VL, layout), transcripción con Whisper y entidades con spaCy";
    components.push(match app_handle {
        Some(handle) => match crate::runtime::RuntimeManager::new().status(handle) {
            Ok(st) => component(
                COMPONENT_RUNTIME,
                "Base de IA local (runtime)",
                st.state == crate::runtime::status::RuntimeState::Healthy,
                st.summary,
                RUNTIME_ENABLES,
            ),
            Err(error) => component(
                COMPONENT_RUNTIME,
                "Base de IA local (runtime)",
                false,
                format!("No se pudo revisar: {error}"),
                RUNTIME_ENABLES,
            ),
        },
        None => component(
            COMPONENT_RUNTIME,
            "Base de IA local (runtime)",
            false,
            "Sin la app abierta no se puede revisar.",
            RUNTIME_ENABLES,
        ),
    });

    // Text LLM (Gemma): chat, resúmenes, entidades, tripletas.
    {
        let info = crate::llm::get_local_model_info(db_path);
        components.push(component(
            COMPONENT_LLM,
            "Modelo de texto local (Gemma)",
            info.exists,
            if info.exists {
                format!(
                    "Instalado: {} ({}).",
                    info.filename,
                    human_size(info.size_bytes.unwrap_or(0))
                )
            } else {
                format!("Falta descargar {}.", info.filename)
            },
            "Chat, resúmenes, entidades y tripletas sin enviar el texto afuera",
        ));
    }

    // Embeddings locales (BGE-M3): búsqueda y RAG sin nube.
    {
        let dir = crate::nlp::commands::resolve_embedding_model_dir_for(db_path)
            .unwrap_or_else(|_| PathBuf::from("<?"));
        let info = crate::nlp::embeddings::get_local_embedding_model_info(Some(dir));
        components.push(component(
            COMPONENT_EMBEDDINGS,
            "Embeddings locales (BGE-M3)",
            info.available,
            if info.available {
                format!("Instalado en {}.", info.directory)
            } else {
                let missing: u64 = info
                    .missing_files
                    .iter()
                    .map(|file| file.size_bytes.unwrap_or(0))
                    .sum();
                format!(
                    "Faltan {} archivo(s), unos {}.",
                    info.missing_files.len(),
                    human_size(missing)
                )
            },
            "Búsqueda semántica y Chat sin enviar el texto afuera",
        ));
    }

    // Reranker local: ordena candidatos del RAG.
    {
        let dir = crate::rag::reranker::resolve_local_reranker_model_dir(
            &crate::path_utils::model_root(db_path),
        );
        let info = crate::rag::reranker::get_local_reranker_model_info(dir);
        components.push(component(
            COMPONENT_RERANKER,
            "Reranker local (BGE v2 M3)",
            info.available,
            if info.available {
                format!("Instalado en {}.", info.directory)
            } else {
                "Falta descargar (unos 560 MB).".to_string()
            },
            "Ordena los pasajes del Chat en el equipo",
        ));
    }

    // Parakeet: dictado y transcripción local.
    {
        let info = crate::transcription::commands::parakeet_info(db_path);
        components.push(component(
            COMPONENT_PARAKEET,
            "Transcripción local (Parakeet)",
            info.installed,
            if info.installed {
                format!("Instalado en {}.", info.directory)
            } else if info.downloading {
                "Descargando.".to_string()
            } else {
                format!("Falta descargar unos {}.", human_size(info.missing_bytes))
            },
            "Dictado y transcripción de audio sin enviar el audio afuera",
        ));
    }

    // llama-server: the program both local vision models run on.
    {
        let server = crate::ocr::local_server::resolve_server(
            db_path,
            crate::ocr::local_server::env_server_override(),
            resource_dir.as_deref(),
        );
        let present = server.is_file();
        let source = crate::ocr::local_server::server_source();
        let mut entry = component(
            COMPONENT_LLAMA_SERVER,
            "Servidor de modelos de visión (llama-server)",
            present,
            if present {
                format!("Instalado: {}.", server.display())
            } else if source == ServerSource::Bundled {
                "Falta: este sistema lo trae con el instalador; reinstalá EntropIA Pro.".to_string()
            } else {
                "Falta descargar (unos 610 MB; ocupa 1,1 GB).".to_string()
            },
            "Ejecuta GLM-OCR y Qwen3-VL en la GPU",
        );
        if !present && source == ServerSource::RequiresNvidiaGpu {
            entry.unavailable_reason = Some(NEEDS_GPU.to_string());
        }
        components.push(entry);
    }

    // GLM-OCR: OCRH local de alta precisión.
    {
        let info = crate::ocr::glm_ocr_local::local_info(db_path, resource_dir.as_deref());
        let ready = info.model_present && info.mmproj_present;
        let mut entry = component(
            COMPONENT_GLM_OCR,
            "OCRH local (GLM-OCR)",
            ready,
            if ready {
                format!("Instalado en {}.", info.models_dir)
            } else {
                "Faltan los modelos (unos 1,4 GB).".to_string()
            },
            "OCRH de alta precisión sin enviar las páginas afuera",
        );
        if !ready && !has_gpu {
            entry.unavailable_reason = Some(NEEDS_GPU.to_string());
        }
        components.push(entry);
    }

    // Qwen3-VL: corrección de OCR local.
    {
        let info = crate::ocr::qwen_local::local_info(db_path, resource_dir.as_deref());
        let ready = info.model_present && info.mmproj_present;
        let mut entry = component(
            COMPONENT_QWEN_CORRECT,
            "Corrector de OCR local (Qwen3-VL)",
            ready,
            if ready {
                format!("Instalado en {}.", info.models_dir)
            } else {
                "Faltan los modelos (unos 2,9 GB).".to_string()
            },
            "Corrección de OCR (OCRC) sin enviar el texto afuera",
        );
        if !ready && !has_gpu {
            entry.unavailable_reason = Some(NEEDS_GPU.to_string());
        }
        components.push(entry);
    }

    ModelsStatus {
        local_ml: true,
        components,
    }
}

/// What one install request did for one component.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub id: String,
    /// A download started, or one was already running or complete.
    pub started: bool,
    /// In plain words: what started, or why it did not.
    pub message: String,
}

impl InstallOutcome {
    fn line(&self) -> String {
        format!("{}: {}", self.id, self.message)
    }
}

/// The starters answer short codes; say them as the person reads them.
#[cfg_attr(not(feature = "local-ml"), allow(dead_code))]
fn started_message(code: &str) -> String {
    match code {
        "started" => "descarga iniciada".to_string(),
        "already complete" => "ya estaba instalado".to_string(),
        "already running" | "in_progress" => "ya se estaba descargando".to_string(),
        other => other.to_string(),
    }
}

/// Queue the missing installs in the background and return what was started.
/// Mirrors the Settings buttons; progress flows through the same events the UI
/// listens to. Requires the running app (the downloads emit progress).
pub async fn start_missing(
    db_path: &Path,
    app_handle: &tauri::AppHandle,
    only: &[String],
) -> Result<(Vec<String>, ModelsStatus), String> {
    let (outcomes, status) = start_missing_outcomes(db_path, app_handle, only).await?;
    let mut lines: Vec<String> = outcomes.iter().map(InstallOutcome::line).collect();
    report_unknown_components(only, &status.components, &mut lines);
    Ok((lines, status))
}

/// [`start_missing`], one structured outcome per component it looked at.
pub async fn start_missing_outcomes(
    db_path: &Path,
    app_handle: &tauri::AppHandle,
    only: &[String],
) -> Result<(Vec<InstallOutcome>, ModelsStatus), String> {
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = (db_path, app_handle, only);
        Err(
            "Este build es solo OpenRouter (Lite): no hay modelos locales que instalar."
                .to_string(),
        )
    }
    #[cfg(feature = "local-ml")]
    {
        start_missing_local_ml(db_path, app_handle, only).await
    }
}

#[cfg(feature = "local-ml")]
async fn start_missing_local_ml(
    db_path: &Path,
    app_handle: &tauri::AppHandle,
    only: &[String],
) -> Result<(Vec<InstallOutcome>, ModelsStatus), String> {
    let db_path_owned = db_path.to_path_buf();
    let handle = app_handle.clone();
    // The inventory revalidates files and probes the GPU: off the async thread.
    let status = tokio::task::spawn_blocking(move || status(&db_path_owned, Some(&handle)))
        .await
        .map_err(|e| format!("No se pudo revisar la instalación local: {e}"))?;
    let wanted = |id: &str| only.is_empty() || only.iter().any(|w| w == id);
    let mut outcomes = Vec::new();
    for component_state in &status.components {
        if component_state.ready || !wanted(&component_state.id) {
            continue;
        }
        // Not for this machine: say why instead of downloading gigabytes
        // nothing can run.
        if let Some(reason) = &component_state.unavailable_reason {
            outcomes.push(InstallOutcome {
                id: component_state.id.clone(),
                started: false,
                message: format!("no se instala: {reason}"),
            });
            continue;
        }
        let outcome = match component_state.id.as_str() {
            COMPONENT_RUNTIME => crate::runtime::runtime_bootstrap_start(app_handle.clone())
                .await
                .map(|_| "started".to_string()),
            COMPONENT_LLM => {
                crate::llm::commands::start_local_model_download(db_path, app_handle.clone())
            }
            COMPONENT_GLM_OCR => {
                crate::ocr::commands::start_glm_ocr_download(db_path, app_handle.clone())
            }
            COMPONENT_QWEN_CORRECT => {
                crate::ocr::commands::start_qwen_correction_download(db_path, app_handle.clone())
            }
            COMPONENT_LLAMA_SERVER => {
                crate::ocr::llama_server_download::start_download(db_path, app_handle.clone())
            }
            COMPONENT_PARAKEET => {
                crate::transcription::commands::start_parakeet_download(db_path, app_handle.clone())
            }
            COMPONENT_EMBEDDINGS => {
                crate::nlp::commands::start_embedding_download(db_path, app_handle.clone())
            }
            COMPONENT_RERANKER => {
                crate::rag::commands::start_reranker_download(db_path, app_handle.clone())
            }
            other => Err(format!("componente desconocido: {other}")),
        };
        outcomes.push(match outcome {
            Ok(code) => InstallOutcome {
                id: component_state.id.clone(),
                started: true,
                message: started_message(&code),
            },
            Err(error) => InstallOutcome {
                id: component_state.id.clone(),
                started: false,
                message: format!("no se pudo iniciar: {error}"),
            },
        });
    }
    Ok((outcomes, status))
}

/// Configuración → Dependencias de IA reads the same inventory the assistant
/// `models_setup` tool answers, so both say the same thing. Blocking work:
/// it revalidates the installed files.
#[tauri::command]
pub async fn local_setup_status(
    db: tauri::State<'_, crate::db::state::AppDbState>,
    app_handle: tauri::AppHandle,
) -> Result<ModelsStatus, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || status(&db_path, Some(&app_handle)))
        .await
        .map_err(|e| format!("No se pudo revisar la instalación local: {e}"))
}

/// Preparar EntropIA Pro: start every missing install this machine can use
/// (or only `only`). Answers, per component, what started or why it did not.
#[tauri::command]
pub async fn local_setup_install(
    only: Vec<String>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<InstallOutcome>, String> {
    start_missing_outcomes(&db.db_path, &app_handle, &only)
        .await
        .map(|(outcomes, _)| outcomes)
}

/// Flag filter ids that match no known component, so a typo does not look
/// like "nothing to do".
#[cfg_attr(not(feature = "local-ml"), allow(dead_code))]
pub(crate) fn report_unknown_components(
    only: &[String],
    components: &[ComponentStatus],
    started: &mut Vec<String>,
) {
    for unknown in only {
        if !components.iter().any(|c| &c.id == unknown) {
            started.push(format!(
                "{unknown}: componente desconocido (ids válidos: {})",
                COMPONENT_IDS.join(", ")
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_filter_ids_are_reported_not_ignored() {
        let components = vec![component("runtime", "R", true, "ok", "x")];
        let mut started = Vec::new();
        report_unknown_components(&["inexistente".to_string()], &components, &mut started);
        assert_eq!(started.len(), 1);
        assert!(started[0].contains("componente desconocido"));
        let mut known_only = Vec::new();
        report_unknown_components(&["runtime".to_string()], &components, &mut known_only);
        assert!(known_only.is_empty());
    }
}
