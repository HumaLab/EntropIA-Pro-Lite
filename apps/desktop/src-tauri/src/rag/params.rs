//! Parámetros runtime del pipeline RAG, leídos de `app_settings`.
//!
//! Las claves son un CONTRATO con el frontend (pantalla de configuración):
//! `rag_top_k`, `rag_min_similarity`, `rag_candidates_per_leg`, `rag_rrf_k`,
//! `rag_snippet_max_chars`, `rag_context_max_chars`, `rag_history_turns`,
//! `rag_history_turn_max_chars`, `rag_temperature`, `rag_max_tokens`.
//!
//! Mismo idioma que `model_params_from_settings` en `llm/mod.rs`: un setting
//! ausente, inválido o fuera de rango cae al default (nunca falla). Las
//! constantes `DEFAULT_*` de este módulo son la única fuente de defaults.

use rusqlite::Connection;

/// Fuentes finales por consulta (`rag_top_k`), rango 1..=20.
pub(crate) const DEFAULT_TOP_K: usize = 6;
pub(crate) const TOP_K_MIN: usize = 1;
pub(crate) const TOP_K_MAX: usize = 20;

/// Similitud coseno mínima de la pierna vectorial (`rag_min_similarity`),
/// rango 0.0..=1.0. `0.0` = filtro deshabilitado.
pub(crate) const DEFAULT_MIN_SIMILARITY: f64 = 0.0;

/// Candidatos por pierna antes de la fusión RRF (`rag_candidates_per_leg`),
/// rango 4..=200.
pub(crate) const DEFAULT_LEG_CANDIDATES: usize = 24;

/// Constante de amortiguación RRF (`rag_rrf_k`), rango 1..=500. Entero por
/// contrato con la UI; retrieval lo convierte a f64 donde lo necesita.
pub(crate) const DEFAULT_RRF_K: usize = 60;

/// Máximo de caracteres por snippet de fuente (`rag_snippet_max_chars`),
/// rango 200..=8000.
pub(crate) const DEFAULT_SNIPPET_MAX_CHARS: usize = 1600;

/// Máximo total de caracteres de contexto (`rag_context_max_chars`),
/// rango 1000..=350000. El techo cubre el presupuesto de evidencia de la
/// ventana local de 131K tokens (~116K tokens ≈ 350K chars); el default se
/// mantiene conservador para no cambiar el comportamiento de Lite por API.
pub(crate) const DEFAULT_CONTEXT_MAX_CHARS: usize = 10_000;
pub(crate) const CONTEXT_MAX_CHARS_LIMIT: usize = 350_000;

/// Turnos persistidos incluidos como historial (`rag_history_turns`),
/// rango 0..=20.
pub(crate) const DEFAULT_HISTORY_TURNS: usize = 6;

/// Truncado por turno de historial (`rag_history_turn_max_chars`),
/// rango 100..=4000.
pub(crate) const DEFAULT_HISTORY_TURN_MAX_CHARS: usize = 500;

/// Temperatura del chat completion (`rag_temperature`), rango 0.0..=2.0.
pub(crate) const DEFAULT_TEMPERATURE: f32 = 0.2;

/// Máximo de tokens del chat completion (`rag_max_tokens`), rango 64..=16000.
pub(crate) const DEFAULT_MAX_TOKENS: i32 = 4096;

/// Parámetros efectivos de una consulta RAG. Se leen una vez por pregunta en
/// el mismo scope de lock que el resto de los settings de `rag_ask`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RagParams {
    pub top_k: usize,
    pub min_similarity: f64,
    pub candidates_per_leg: usize,
    pub rrf_k: usize,
    pub snippet_max_chars: usize,
    pub context_max_chars: usize,
    pub history_turns: usize,
    pub history_turn_max_chars: usize,
    pub temperature: f32,
    pub max_tokens: i32,
}

impl Default for RagParams {
    fn default() -> Self {
        Self {
            top_k: DEFAULT_TOP_K,
            min_similarity: DEFAULT_MIN_SIMILARITY,
            candidates_per_leg: DEFAULT_LEG_CANDIDATES,
            rrf_k: DEFAULT_RRF_K,
            snippet_max_chars: DEFAULT_SNIPPET_MAX_CHARS,
            context_max_chars: DEFAULT_CONTEXT_MAX_CHARS,
            history_turns: DEFAULT_HISTORY_TURNS,
            history_turn_max_chars: DEFAULT_HISTORY_TURN_MAX_CHARS,
            temperature: DEFAULT_TEMPERATURE,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }
}

/// Lee los parámetros RAG desde `app_settings`. Cada clave ausente, inválida
/// o fuera de rango cae a su default — esta función nunca falla.
///
/// Invariante cross-field: `snippet_max_chars <= context_max_chars`. Si los
/// settings lo violan, el snippet se recorta al tope de contexto.
pub(crate) fn rag_params_from_settings(conn: &Connection) -> RagParams {
    let mut params = RagParams {
        top_k: parse_setting_usize(conn, "rag_top_k", DEFAULT_TOP_K, TOP_K_MIN, TOP_K_MAX),
        min_similarity: parse_setting_f64(
            conn,
            "rag_min_similarity",
            DEFAULT_MIN_SIMILARITY,
            0.0,
            1.0,
        ),
        candidates_per_leg: parse_setting_usize(
            conn,
            "rag_candidates_per_leg",
            DEFAULT_LEG_CANDIDATES,
            4,
            200,
        ),
        rrf_k: parse_setting_usize(conn, "rag_rrf_k", DEFAULT_RRF_K, 1, 500),
        snippet_max_chars: parse_setting_usize(
            conn,
            "rag_snippet_max_chars",
            DEFAULT_SNIPPET_MAX_CHARS,
            200,
            8000,
        ),
        context_max_chars: parse_setting_usize(
            conn,
            "rag_context_max_chars",
            DEFAULT_CONTEXT_MAX_CHARS,
            1000,
            CONTEXT_MAX_CHARS_LIMIT,
        ),
        history_turns: parse_setting_usize(conn, "rag_history_turns", DEFAULT_HISTORY_TURNS, 0, 20),
        history_turn_max_chars: parse_setting_usize(
            conn,
            "rag_history_turn_max_chars",
            DEFAULT_HISTORY_TURN_MAX_CHARS,
            100,
            4000,
        ),
        temperature: parse_setting_f32(conn, "rag_temperature", DEFAULT_TEMPERATURE, 0.0, 2.0),
        max_tokens: parse_setting_i32(conn, "rag_max_tokens", DEFAULT_MAX_TOKENS, 64, 16_000),
    };
    params.snippet_max_chars = params.snippet_max_chars.min(params.context_max_chars);
    params
}

// ── Helpers de parseo (espejo del idioma de llm/mod.rs) ──────────────────────

fn parse_setting_f32(conn: &Connection, key: &str, default: f32, min: f32, max: f32) -> f32 {
    crate::settings::get_setting(conn, key)
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value >= min && *value <= max)
        .unwrap_or(default)
}

fn parse_setting_f64(conn: &Connection, key: &str, default: f64, min: f64, max: f64) -> f64 {
    crate::settings::get_setting(conn, key)
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= min && *value <= max)
        .unwrap_or(default)
}

fn parse_setting_i32(conn: &Connection, key: &str, default: i32, min: i32, max: i32) -> i32 {
    crate::settings::get_setting(conn, key)
        .and_then(|value| value.trim().parse::<i32>().ok())
        .filter(|value| *value >= min && *value <= max)
        .unwrap_or(default)
}

fn parse_setting_usize(
    conn: &Connection,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> usize {
    crate::settings::get_setting(conn, key)
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value >= min && *value <= max)
        .unwrap_or(default)
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn conn_with_settings(pairs: &[(&str, &str)]) -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("app_settings schema creation failed");
        for (key, value) in pairs {
            conn.execute(
                "INSERT INTO app_settings(key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .expect("setting insert failed");
        }
        conn
    }

    #[test]
    fn rag_params_defaults_when_settings_unset() {
        let conn = conn_with_settings(&[]);
        assert_eq!(rag_params_from_settings(&conn), RagParams::default());
    }

    #[test]
    fn rag_params_defaults_when_settings_table_missing() {
        // Base sin app_settings: get_setting devuelve None y todo cae a default.
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        assert_eq!(rag_params_from_settings(&conn), RagParams::default());
    }

    #[test]
    fn rag_params_parses_valid_values_for_every_key() {
        let conn = conn_with_settings(&[
            ("rag_top_k", "12"),
            ("rag_min_similarity", "0.35"),
            ("rag_candidates_per_leg", "48"),
            ("rag_rrf_k", "90"),
            ("rag_snippet_max_chars", "2400"),
            ("rag_context_max_chars", "20000"),
            ("rag_history_turns", "0"),
            ("rag_history_turn_max_chars", "1000"),
            ("rag_temperature", "0.7"),
            ("rag_max_tokens", "4096"),
        ]);
        let params = rag_params_from_settings(&conn);
        assert_eq!(params.top_k, 12);
        assert!((params.min_similarity - 0.35).abs() < 1e-12);
        assert_eq!(params.candidates_per_leg, 48);
        assert_eq!(params.rrf_k, 90);
        assert_eq!(params.snippet_max_chars, 2400);
        assert_eq!(params.context_max_chars, 20_000);
        assert_eq!(params.history_turns, 0);
        assert_eq!(params.history_turn_max_chars, 1000);
        assert!((params.temperature - 0.7).abs() < 1e-6);
        assert_eq!(params.max_tokens, 4096);
    }

    #[test]
    fn rag_params_trims_whitespace_before_parsing() {
        let conn = conn_with_settings(&[("rag_top_k", "  8  ")]);
        assert_eq!(rag_params_from_settings(&conn).top_k, 8);
    }

    #[test]
    fn rag_params_falls_back_on_garbage_values() {
        let conn = conn_with_settings(&[
            ("rag_top_k", "muchos"),
            ("rag_min_similarity", "NaN"),
            ("rag_rrf_k", "infinity y más allá"),
            ("rag_temperature", ""),
            ("rag_max_tokens", "4.5"),
        ]);
        assert_eq!(rag_params_from_settings(&conn), RagParams::default());
    }

    #[test]
    fn rag_params_falls_back_on_out_of_range_values() {
        let conn = conn_with_settings(&[
            ("rag_top_k", "21"),
            ("rag_min_similarity", "1.5"),
            ("rag_candidates_per_leg", "3"),
            ("rag_rrf_k", "0"),
            ("rag_snippet_max_chars", "199"),
            ("rag_context_max_chars", "350001"),
            ("rag_history_turns", "-1"),
            ("rag_history_turn_max_chars", "99"),
            ("rag_temperature", "2.5"),
            ("rag_max_tokens", "63"),
        ]);
        assert_eq!(rag_params_from_settings(&conn), RagParams::default());
    }

    #[test]
    fn rag_params_accepts_range_boundaries() {
        let conn = conn_with_settings(&[
            ("rag_top_k", "20"),
            ("rag_min_similarity", "1.0"),
            ("rag_candidates_per_leg", "4"),
            ("rag_rrf_k", "500"),
            ("rag_max_tokens", "16000"),
            ("rag_context_max_chars", "350000"),
        ]);
        let params = rag_params_from_settings(&conn);
        assert_eq!(params.top_k, 20);
        assert!((params.min_similarity - 1.0).abs() < 1e-12);
        assert_eq!(params.candidates_per_leg, 4);
        assert_eq!(params.rrf_k, 500);
        assert_eq!(params.max_tokens, 16_000);
        assert_eq!(params.context_max_chars, CONTEXT_MAX_CHARS_LIMIT);
    }

    #[test]
    fn rag_params_rejects_more_than_sixteen_thousand_generation_tokens() {
        let conn = conn_with_settings(&[("rag_max_tokens", "16001")]);
        assert_eq!(
            rag_params_from_settings(&conn).max_tokens,
            DEFAULT_MAX_TOKENS
        );
    }

    #[test]
    fn rag_params_rrf_k_rejects_non_integer_text() {
        // La UI valida rrfK como entero: "60.5" no debe parsear (caía como
        // f64 antes del Cambio de simetría).
        let conn = conn_with_settings(&[("rag_rrf_k", "60.5")]);
        assert_eq!(rag_params_from_settings(&conn).rrf_k, DEFAULT_RRF_K);
    }

    #[test]
    fn rag_params_clamps_snippet_to_context_budget() {
        // Ambos válidos en su rango individual, pero snippet > context:
        // el invariante cross-field recorta el snippet al tope de contexto.
        let conn = conn_with_settings(&[
            ("rag_snippet_max_chars", "5000"),
            ("rag_context_max_chars", "2000"),
        ]);
        let params = rag_params_from_settings(&conn);
        assert_eq!(params.context_max_chars, 2000);
        assert_eq!(params.snippet_max_chars, 2000);
    }

    #[test]
    fn rag_params_keeps_snippet_when_within_context_budget() {
        let conn = conn_with_settings(&[
            ("rag_snippet_max_chars", "1500"),
            ("rag_context_max_chars", "2000"),
        ]);
        let params = rag_params_from_settings(&conn);
        assert_eq!(params.snippet_max_chars, 1500);
        assert_eq!(params.context_max_chars, 2000);
    }
}
