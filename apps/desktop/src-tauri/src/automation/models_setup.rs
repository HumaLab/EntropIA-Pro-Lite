//! Local-model setup for external assistants: one place to see which local
//! models are installed and to queue the missing downloads.
//!
//! The Settings UI drives the same managers through the Tauri commands; this
//! module exposes the shared pieces so the assistant `models_setup` MCP tool
//! offers them over the connector (Claude Code, Codex, OpenCode, Pi, …).
//! Installs are fire-and-forget background tasks with progress events, exactly
//! like the UI buttons: `start_missing` returns what it queued, and the agent
//! polls `status` to follow progress.

use std::path::{Path, PathBuf};

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
    }
}

/// Inspect every component without starting anything.
pub fn status(db_path: &Path, app_handle: Option<&tauri::AppHandle>) -> ModelsStatus {
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = (db_path, app_handle);
        return ModelsStatus {
            local_ml: false,
            components: Vec::new(),
        };
    }
    #[cfg(feature = "local-ml")]
    {
        status_local_ml(db_path, app_handle)
    }
}

#[cfg(feature = "local-ml")]
fn status_local_ml(db_path: &Path, app_handle: Option<&tauri::AppHandle>) -> ModelsStatus {
    let mut components = Vec::new();

    // Runtime pack (Python, venv, Paddle deps, scripts): needed for every
    // local OCR path. Needs the app handle for bundle/data-dir resolution.
    match app_handle {
        Some(handle) => match crate::runtime::RuntimeManager::new().status(handle) {
            Ok(st) => {
                let ready = st.state == crate::runtime::status::RuntimeState::Healthy;
                let state = format!("{:?}", st.state);
                components.push(component(
                    COMPONENT_RUNTIME,
                    "Runtime de IA local",
                    ready,
                    format!("estado: {state}"),
                    "OCR local (PaddleOCR, GLM-OCR, layout), transcripción y dependencias",
                ));
            }
            Err(error) => components.push(component(
                COMPONENT_RUNTIME,
                "Runtime de IA local",
                false,
                format!("no se pudo inspeccionar: {error}"),
                "OCR local (PaddleOCR, GLM-OCR, layout), transcripción y dependencias",
            )),
        },
        None => components.push(component(
            COMPONENT_RUNTIME,
            "Runtime de IA local",
            false,
            "sin contexto de app (tests): no inspeccionable aquí",
            "OCR local (PaddleOCR, GLM-OCR, layout), transcripción y dependencias",
        )),
    }

    // Text LLM (Gemma): chat, resúmenes, entidades, tripletas.
    {
        let info = crate::llm::get_local_model_info(db_path);
        components.push(component(
            COMPONENT_LLM,
            "Modelo de texto local (Gemma)",
            info.exists,
            if info.exists {
                format!("{} ({} bytes)", info.filename, info.size_bytes.unwrap_or(0))
            } else {
                format!("falta {}; fuente: {}", info.filename, info.source_url)
            },
            "Chat, resúmenes, entidades y tripletas sin nube",
        ));
    }

    // GLM-OCR (visión + servidor llama-server): motor OCRH preferido.
    {
        let info = crate::ocr::glm_ocr_local::local_info(db_path);
        let ready = info.model_present && info.mmproj_present && info.server_present;
        let mut missing = Vec::new();
        if !info.model_present {
            missing.push("modelo");
        }
        if !info.mmproj_present {
            missing.push("mmproj");
        }
        if !info.server_present {
            missing.push("servidor llama-server");
        }
        components.push(component(
            COMPONENT_GLM_OCR,
            "GLM-OCR local",
            ready,
            if ready {
                format!("modelos en {}", info.models_dir)
            } else {
                format!("falta: {}", missing.join(", "))
            },
            "OCRH de alta calidad en GPU/CPU sin nube",
        ));
    }

    // Qwen3-VL corrección OCR.
    {
        let info = crate::ocr::qwen_local::local_info(db_path);
        let ready = info.model_present && info.mmproj_present && info.server_present;
        components.push(component(
            COMPONENT_QWEN_CORRECT,
            "Corrección OCR local (Qwen3-VL)",
            ready,
            if ready {
                format!("modelos en {}", info.models_dir)
            } else {
                "incompleto (modelo, mmproj o servidor)".to_string()
            },
            "Corrección OCRC de OCR con visión",
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
                format!(
                    "{} ({} archivos)",
                    info.directory,
                    info.required_files.len()
                )
            } else {
                format!("faltan {} archivos", info.missing_files.len())
            },
            "Búsqueda semántica, RAG y chat con contexto",
        ));
    }

    // Reranker local: ordena candidatos del RAG.
    {
        let dir = crate::rag::reranker::resolve_local_reranker_model_dir(db_path.parent());
        let info = crate::rag::reranker::get_local_reranker_model_info(dir);
        components.push(component(
            COMPONENT_RERANKER,
            "Reranker local",
            info.available,
            if info.available {
                format!("{} válido", info.directory)
            } else {
                format!("incompleto en {}", info.directory)
            },
            "Mejor ranking de pasajes en chat e investigación",
        ));
    }

    ModelsStatus {
        local_ml: true,
        components,
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
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = (db_path, app_handle, only);
        return Err(
            "Este build es solo OpenRouter (Lite): no hay modelos locales que instalar."
                .to_string(),
        );
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
) -> Result<(Vec<String>, ModelsStatus), String> {
    let status = status(db_path, Some(app_handle));
    let wanted = |id: &str| only.is_empty() || only.iter().any(|w| w == id);
    let mut started = Vec::new();
    for component_state in &status.components {
        if component_state.ready || !wanted(&component_state.id) {
            continue;
        }
        let outcome = match component_state.id.as_str() {
            COMPONENT_RUNTIME => crate::runtime::runtime_bootstrap_start(app_handle.clone())
                .await
                .map(|_| "runtime: descarga iniciada".to_string()),
            COMPONENT_LLM => {
                crate::llm::commands::start_local_model_download(db_path, app_handle.clone())
            }
            COMPONENT_GLM_OCR => {
                crate::ocr::commands::start_glm_ocr_download(db_path, app_handle.clone())
            }
            COMPONENT_QWEN_CORRECT => {
                crate::ocr::commands::start_qwen_correction_download(db_path, app_handle.clone())
            }
            COMPONENT_EMBEDDINGS => {
                crate::nlp::commands::start_embedding_download(db_path, app_handle.clone())
            }
            COMPONENT_RERANKER => {
                crate::rag::commands::start_reranker_download(db_path, app_handle.clone())
            }
            other => Err(format!("componente desconocido: {other}")),
        };
        match outcome {
            Ok(message) => started.push(format!("{}: {message}", component_state.id)),
            Err(error) => started.push(format!(
                "{}: no se pudo iniciar ({error})",
                component_state.id
            )),
        }
    }
    report_unknown_components(only, &status.components, &mut started);
    Ok((started, status))
}

/// Flag filter ids that match no known component, so a typo does not look
/// like "nothing to do".
pub(crate) fn report_unknown_components(
    only: &[String],
    components: &[ComponentStatus],
    started: &mut Vec<String>,
) {
    for unknown in only {
        if !components.iter().any(|c| &c.id == unknown) {
            started.push(format!(
                "{unknown}: componente desconocido (ids válidos: runtime, llm, glm_ocr, qwen_correct, embeddings, reranker)"
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
