//! The diagnostic snapshot the help assistant may send with a question (S5).
//!
//! Built here, in Rust, so the text the user previews is the text that leaves
//! (D3). Settings travel only from an allowlist of mode and model keys; API
//! keys travel only as configured or not, never their value; log lines lose
//! URLs paths, file paths, file names and quoted text (where document names and
//! snippets appear) on top of the credential redaction the log already applies.

use std::sync::OnceLock;

use regex::Regex;

use crate::app_logs::{sanitize_field, AppLogEntry, AppLogLevel};

/// Mode and model settings that say how the app is wired, never what it holds.
pub(crate) const SETTINGS_ALLOWLIST: &[&str] = &[
    "llm_mode",
    "openrouter_model",
    "rag_model",
    "rag_reranker_model",
    "openrouter_ner_model",
    "llm_ocr_correction_model",
    "llm_summary_model",
    "llm_triplets_model",
    "embedding_provider",
    "openrouter_embedding_model",
    "stt_mode",
    "ocrh_mode",
    "language",
    "search_fuzzy",
];

/// API keys reported as configured or not; `(setting key, label)`.
pub(crate) const KEY_SETTINGS: &[(&str, &str)] = &[
    ("openrouter_api_key", "OpenRouter"),
    ("vercel_ai_gateway_api_key", "Vercel AI Gateway"),
    ("assemblyai_api_key", "AssemblyAI"),
    ("glm_ocr_api_key", "GLM-OCR"),
    ("zotero_api_key", "Zotero"),
    ("hlab_publish_key", "hlab"),
];

/// Longest single line kept; longer ones are cut and marked.
const MAX_LINE_CHARS: usize = 2_000;

/// Most log entries that go along.
pub(crate) const MAX_LOG_ENTRIES: usize = 30;

/// Longest diagnostic text accepted back from the frontend.
pub(crate) const MAX_REPORT_CHARS: usize = 16_000;

/// Everything the snapshot is built from, gathered by the command.
pub(crate) struct DiagnosticInputs {
    pub product: String,
    pub version: String,
    pub os: String,
    /// Raw `app_settings` rows the command read; the builder filters them.
    pub settings: Vec<(String, String)>,
    pub logs: Vec<AppLogEntry>,
}

/// The snapshot as the user previews it and the model reads it.
pub(crate) fn build_report(inputs: &DiagnosticInputs) -> String {
    let value_of = |key: &str| {
        inputs
            .settings
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
    };

    let mut out = Vec::new();
    out.push(scrub_line(&format!(
        "Producto: {} {} ({})",
        inputs.product, inputs.version, inputs.os
    )));

    out.push(String::new());
    out.push("Configuración:".to_string());
    for key in SETTINGS_ALLOWLIST {
        if let Some(value) = value_of(key) {
            out.push(scrub_line(&format!("- {key}: {value}")));
        }
    }

    out.push(String::new());
    out.push("Claves de API:".to_string());
    for (key, label) in KEY_SETTINGS {
        let state = if value_of(key).is_some() {
            "configurada"
        } else {
            "no configurada"
        };
        out.push(format!("- {label}: {state}"));
    }

    let relevant: Vec<&AppLogEntry> = inputs
        .logs
        .iter()
        .filter(|entry| !matches!(entry.level, AppLogLevel::Info))
        .collect();
    let recent = &relevant[relevant.len().saturating_sub(MAX_LOG_ENTRIES)..];
    out.push(String::new());
    if recent.is_empty() {
        out.push("Sin avisos ni errores recientes.".to_string());
    } else {
        out.push(format!("Últimos avisos y errores ({}):", recent.len()));
        for entry in recent {
            let level = match entry.level {
                AppLogLevel::Info => "info",
                AppLogLevel::Warn => "aviso",
                AppLogLevel::Error => "error",
            };
            let message = entry.message.replace(['\r', '\n'], " ");
            out.push(scrub_line(&format!(
                "- {} {level} [{}] {message}",
                format_utc(entry.timestamp_ms),
                entry.source
            )));
        }
    }
    out.join("\n")
}

struct Patterns {
    url: Regex,
    windows_path: Regex,
    unc_path: Regex,
    unix_path: Regex,
    file_name: Regex,
    double_quoted: Regex,
    single_quoted: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let re = |pattern: &str| Regex::new(pattern).expect("valid scrub pattern");
        Patterns {
            // Keep scheme and host (which service failed), drop the path.
            url: re(r#"(https?)://([^/\s"'«»]+)[^\s"'«»]*"#),
            // Folder names may hold spaces; the last part may not.
            windows_path: re(r#"[A-Za-z]:\\(?:[^\\\n:"'«»]*\\)*[^\\\s:"'«»]*"#),
            unc_path: re(r#"\\\\(?:[^\\\n:"'«»]*\\)*[^\\\s:"'«»]*"#),
            unix_path: re(r#"(^|[\s(=:,\[])~?/[^/\s"'«»)\]][^\s"'«»)\]]*"#),
            file_name: re(
                r#"[^\s/\\"'«»\[\]]+\.(?i:pdf|jpe?g|png|tiff?|gif|webp|bmp|heic|txt|md|docx?|odt|rtf|epub|mp3|wav|m4a|ogg|flac|mp4|mov|mkv|webm|csv|xlsx?|json|xml|html?|zip)\b"#,
            ),
            double_quoted: re(r#""[^"\n]*"|«[^»\n]*»|“[^”\n]*”|`[^`\n]*`"#),
            single_quoted: re(r#"(^|[\s(=:,\[])'[^'\n]*'"#),
        }
    })
}

/// One line without what could identify the user's documents or credentials:
/// URL paths, file paths, file names and quoted text become placeholders, and
/// the log's own credential redaction runs last.
pub(crate) fn scrub_line(line: &str) -> String {
    let p = patterns();
    let line = p.url.replace_all(line, "$1://$2");
    let line = p.windows_path.replace_all(&line, "[ruta]");
    let line = p.unc_path.replace_all(&line, "[ruta]");
    let line = p.unix_path.replace_all(&line, "${1}[ruta]");
    let line = p.file_name.replace_all(&line, "[archivo]");
    let line = p.double_quoted.replace_all(&line, "[texto]");
    let line = p.single_quoted.replace_all(&line, "${1}[texto]");
    sanitize_field(line.into_owned(), MAX_LINE_CHARS)
}

/// Scrubs a report again on its way out (the frontend sends back the text it
/// previewed) and bounds its size.
pub(crate) fn scrub_report(report: &str) -> String {
    let scrubbed = report
        .lines()
        .map(scrub_line)
        .collect::<Vec<_>>()
        .join("\n");
    if scrubbed.chars().count() <= MAX_REPORT_CHARS {
        return scrubbed;
    }
    let mut cut: String = scrubbed.chars().take(MAX_REPORT_CHARS).collect();
    cut.push_str("… [truncado]");
    cut
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, without a date library.
fn format_utc(timestamp_ms: u64) -> String {
    let secs = timestamp_ms / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, level: AppLogLevel, message: &str) -> AppLogEntry {
        AppLogEntry {
            id,
            timestamp_ms: 1_763_000_000_000 + id * 1000,
            level,
            source: "transcription".to_string(),
            message: message.to_string(),
        }
    }

    fn inputs(logs: Vec<AppLogEntry>) -> DiagnosticInputs {
        DiagnosticInputs {
            product: "EntropIA Lite".to_string(),
            version: "1.4.0".to_string(),
            os: "windows x86_64".to_string(),
            settings: vec![
                ("llm_mode".into(), "openrouter".into()),
                (
                    "openrouter_model".into(),
                    "google/gemma-4-26b-a4b-it".into(),
                ),
                ("stt_mode".into(), "assemblyai".into()),
                (
                    "openrouter_api_key".into(),
                    "sk-or-v1-0123456789abcdef0123456789abcdef".into(),
                ),
                ("assemblyai_api_key".into(), "   ".into()),
                ("zotero_api_key".into(), "secret_ref:zotero_api_key".into()),
                (
                    "prompt_summary".into(),
                    "Resumí las actas de la Sociedad Rural".into(),
                ),
                ("rag_active_conversation".into(), "conv-123".into()),
            ],
            logs,
        }
    }

    #[test]
    fn scrub_line_drops_paths_file_names_quotes_urls_and_credentials() {
        let cases = [
            (
                r"No se pudo abrir C:\Users\ana\Documents\Actas 1890\acta.pdf: acceso denegado",
                "acceso denegado",
                &["ana", "Actas", "acta.pdf"][..],
            ),
            (
                "fallo al leer /home/ana/archivo/actas/acta-12.pdf (os error 2)",
                "os error 2",
                &["ana", "acta-12"],
            ),
            (
                "el documento \"Carta de Sarmiento a Mitre\" no tiene páginas",
                "no tiene páginas",
                &["Sarmiento", "Mitre"],
            ),
            (
                "título «Memorias del Coronel Dorrego» sin texto",
                "sin texto",
                &["Dorrego"],
            ),
            (
                "importado Diario_1902.jpg con 3 páginas",
                "con 3 páginas",
                &["Diario_1902"],
            ),
            (
                "GET https://archive.org/download/memorias-dorrego/file.pdf -> 404",
                "archive.org",
                &["memorias-dorrego", "file.pdf"],
            ),
            (
                "AssemblyAI respondió 401 Authorization: Bearer abcdefabcdefabcdefabcdefabcdefabcdef12",
                "401",
                &["abcdefabcdefabcdefabcdefabcdefabcdef12"],
            ),
        ];
        for (raw, kept, gone) in cases {
            let scrubbed = scrub_line(raw);
            assert!(scrubbed.contains(kept), "{scrubbed:?} lost {kept:?}");
            for word in gone {
                assert!(!scrubbed.contains(word), "{scrubbed:?} still has {word:?}");
            }
        }
    }

    #[test]
    fn report_states_keys_as_configured_and_never_their_value() {
        let report = build_report(&inputs(Vec::new()));
        assert!(report.contains("EntropIA Lite 1.4.0"), "{report}");
        assert!(report.contains("windows x86_64"), "{report}");
        assert!(report.contains("OpenRouter: configurada"), "{report}");
        assert!(report.contains("AssemblyAI: no configurada"), "{report}");
        assert!(report.contains("Zotero: configurada"), "{report}");
        assert!(!report.contains("sk-or-v1"), "{report}");
        assert!(!report.contains("secret_ref"), "{report}");
    }

    #[test]
    fn report_carries_only_allowlisted_settings() {
        let report = build_report(&inputs(Vec::new()));
        assert!(report.contains("llm_mode: openrouter"), "{report}");
        assert!(
            report.contains("openrouter_model: google/gemma-4-26b-a4b-it"),
            "{report}"
        );
        assert!(report.contains("stt_mode: assemblyai"), "{report}");
        assert!(!report.contains("Sociedad Rural"), "{report}");
        assert!(!report.contains("conv-123"), "{report}");
        assert!(!report.contains("prompt_summary"), "{report}");
    }

    #[test]
    fn report_keeps_the_last_warnings_and_errors_scrubbed() {
        let mut logs = vec![entry(1, AppLogLevel::Info, "arranque normal")];
        for id in 2..=40 {
            logs.push(entry(id, AppLogLevel::Warn, &format!("aviso número {id}")));
        }
        logs.push(entry(
            41,
            AppLogLevel::Error,
            "falló \"Carta de Sarmiento\" con 401",
        ));
        let report = build_report(&inputs(logs));
        assert!(
            !report.contains("arranque normal"),
            "info lines must stay home"
        );
        assert!(report.contains("falló [texto] con 401"), "{report}");
        assert!(report.contains("aviso número 40"), "{report}");
        assert!(!report.contains("aviso número 11\n"), "{report}");
        let listed = report.lines().filter(|l| l.starts_with("- 20")).count();
        assert_eq!(listed, MAX_LOG_ENTRIES, "{report}");
        assert!(report.contains("[transcription]"), "{report}");
    }

    #[test]
    fn timestamps_render_as_utc_dates() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(1_763_000_000_000), "2025-11-13 02:13:20");
        assert_eq!(format_utc(951_782_400_000), "2000-02-29 00:00:00");
    }

    #[test]
    fn a_report_without_logs_says_so() {
        let report = build_report(&inputs(Vec::new()));
        assert!(
            report.contains("Sin avisos ni errores recientes."),
            "{report}"
        );
    }

    #[test]
    fn scrub_report_leaves_a_built_report_unchanged_and_scrubs_an_edited_one() {
        let logs = vec![entry(
            1,
            AppLogLevel::Error,
            r"no abre D:\Fondo\caja 3\foto.png",
        )];
        let report = build_report(&inputs(logs));
        assert_eq!(scrub_report(&report), report);

        let edited = format!("{report}\nagregado: C:\\Users\\ana\\tesis.docx token abcdefabcdefabcdefabcdefabcdefabcdef12");
        let scrubbed = scrub_report(&edited);
        assert!(!scrubbed.contains("tesis"), "{scrubbed}");
        assert!(
            !scrubbed.contains("abcdefabcdefabcdefabcdefabcdefabcdef12"),
            "{scrubbed}"
        );

        let long = "x ".repeat(MAX_REPORT_CHARS);
        assert!(scrub_report(&long).chars().count() <= MAX_REPORT_CHARS + 20);
    }
}
