//! Where each feature runs, and which provider APIs are switched on, for
//! assistants: the `engines_setup` MCP tool (and through the help bridge, the
//! Ayuda agent). It reads and writes the same `app_settings` keys as
//! Configuración → Motores and the "Usar esta API" boxes, so the app and an
//! assistant can never disagree about what is set.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;

/// One choice an engine setting accepts.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EngineOption {
    pub value: String,
    pub label: String,
}

/// One "where does this run" setting.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EngineSetting {
    /// The setting key, also the name to use in `set`.
    pub setting: String,
    pub label: String,
    pub value: String,
    pub options: Vec<EngineOption>,
    /// What the setting decides.
    pub description: String,
}

/// One provider API and its "Usar esta API" switch.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiState {
    /// The id to use in `set`.
    pub id: String,
    pub label: String,
    /// A key is saved (it is never shown).
    pub key_stored: bool,
    /// Switched on. Off keeps the key but nothing is sent to the provider.
    pub enabled: bool,
}

/// What the tool answers, for `status` and `set` alike.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnginesReport {
    pub action: String,
    /// `false` in Lite: every feature uses the provider APIs, there is no
    /// engine to choose.
    pub local_ml: bool,
    pub engines: Vec<EngineSetting>,
    pub apis: Vec<ApiState>,
    /// The settings `set` changed, as `key=value`.
    pub changed: Vec<String>,
}

struct EngineSpec {
    setting: &'static str,
    label: &'static str,
    default: &'static str,
    options: &'static [(&'static str, &'static str)],
    description: &'static str,
}

const ENGINES: &[EngineSpec] = &[
    EngineSpec {
        setting: "llm_mode",
        label: "IA de texto (Modo LLM)",
        default: "local",
        options: &[
            ("local", "Local (Gemma, en este equipo)"),
            ("openrouter", "OpenRouter"),
            ("auto", "Automático (local primero, OpenRouter de respaldo)"),
        ],
        description: "Resúmenes, entidades, tripletas, esquemas y Chat.",
    },
    EngineSpec {
        setting: "embedding_provider",
        label: "Embeddings",
        default: "local",
        options: &[
            ("local", "Local ONNX (BGE-M3, en este equipo)"),
            ("api", "API de OpenRouter (BGE-M3)"),
        ],
        description: "Búsqueda semántica, páginas similares y recuperación del Chat.",
    },
    EngineSpec {
        setting: "stt_mode",
        label: "Transcripción (Modo STT)",
        default: "local",
        options: &[
            ("local", "Local"),
            ("assemblyai", "AssemblyAI"),
            ("auto", "Automático (local primero, AssemblyAI de respaldo)"),
        ],
        description: "Transcripción de audio y dictado.",
    },
    EngineSpec {
        setting: "stt_local_engine",
        label: "Motor de transcripción local",
        default: "parakeet",
        options: &[("parakeet", "Parakeet (rápido)"), ("whisper", "Whisper")],
        description: "Qué motor usa la transcripción cuando corre en el equipo.",
    },
    EngineSpec {
        setting: "ocrh_mode",
        label: "OCR de alta precisión (Modo OCRH)",
        default: "local",
        options: &[
            ("local", "Local"),
            ("glm_ocr", "GLM-OCR (z.ai)"),
            ("auto", "Automático (GLM-OCR primero, local de respaldo)"),
        ],
        description: "OCRH de páginas e imágenes.",
    },
    EngineSpec {
        setting: "ocrh_local_engine",
        label: "Motor OCRH local",
        default: "auto",
        options: &[
            (
                "auto",
                "Automático (GLM-OCR local con GPU NVIDIA; si no, PaddleOCR-VL)",
            ),
            ("glm_ocr", "GLM-OCR local (necesita GPU NVIDIA)"),
            ("paddle_vl", "PaddleOCR-VL"),
        ],
        description: "Qué motor usa el OCRH cuando corre en el equipo.",
    },
];

/// `(id, label, key setting)` for every provider with a "Usar esta API" box.
const APIS: &[(&str, &str, &str)] = &[
    (
        "openrouter",
        "OpenRouter",
        crate::settings::OPENROUTER_API_KEY,
    ),
    (
        "vercel",
        "Vercel AI Gateway",
        crate::settings::VERCEL_AI_GATEWAY_API_KEY,
    ),
    (
        "assemblyai",
        "AssemblyAI",
        crate::settings::ASSEMBLYAI_API_KEY,
    ),
    (
        "glm_ocr",
        "GLM-OCR (z.ai)",
        crate::settings::GLM_OCR_API_KEY,
    ),
    ("zotero", "Zotero Web API", crate::settings::ZOTERO_API_KEY),
];

fn local_ml() -> bool {
    cfg!(feature = "local-ml")
}

/// The current engines and APIs, without changing anything.
pub fn status(conn: &Connection) -> EnginesReport {
    report(conn, "status", Vec::new())
}

/// [`status`] for the archive at `db_path`.
pub fn status_at(db_path: &Path) -> Result<EnginesReport, String> {
    let conn = crate::db::open::open_archive_connection(db_path)?;
    Ok(status(&conn))
}

fn report(conn: &Connection, action: &str, changed: Vec<String>) -> EnginesReport {
    let engines = if local_ml() {
        ENGINES
            .iter()
            .map(|spec| EngineSetting {
                setting: spec.setting.to_string(),
                label: spec.label.to_string(),
                value: crate::settings::get_raw_setting(conn, spec.setting)
                    .filter(|value| spec.options.iter().any(|(option, _)| option == value))
                    .unwrap_or_else(|| spec.default.to_string()),
                options: spec
                    .options
                    .iter()
                    .map(|(value, label)| EngineOption {
                        value: value.to_string(),
                        label: label.to_string(),
                    })
                    .collect(),
                description: spec.description.to_string(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let apis = APIS
        .iter()
        .map(|(id, label, key)| ApiState {
            id: id.to_string(),
            label: label.to_string(),
            key_stored: crate::settings::get_raw_setting(conn, key)
                .is_some_and(|value| !value.trim().is_empty()),
            enabled: !crate::settings::api_key_disabled(conn, key),
        })
        .collect();
    EnginesReport {
        action: action.to_string(),
        local_ml: local_ml(),
        engines,
        apis,
        changed,
    }
}

/// The writes `set` would make, every value checked before the first one:
/// a bad name or value refuses the whole request with nothing changed.
pub fn plan_changes(
    engines: &BTreeMap<String, String>,
    apis: &BTreeMap<String, bool>,
) -> Result<Vec<(String, String)>, String> {
    if !engines.is_empty() && !local_ml() {
        return Err(
            "EntropIA Lite no tiene motores locales: cada función usa su API. Solo se pueden encender o apagar las APIs."
                .to_string(),
        );
    }
    let mut writes = Vec::new();
    for (setting, value) in engines {
        let spec = ENGINES
            .iter()
            .find(|spec| spec.setting == setting)
            .ok_or_else(|| {
                format!(
                    "«{setting}» no es un motor. Los válidos son: {}.",
                    ENGINES
                        .iter()
                        .map(|spec| spec.setting)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        if !spec.options.iter().any(|(option, _)| option == value) {
            return Err(format!(
                "«{value}» no es un valor de {setting}. Los válidos son: {}.",
                spec.options
                    .iter()
                    .map(|(option, _)| *option)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        writes.push((setting.clone(), value.clone()));
    }
    for (id, enabled) in apis {
        let (_, _, key) = APIS.iter().find(|(api, _, _)| api == id).ok_or_else(|| {
            format!(
                "«{id}» no es una API. Las válidas son: {}.",
                APIS.iter()
                    .map(|(api, _, _)| *api)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let flag = crate::settings::api_disabled_setting(key)
            .expect("every listed API key can be switched off");
        writes.push((flag, if *enabled { "0" } else { "1" }.to_string()));
    }
    if writes.is_empty() {
        return Err("No hay nada que cambiar: indicá `engines` o `apis`.".to_string());
    }
    Ok(writes)
}

/// Apply `set`: the same writes Configuración makes, then the same follow-up
/// (work parked on a setting resumes once it changes).
pub fn apply(
    db_path: &Path,
    engines: &BTreeMap<String, String>,
    apis: &BTreeMap<String, bool>,
) -> Result<EnginesReport, String> {
    let writes = plan_changes(engines, apis)?;
    let conn = crate::db::open::open_archive_connection(db_path)?;
    let mut changed = Vec::new();
    for (key, value) in &writes {
        crate::settings::set_setting(&conn, key, value)
            .map_err(|error| format!("No se pudo guardar {key}: {error}"))?;
        crate::settings::resume_work_after_setting_change(&conn, key);
        changed.push(format!("{key}={value}"));
    }
    Ok(report(&conn, "set", changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engines(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn apis(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
        pairs
            .iter()
            .map(|(key, on)| (key.to_string(), *on))
            .collect()
    }

    #[test]
    fn switching_an_api_writes_its_flag_and_never_the_key() {
        let writes = plan_changes(
            &BTreeMap::new(),
            &apis(&[("openrouter", false), ("zotero", true)]),
        )
        .expect("valid");
        assert_eq!(
            writes,
            vec![
                ("openrouter_api_key_disabled".to_string(), "1".to_string()),
                ("zotero_api_key_disabled".to_string(), "0".to_string()),
            ]
        );
    }

    #[test]
    fn a_bad_name_or_value_refuses_everything() {
        assert!(plan_changes(&BTreeMap::new(), &apis(&[("dropbox", true)]))
            .unwrap_err()
            .contains("no es una API"));
        assert!(plan_changes(&BTreeMap::new(), &BTreeMap::new())
            .unwrap_err()
            .contains("nada que cambiar"));
        if local_ml() {
            assert!(
                plan_changes(&engines(&[("llm_mode", "cloud")]), &BTreeMap::new())
                    .unwrap_err()
                    .contains("local, openrouter, auto")
            );
            assert!(
                plan_changes(&engines(&[("color", "rojo")]), &BTreeMap::new())
                    .unwrap_err()
                    .contains("no es un motor")
            );
            assert_eq!(
                plan_changes(&engines(&[("ocrh_mode", "local")]), &BTreeMap::new()).unwrap(),
                vec![("ocrh_mode".to_string(), "local".to_string())]
            );
        } else {
            assert!(
                plan_changes(&engines(&[("llm_mode", "local")]), &BTreeMap::new())
                    .unwrap_err()
                    .contains("Lite")
            );
        }
    }

    #[test]
    fn status_reads_what_configuracion_wrote() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO app_settings VALUES ('stt_mode', 'assemblyai');
             INSERT INTO app_settings VALUES ('glm_ocr_api_key', 'secret_ref:glm_ocr_api_key');
             INSERT INTO app_settings VALUES ('glm_ocr_api_key_disabled', '1');",
        )
        .unwrap();
        let report = status(&conn);
        let glm = report.apis.iter().find(|api| api.id == "glm_ocr").unwrap();
        assert!(glm.key_stored && !glm.enabled);
        let openrouter = report
            .apis
            .iter()
            .find(|api| api.id == "openrouter")
            .unwrap();
        assert!(!openrouter.key_stored && openrouter.enabled);
        if local_ml() {
            let stt = report
                .engines
                .iter()
                .find(|e| e.setting == "stt_mode")
                .unwrap();
            assert_eq!(stt.value, "assemblyai");
            let llm = report
                .engines
                .iter()
                .find(|e| e.setting == "llm_mode")
                .unwrap();
            assert_eq!(llm.value, "local", "an unset mode reads as its default");
        } else {
            assert!(report.engines.is_empty());
        }
    }
}
