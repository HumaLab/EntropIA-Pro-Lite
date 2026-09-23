//! Evaluation harness (E7c): Recall@k, nDCG@k, and MRR over judged runs.
//!
//! Pure math, no retrieval, no network. Human judgments arrive separately
//! (blocked on the isolated personal library per E0); the harness and the
//! `zsb-eval-v1` seed skeleton are verifiable today with synthetic
//! judgments. Empty relevant sets skip the query; empty retrieved lists
//! score zero — never NaN, never silent.

use std::collections::{HashMap, HashSet};

/// One judged query: graded relevance per work (`grade > 0` counts as
/// relevant). Ids are opaque; the seed carries no private content.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgedQuery {
    pub query_id: String,
    pub query_text: String,
    pub relevance: Vec<(String, u32)>,
}

/// One system run for one query: ranked work ids, best first.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalRun {
    pub query_id: String,
    pub ranked_item_ids: Vec<String>,
}

/// Per-query scores plus the corpus means.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalMetrics {
    pub per_query: Vec<QueryScores>,
    pub mean_recall_at_k: f64,
    pub mean_ndcg_at_k: f64,
    pub mean_reciprocal_rank: f64,
    pub queries_scored: usize,
    pub queries_skipped: usize,
}

/// Scores for one query at cutoff k.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryScores {
    pub query_id: String,
    pub recall_at_k: f64,
    pub ndcg_at_k: f64,
    pub reciprocal_rank: f64,
}

/// Scores one run against graded judgments at cutoff `k`.
pub fn evaluate_run(judgments: &[JudgedQuery], runs: &[EvalRun], k: usize) -> EvalMetrics {
    let k = k.max(1);
    let by_query: HashMap<&str, &[String]> = runs
        .iter()
        .map(|run| (run.query_id.as_str(), run.ranked_item_ids.as_slice()))
        .collect();
    let mut per_query = Vec::new();
    let mut skipped = 0usize;
    for judgment in judgments {
        let relevant: HashMap<&str, u32> = judgment
            .relevance
            .iter()
            .filter(|(_, grade)| *grade > 0)
            .map(|(item_id, grade)| (item_id.as_str(), *grade))
            .collect();
        if relevant.is_empty() {
            skipped += 1;
            continue;
        }
        let retrieved: &[String] = by_query
            .get(judgment.query_id.as_str())
            .copied()
            .unwrap_or(&[]);
        let top: Vec<&str> = retrieved.iter().take(k).map(String::as_str).collect();
        let mut retrieved_relevant = HashSet::new();
        let mut dcg = 0.0;
        let mut reciprocal_rank = 0.0;
        for (rank, item_id) in top.iter().enumerate() {
            let grade = relevant.get(item_id).copied().unwrap_or(0) as f64;
            if grade > 0.0 {
                retrieved_relevant.insert(*item_id);
                if reciprocal_rank == 0.0 {
                    reciprocal_rank = 1.0 / (rank as f64 + 1.0);
                }
            }
            dcg += grade / ((rank as f64 + 2.0).log2());
        }
        let mut grades: Vec<u32> = relevant.values().copied().collect();
        grades.sort_unstable_by(|a, b| b.cmp(a));
        let idcg: f64 = grades
            .into_iter()
            .take(k)
            .enumerate()
            .map(|(rank, grade)| grade as f64 / ((rank as f64 + 2.0).log2()))
            .sum();
        per_query.push(QueryScores {
            query_id: judgment.query_id.clone(),
            recall_at_k: retrieved_relevant.len() as f64 / relevant.len() as f64,
            ndcg_at_k: if idcg > 0.0 { dcg / idcg } else { 0.0 },
            reciprocal_rank,
        });
    }
    let scored = per_query.len();
    let mean = |pick: fn(&QueryScores) -> f64| {
        if scored == 0 {
            0.0
        } else {
            per_query.iter().map(pick).sum::<f64>() / scored as f64
        }
    };
    EvalMetrics {
        mean_recall_at_k: mean(|q| q.recall_at_k),
        mean_ndcg_at_k: mean(|q| q.ndcg_at_k),
        mean_reciprocal_rank: mean(|q| q.reciprocal_rank),
        per_query,
        queries_scored: scored,
        queries_skipped: skipped,
    }
}

/// Loads one `zsb-eval-v1` seed file into judged queries. The file must
/// carry the seed name, opaque ids, and graded relevance; anything else
/// fails closed instead of scoring against malformed judgments.
pub fn load_eval_seed(json: &str) -> Result<Vec<JudgedQuery>, String> {
    let seed: serde_json::Value =
        serde_json::from_str(json).map_err(|error| format!("invalid eval seed: {error}"))?;
    if seed.get("seed").and_then(|value| value.as_str()) != Some("zsb-eval-v1") {
        return Err("invalid eval seed: expected seed zsb-eval-v1".to_string());
    }
    let queries = seed
        .get("queries")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "invalid eval seed: missing queries array".to_string())?;
    let mut judged = Vec::with_capacity(queries.len());
    for query in queries {
        let query_id = query
            .get("query_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "invalid eval seed: query without query_id".to_string())?;
        let query_text = query
            .get("query_text")
            .and_then(|value| value.as_str())
            .ok_or_else(|| format!("invalid eval seed: {query_id} without query_text"))?;
        let relevance = query
            .get("relevance")
            .and_then(|value| value.as_array())
            .ok_or_else(|| format!("invalid eval seed: {query_id} without relevance"))?;
        let mut graded = Vec::with_capacity(relevance.len());
        for entry in relevance {
            let item_id = entry
                .get("item_id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    format!("invalid eval seed: {query_id} relevance without item_id")
                })?;
            let grade = entry
                .get("grade")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| format!("invalid eval seed: {query_id} relevance without grade"))?;
            graded.push((item_id.to_string(), grade as u32));
        }
        judged.push(JudgedQuery {
            query_id: query_id.to_string(),
            query_text: query_text.to_string(),
            relevance: graded,
        });
    }
    Ok(judged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judged() -> Vec<JudgedQuery> {
        vec![
            JudgedQuery {
                query_id: "q1".to_string(),
                query_text: "revoluciones".to_string(),
                relevance: vec![("A".to_string(), 3), ("B".to_string(), 1)],
            },
            JudgedQuery {
                query_id: "q2".to_string(),
                query_text: "helechos".to_string(),
                relevance: vec![("C".to_string(), 2)],
            },
        ]
    }

    #[test]
    fn perfect_run_scores_one_everywhere() {
        let runs = vec![
            EvalRun {
                query_id: "q1".to_string(),
                ranked_item_ids: vec!["A".to_string(), "B".to_string()],
            },
            EvalRun {
                query_id: "q2".to_string(),
                ranked_item_ids: vec!["C".to_string()],
            },
        ];
        let metrics = evaluate_run(&judged(), &runs, 5);
        assert_eq!(metrics.queries_scored, 2);
        assert_eq!(metrics.queries_skipped, 0);
        assert!((metrics.mean_recall_at_k - 1.0).abs() < 1e-12);
        assert!((metrics.mean_ndcg_at_k - 1.0).abs() < 1e-12);
        assert!((metrics.mean_reciprocal_rank - 1.0).abs() < 1e-12);
    }

    #[test]
    fn partial_run_matches_hand_computed_values() {
        // q1 relevant {A:3, B:1}, retrieved [B, A, C], k=3:
        // DCG = 1/log2(2) + 3/log2(3); IDCG = 3/log2(2) + 1/log2(3).
        let runs = vec![EvalRun {
            query_id: "q1".to_string(),
            ranked_item_ids: vec!["B".to_string(), "A".to_string(), "C".to_string()],
        }];
        let metrics = evaluate_run(&judged(), &runs, 3);
        assert_eq!(
            metrics.queries_scored, 2,
            "the run-less query scores zero, not missing"
        );
        assert_eq!(metrics.per_query[1].query_id, "q2");
        assert_eq!(
            (
                metrics.per_query[1].recall_at_k,
                metrics.per_query[1].reciprocal_rank
            ),
            (0.0, 0.0)
        );
        let q = &metrics.per_query[0];
        assert!(
            (q.recall_at_k - 1.0).abs() < 1e-12,
            "both relevant retrieved"
        );
        let expected_ndcg = (1.0 + 3.0 / 3.0_f64.log2()) / (3.0 + 1.0 / 3.0_f64.log2());
        assert!(
            (q.ndcg_at_k - expected_ndcg).abs() < 1e-9,
            "graded nDCG, got {}",
            q.ndcg_at_k
        );
        assert!(
            (q.reciprocal_rank - 1.0).abs() < 1e-12,
            "B is relevant at rank 1"
        );
    }

    #[test]
    fn empty_sides_behave_honestly() {
        let judgments = vec![
            JudgedQuery {
                query_id: "q-empty".to_string(),
                query_text: "x".to_string(),
                relevance: vec![],
            },
            JudgedQuery {
                query_id: "q1".to_string(),
                query_text: "y".to_string(),
                relevance: vec![("A".to_string(), 1)],
            },
        ];
        let runs = vec![
            EvalRun {
                query_id: "q-empty".to_string(),
                ranked_item_ids: vec!["A".to_string()],
            },
            EvalRun {
                query_id: "q1".to_string(),
                ranked_item_ids: vec![],
            },
        ];
        let metrics = evaluate_run(&judgments, &runs, 5);
        assert_eq!(
            metrics.queries_skipped, 1,
            "no relevant set means nothing to score"
        );
        assert_eq!(metrics.queries_scored, 1);
        let q = &metrics.per_query[0];
        assert_eq!(q.query_id, "q1");
        assert_eq!(
            (q.recall_at_k, q.ndcg_at_k, q.reciprocal_rank),
            (0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn seed_file_loads_and_validates() {
        let json = include_str!("../../tests/fixtures/zsb-eval-v1.json");
        let judged = load_eval_seed(json).expect("seed loads");
        assert_eq!(judged.len(), 2);
        assert_eq!(judged[0].query_id, "zsb-q1");
        assert!(judged[0]
            .relevance
            .iter()
            .any(|(id, grade)| id == "work-alpha" && *grade == 3));
        // Malformed seeds fail closed.
        assert!(load_eval_seed("{}").is_err());
        assert!(load_eval_seed(r#"{"seed":"other"}"#).is_err());
        assert!(load_eval_seed(r#"{"seed":"zsb-eval-v1"}"#).is_err());
    }

    #[test]
    fn missing_runs_score_zero_not_missing() {
        let metrics = evaluate_run(&judged(), &[], 5);
        assert_eq!(metrics.queries_scored, 2);
        assert!(metrics
            .per_query
            .iter()
            .all(|q| q.recall_at_k == 0.0 && q.ndcg_at_k == 0.0 && q.reciprocal_rank == 0.0));
    }
}
