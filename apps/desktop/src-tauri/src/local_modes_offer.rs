//! One-time offer, in Pro, to switch the cloud modes an archive inherited
//! from Lite to the local engines.
//!
//! The modes live in `app_settings`, per archive and never synced. Pro's
//! local defaults apply only while a key is absent, so an archive Lite already
//! wrote keeps its cloud modes in Pro, and the local engines whose cards only
//! show in local mode (Parakeet, local GLM-OCR) stay hidden (R6).
//!
//! The offer is computed once per archive: [`OFFERED_KEY`] records that it was
//! answered, or that there was nothing to offer, so a cloud mode the user picks
//! later in Pro is never second-guessed. It only proposes what runs well on
//! this machine: dictation always (Parakeet runs on CPU), OCRH only with a GPU
//! that fits local GLM-OCR, the text LLM only with one that fits the local
//! Qwen corrector, and never the embedding provider (switching it re-embeds
//! the whole archive for identical vectors, R15).

use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use tauri::State;

/// Set once the offer was answered or found empty.
pub(crate) const OFFERED_KEY: &str = "pro_local_modes_offered";

/// One mode the offer would switch.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ModeOffer {
    pub key: &'static str,
    pub current: String,
    pub local: &'static str,
}

/// The GPU a local mode needs to run well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Needs {
    Nothing,
    /// Local GLM-OCR (4 GB on AMD/Intel/Apple).
    GlmGpu,
    /// The local Qwen corrector (6 GB on AMD/Intel/Apple).
    QwenGpu,
}

/// What this machine's GPU runs.
#[derive(Clone, Copy, Debug, Default)]
struct GpuRuns {
    glm: bool,
    qwen: bool,
}

/// `(setting, Lite's cloud value, what it needs)`. Only Lite's own cloud
/// value is offered: `auto` or anything else was chosen on purpose.
const OFFERABLE: &[(&str, &str, Needs)] = &[
    ("stt_mode", "assemblyai", Needs::Nothing),
    ("ocrh_mode", "glm_ocr", Needs::GlmGpu),
    ("llm_mode", "openrouter", Needs::QwenGpu),
];

fn pending_offer(lookup: impl Fn(&str) -> Option<String>, gpu: GpuRuns) -> Vec<ModeOffer> {
    if lookup(OFFERED_KEY).is_some() {
        return Vec::new();
    }
    OFFERABLE
        .iter()
        .filter(|(_, _, needs)| match needs {
            Needs::Nothing => true,
            Needs::GlmGpu => gpu.glm,
            Needs::QwenGpu => gpu.qwen,
        })
        .filter_map(|(key, cloud, _)| {
            let current = lookup(key)?;
            (current == *cloud).then_some(ModeOffer {
                key,
                current,
                local: "local",
            })
        })
        .collect()
}

/// The modes to offer switching to local, empty when there is nothing to
/// ask. An empty offer is recorded right away, so it is never asked later.
#[tauri::command]
pub async fn local_modes_offer(db: State<'_, AppDbState>) -> Result<Vec<ModeOffer>, String> {
    if !cfg!(feature = "local-ml") {
        return Ok(Vec::new());
    }
    let db = db.inner().clone();
    run_blocking_db_task(move || {
        // GPU detection spawns nvidia-smi: never with the connection held.
        #[cfg(feature = "local-ml")]
        let gpu = {
            use crate::ocr::llama_gpu::{engine_runs, Engine};
            GpuRuns {
                glm: engine_runs(Engine::GlmOcr),
                qwen: engine_runs(Engine::Qwen),
            }
        };
        #[cfg(not(feature = "local-ml"))]
        let gpu = GpuRuns::default();
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        let offer = pending_offer(|key| crate::settings::get_raw_setting(&conn, key), gpu);
        if offer.is_empty() && crate::settings::get_raw_setting(&conn, OFFERED_KEY).is_none() {
            crate::settings::set_setting(&conn, OFFERED_KEY, "1")
                .map_err(|e| format!("Failed to record the local modes offer: {e}"))?;
        }
        Ok(offer)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn offer(settings: &[(&str, &str)], has_gpu: bool) -> Vec<&'static str> {
        offer_with(
            settings,
            GpuRuns {
                glm: has_gpu,
                qwen: has_gpu,
            },
        )
    }

    fn offer_with(settings: &[(&str, &str)], gpu: GpuRuns) -> Vec<&'static str> {
        let map: HashMap<&str, &str> = settings.iter().copied().collect();
        pending_offer(|key| map.get(key).map(|v| v.to_string()), gpu)
            .into_iter()
            .map(|offer| offer.key)
            .collect()
    }

    const LITE_ARCHIVE: &[(&str, &str)] = &[
        ("stt_mode", "assemblyai"),
        ("ocrh_mode", "glm_ocr"),
        ("llm_mode", "openrouter"),
        ("embedding_provider", "api"),
    ];

    #[test]
    fn a_lite_archive_offers_every_local_engine_this_machine_runs() {
        assert_eq!(
            offer(LITE_ARCHIVE, true),
            ["stt_mode", "ocrh_mode", "llm_mode"]
        );
        assert_eq!(offer(LITE_ARCHIVE, false), ["stt_mode"]);
    }

    #[test]
    fn a_4_gb_gpu_offers_local_ocrh_but_not_the_local_llm() {
        assert_eq!(
            offer_with(
                LITE_ARCHIVE,
                GpuRuns {
                    glm: true,
                    qwen: false
                }
            ),
            ["stt_mode", "ocrh_mode"]
        );
    }

    #[test]
    fn deliberate_choices_and_answered_offers_are_left_alone() {
        assert!(offer(&[("stt_mode", "auto"), ("ocrh_mode", "local")], true).is_empty());
        assert!(offer(&[], true).is_empty());
        let answered = [("stt_mode", "assemblyai"), (OFFERED_KEY, "1")];
        assert!(offer(&answered, true).is_empty());
    }
}
