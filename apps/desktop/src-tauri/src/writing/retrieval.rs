//! Corpus evidence for the writing agent (plan-editor.md §14.1, §14.3).
//!
//! # Why the writing module has its own entry to retrieval
//!
//! Because the only one that existed answered questions. `rag_ask` retrieves,
//! reranks, builds a prompt and generates a reply, and none of that is what the
//! four evidence actions of §14.1 need: they want the passages, so the writer
//! can see what a proposal rests on and check it. Asking `rag_ask` and throwing
//! away its answer would pay for a generation nobody reads.
//!
//! So this reuses the retrieval that is already there — the same hybrid legs,
//! the same parameters read from the same settings — and stops before the part
//! that costs a model call.
//!
//! # Why the rerank is skipped
//!
//! Reranking calls a provider once per retrieval, and these actions are meant
//! to be pressed while writing. The fused candidates are ordered well enough to
//! put evidence in front of someone who is going to read it and judge it
//! themselves — which is the whole premise of §14.2 — and the writer is not
//! charged for a second model call to reorder a list they can see.

use super::repository::{WritingError, WritingResult};
use crate::rag::retrieval::RrfCandidate;

/// One passage of the corpus, as the agent will be given it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RetrievedPassage {
    /// The identity §14.2 needs to call this evidence rather than prose: the
    /// writer can open it, and `evidenceOf` can record that text was consulted.
    pub asset_id: String,
    pub item_id: String,
    pub item_title: String,
    pub collection_id: String,
    pub collection_name: String,
    pub text: String,
    /// Where in the source it starts, so a citation from it can be anchored.
    pub start_char: usize,
}

/// Cuts a passage at a boundary a reader would accept.
///
/// Mid-word is the one place it must not land: `…la organiz` reads as a defect
/// in the application rather than as a passage that was shortened. So the cut
/// walks back to the last space, unless that would throw away most of what was
/// kept.
fn clip(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let cut: String = trimmed.chars().take(max_chars).collect();
    let boundary = cut.rfind(char::is_whitespace).unwrap_or(cut.len());
    // Only honour the boundary when it keeps most of the passage; a passage
    // with no spaces in its last half would otherwise lose that half.
    let kept = if boundary * 2 >= cut.len() {
        &cut[..boundary]
    } else {
        cut.as_str()
    };
    format!("{}…", kept.trim_end())
}

/// Turns retrieval candidates into the passages the agent is given.
///
/// Deduplicated by source and offset, because the legs of a hybrid retrieval
/// find the same chunk by different routes and fusion does not always collapse
/// them. The same passage twice is not more evidence; it is one piece of
/// evidence taking the budget of two.
pub(crate) fn passages(
    candidates: Vec<RrfCandidate>,
    limit: usize,
    max_chars: usize,
) -> Vec<RetrievedPassage> {
    let mut seen: Vec<(String, usize)> = Vec::new();
    let mut out = Vec::new();

    for candidate in candidates {
        if out.len() >= limit {
            break;
        }
        let record = candidate.record;
        let text = clip(&record.text_content, max_chars);
        // A passage with no words is not evidence, and showing it as a source
        // would make the agent look as though it had read something.
        if text.is_empty() {
            continue;
        }

        let identity = (record.asset_id.clone(), record.source_start_char);
        if seen.contains(&identity) {
            continue;
        }
        seen.push(identity);

        out.push(RetrievedPassage {
            asset_id: record.asset_id,
            item_id: record.item_id,
            item_title: record.item_title,
            collection_id: record.collection_id,
            collection_name: record.collection_name,
            text,
            start_char: record.source_start_char,
        });
    }

    out
}

/// The body of `writing_corpus_retrieve`, without Tauri state: settings read
/// from `settings`, the query embedded by `embed` (a network call in Lite,
/// made without holding `worker`), then the shared hybrid retrieval on
/// `worker` and its passages. The rerank is skipped, see the module docs.
pub(crate) fn corpus_evidence(
    settings: &rusqlite::Connection,
    worker: &std::sync::Mutex<rusqlite::Connection>,
    passage: &str,
    limit: usize,
    embed: impl FnOnce(&str) -> Option<Vec<f32>>,
) -> WritingResult<Vec<RetrievedPassage>> {
    let query = require_query(passage)?;
    let wanted = bounded(limit);

    let params = crate::rag::params::rag_params_from_settings(settings);
    let unit_setting = crate::settings::get_setting(settings, "rag_retrieval_unit");
    let unit = crate::rag::retrieval::RetrievalUnit::from_setting(unit_setting.as_deref())
        .map_err(|e| WritingError::new("retrieval_unit_invalid", e))?;

    let embedding = embed(query);
    let queries = [crate::rag::retrieval::RetrievalQuery {
        text: query,
        embedding: embedding.as_deref(),
    }];

    let conn = worker
        .lock()
        .map_err(|_| WritingError::new("db_unavailable", "la conexión de trabajo está tomada"))?;
    let candidates =
        crate::rag::retrieval::hybrid_retrieve_candidates(&conn, &queries, &params, unit)
            .map_err(|e| WritingError::new("retrieval_failed", e))?;

    Ok(passages(candidates, wanted, params.snippet_max_chars))
}

/// How many passages an action may be given, whatever it asks for.
///
/// The context builder has a budget of its own and reports what it drops, so
/// this is not the only guard — but a retrieval that returned forty passages
/// would push the writer's own selection down the priority order for no gain.
pub const MAX_PASSAGES: usize = 8;

pub fn bounded(limit: usize) -> usize {
    limit.clamp(1, MAX_PASSAGES)
}

/// A passage the writer has not actually selected anything for.
pub fn require_query(passage: &str) -> WritingResult<&str> {
    let trimmed = passage.trim();
    if trimmed.is_empty() {
        return Err(WritingError::new(
            "no_selection",
            "no hay un pasaje sobre el cual buscar evidencia",
        ));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::retrieval::SourceRecord;

    fn record(asset: &str, text: &str, start: usize) -> RrfCandidate {
        RrfCandidate {
            record: SourceRecord {
                asset_id: asset.into(),
                item_id: "it1".into(),
                item_title: "Acta del gremio".into(),
                collection_id: "col1".into(),
                collection_name: "Movimiento obrero".into(),
                text_content: text.into(),
                segments_json: None,
                transcription_offset_chars: None,
                source_start_char: start,
                chunk: None,
            },
            score: 1.0,
        }
    }

    fn settings(unit: Option<&str>) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("settings DB");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("settings table");
        if let Some(unit) = unit {
            conn.execute(
                "INSERT INTO app_settings(key, value) VALUES ('rag_retrieval_unit', ?1)",
                [unit],
            )
            .expect("unit setting");
        }
        conn
    }

    fn evidence(unit: Option<&str>, embedding: Option<Vec<f32>>) -> Vec<RetrievedPassage> {
        let worker = std::sync::Mutex::new(crate::rag::retrieval::hybrid_fixture::corpus());
        corpus_evidence(&settings(unit), &worker, "  cabildo ", 8, |query| {
            assert_eq!(query, "cabildo", "the query is trimmed before embedding");
            embedding
        })
        .expect("corpus evidence")
    }

    #[test]
    fn corpus_evidence_keeps_pages_without_chunks_under_the_default_unit() {
        use crate::rag::retrieval::hybrid_fixture::{
            axis, CHUNKED_ASSET, CHUNK_START, PLAIN_ASSET,
        };
        for (setting, embedding) in [(None, Some(axis(2))), (Some("chunk"), None)] {
            let found: Vec<(String, usize)> = evidence(setting, embedding)
                .into_iter()
                .map(|passage| (passage.asset_id, passage.start_char))
                .collect();
            assert!(
                found.contains(&(CHUNKED_ASSET.to_string(), CHUNK_START)),
                "{setting:?}: the chunked page arrives as its passage: {found:?}"
            );
            assert!(
                found.contains(&(PLAIN_ASSET.to_string(), 0)),
                "{setting:?}: the page without chunks still arrives whole: {found:?}"
            );
        }
    }

    #[test]
    fn corpus_evidence_rejects_an_unknown_retrieval_unit_and_an_empty_passage() {
        let worker = std::sync::Mutex::new(crate::rag::retrieval::hybrid_fixture::corpus());
        let error = corpus_evidence(&settings(Some("page")), &worker, "cabildo", 8, |_| None)
            .expect_err("unknown unit");
        assert_eq!(error.code, "retrieval_unit_invalid");
        let error = corpus_evidence(&settings(None), &worker, "   ", 8, |_| {
            panic!("nothing is embedded for an empty passage")
        })
        .expect_err("empty passage");
        assert_eq!(error.code, "no_selection");
    }

    #[test]
    fn carries_the_identity_that_makes_a_passage_evidence() {
        let out = passages(
            vec![record("as1", "los obreros declararon la huelga", 40)],
            8,
            500,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].asset_id, "as1");
        assert_eq!(out[0].item_title, "Acta del gremio");
        assert_eq!(out[0].start_char, 40);
    }

    /// The legs of a hybrid retrieval find the same chunk by different routes.
    /// The same passage twice is one piece of evidence taking the budget of two.
    #[test]
    fn does_not_hand_the_same_passage_over_twice() {
        let out = passages(
            vec![
                record("as1", "los obreros declararon la huelga", 40),
                record("as1", "los obreros declararon la huelga", 40),
                record("as2", "la ciudad quedo detenida", 0),
            ],
            8,
            500,
        );

        assert_eq!(
            out.iter().map(|p| p.asset_id.as_str()).collect::<Vec<_>>(),
            ["as1", "as2"]
        );
    }

    /// The same asset at a different offset is a different passage of it.
    #[test]
    fn keeps_two_passages_of_one_source() {
        let out = passages(
            vec![record("as1", "primero", 0), record("as1", "segundo", 900)],
            8,
            500,
        );

        assert_eq!(out.len(), 2);
    }

    #[test]
    fn stops_at_the_limit_it_was_given() {
        let candidates = (0..20)
            .map(|at| record(&format!("as{at}"), "un fragmento", at))
            .collect();

        assert_eq!(passages(candidates, 3, 500).len(), 3);
    }

    /// A passage with no words would make the agent look as though it had read
    /// something.
    #[test]
    fn drops_a_passage_with_nothing_in_it() {
        let out = passages(
            vec![record("as1", "   \n  ", 0), record("as2", "algo", 0)],
            8,
            500,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].asset_id, "as2");
    }

    /// `…la organiz` reads as a defect in the application rather than as a
    /// passage that was shortened.
    #[test]
    fn never_cuts_in_the_middle_of_a_word() {
        let out = passages(
            vec![record("as1", "la organizacion sindical portuaria", 0)],
            8,
            12,
        );

        assert!(out[0].text.ends_with('…'), "{}", out[0].text);
        assert!(!out[0].text.contains("organiz…"), "{}", out[0].text);
    }

    #[test]
    fn leaves_a_short_passage_whole_and_unmarked() {
        let out = passages(vec![record("as1", "  corto  ", 0)], 8, 500);

        assert_eq!(out[0].text, "corto");
    }

    /// A passage whose second half has no spaces would otherwise lose that half.
    #[test]
    fn does_not_throw_away_most_of_a_passage_to_find_a_space() {
        let out = passages(vec![record("as1", "a bbbbbbbbbbbbbbbbbbbbbb", 0)], 8, 16);

        assert!(out[0].text.len() > 4, "{}", out[0].text);
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    #[test]
    fn bounds_what_an_action_may_ask_for() {
        assert_eq!(bounded(0), 1);
        assert_eq!(bounded(4), 4);
        assert_eq!(bounded(9999), MAX_PASSAGES);
    }

    #[test]
    fn refuses_to_search_for_evidence_about_nothing() {
        assert_eq!(require_query("   ").unwrap_err().code, "no_selection");
        assert_eq!(require_query("  el pasaje  ").unwrap(), "el pasaje");
    }
}
