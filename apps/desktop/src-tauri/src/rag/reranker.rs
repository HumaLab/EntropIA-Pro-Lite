//! Lite-only remote reranking for the owned RRF candidate pool.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::retrieval::RrfCandidate;

const RERANK_DOCUMENT_MAX_CHARS: usize = 6000;
#[cfg(not(feature = "local-ml"))]
const OPENROUTER_RERANK_URL: &str = "https://openrouter.ai/api/v1/rerank";
#[cfg(not(feature = "local-ml"))]
const OPENROUTER_RERANK_MODEL: &str = "cohere/rerank-4-fast";

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct RerankRequest<'a> {
    model: &'static str,
    query: &'a str,
    documents: &'a [String],
    top_n: usize,
}

#[derive(Debug, Deserialize)]
struct RerankResponse {
    results: Vec<RerankResult>,
}

#[derive(Debug, Deserialize)]
struct RerankResult {
    index: usize,
    relevance_score: f64,
}

/// Reranks Lite candidates through OpenRouter. Every failure preserves the
/// complete original RRF list so final source construction remains available.
#[cfg(not(feature = "local-ml"))]
pub(crate) async fn rerank_candidates(
    question: &str,
    candidates: Vec<RrfCandidate>,
    api_key: &str,
    top_n: usize,
) -> Vec<RrfCandidate> {
    if candidates.is_empty() {
        return candidates;
    }
    if api_key.trim().is_empty() {
        warn_fallback("missing OpenRouter API key");
        return candidates;
    }

    let documents = candidate_documents(question, &candidates);
    let request = RerankRequest {
        model: OPENROUTER_RERANK_MODEL,
        query: question,
        documents: &documents,
        top_n,
    };
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            warn_fallback("HTTP client initialization failed");
            return candidates;
        }
    };

    let response = match client
        .post(OPENROUTER_RERANK_URL)
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => {
            warn_fallback("request failed or timed out");
            return candidates;
        }
    };

    if !response.status().is_success() {
        warn_fallback("provider returned an error status");
        return candidates;
    }

    let response = match response.json::<RerankResponse>().await {
        Ok(response) => response,
        Err(_) => {
            warn_fallback("provider response was malformed");
            return candidates;
        }
    };

    let ranked = apply_response(&candidates, response, top_n);
    if ranked.is_none() {
        warn_fallback("provider response failed validation");
    }
    ranked.unwrap_or(candidates)
}

#[cfg(not(feature = "local-ml"))]
fn warn_fallback(reason: &str) {
    eprintln!("[rag] Lite reranking unavailable ({reason}); preserving RRF order");
}

#[cfg(any(test, not(feature = "local-ml")))]
fn candidate_documents(question: &str, candidates: &[RrfCandidate]) -> Vec<String> {
    let terms = super::retrieval::extract_query_terms(question);
    candidates
        .iter()
        .map(|candidate| {
            super::retrieval::snippet_window(
                &candidate.record.text_content,
                &terms,
                RERANK_DOCUMENT_MAX_CHARS,
            )
            .0
        })
        .collect()
}

#[cfg(any(test, not(feature = "local-ml")))]
fn apply_response(
    candidates: &[RrfCandidate],
    response: RerankResponse,
    top_n: usize,
) -> Option<Vec<RrfCandidate>> {
    let expected = top_n.min(candidates.len());
    if response.results.len() != expected {
        return None;
    }

    let mut seen = HashSet::with_capacity(expected);
    let mut ranked = Vec::with_capacity(expected);
    for result in response.results {
        if result.index >= candidates.len()
            || !result.relevance_score.is_finite()
            || !seen.insert(result.index)
        {
            return None;
        }
        let mut candidate = candidates[result.index].clone();
        candidate.score = result.relevance_score;
        ranked.push(candidate);
    }
    Some(ranked)
}

#[cfg(test)]
fn choose_ranking(
    candidates: Vec<RrfCandidate>,
    response: Option<RerankResponse>,
    top_n: usize,
) -> Vec<RrfCandidate> {
    response
        .and_then(|response| apply_response(&candidates, response, top_n))
        .unwrap_or(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::retrieval::SourceRecord;

    fn candidate(asset_id: &str, text: String, score: f64) -> RrfCandidate {
        RrfCandidate {
            record: SourceRecord {
                asset_id: asset_id.to_string(),
                item_id: format!("item-{asset_id}"),
                item_title: "Title".to_string(),
                collection_id: "collection".to_string(),
                collection_name: "Collection".to_string(),
                text_content: text,
                segments_json: None,
                transcription_offset_chars: None,
            },
            score,
        }
    }

    fn response(results: &[(usize, f64)]) -> RerankResponse {
        RerankResponse {
            results: results
                .iter()
                .map(|(index, relevance_score)| RerankResult {
                    index: *index,
                    relevance_score: *relevance_score,
                })
                .collect(),
        }
    }

    #[test]
    fn documents_are_unicode_bounded_and_windowed_near_query_terms() {
        let text = format!("{}objetivo{}", "ñ".repeat(7000), "🦉".repeat(7000));
        let candidates = vec![candidate("a", text, 0.1)];

        let documents = candidate_documents("buscar objetivo", &candidates);

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].chars().count(), RERANK_DOCUMENT_MAX_CHARS);
        assert!(documents[0].contains("objetivo"));
        assert!(std::str::from_utf8(documents[0].as_bytes()).is_ok());
    }

    #[test]
    fn valid_response_uses_provider_order_original_indices_and_scores() {
        let candidates = vec![
            candidate("rrf-first", "first".to_string(), 0.9),
            candidate("rrf-second", "second".to_string(), 0.8),
            candidate("rrf-third", "third".to_string(), 0.7),
        ];

        let ranked = choose_ranking(candidates, Some(response(&[(2, 0.97), (0, 0.42)])), 2);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].record.asset_id, "rrf-third");
        assert_eq!(ranked[0].score, 0.97);
        assert_eq!(ranked[1].record.asset_id, "rrf-first");
        assert_eq!(ranked[1].score, 0.42);
    }

    #[test]
    fn invalid_or_incomplete_response_falls_back_to_original_rrf_ranking() {
        let cases = [
            response(&[(1, 0.9)]),
            response(&[(1, 0.9), (1, 0.8)]),
            response(&[(1, 0.9), (99, 0.8)]),
            response(&[(1, 0.9), (0, f64::NAN)]),
        ];

        for invalid in cases {
            let candidates = vec![
                candidate("first", "first".to_string(), 0.6),
                candidate("second", "second".to_string(), 0.5),
                candidate("third", "third".to_string(), 0.4),
            ];
            let ranked = choose_ranking(candidates, Some(invalid), 2);
            let ids: Vec<&str> = ranked
                .iter()
                .map(|candidate| candidate.record.asset_id.as_str())
                .collect();

            assert_eq!(ids, vec!["first", "second", "third"]);
            assert_eq!(ranked[0].score, 0.6);
        }
    }

    #[test]
    fn missing_or_malformed_response_falls_back_to_original_rrf_ranking() {
        let candidates = vec![
            candidate("first", "first".to_string(), 0.6),
            candidate("second", "second".to_string(), 0.5),
        ];

        let ranked = choose_ranking(candidates, None, 1);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].record.asset_id, "first");
        assert!(serde_json::from_str::<RerankResponse>(r#"{"results":"bad"}"#).is_err());
    }
}
