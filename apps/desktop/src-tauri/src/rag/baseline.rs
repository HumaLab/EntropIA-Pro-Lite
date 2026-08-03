use std::collections::HashSet;

use serde::{Deserialize, Serialize};

const GOLDEN_MANIFEST_JSON: &str = include_str!("../../tests/fixtures/rag_golden/soip_1961.json");
const GOLDEN_SCHEMA_VERSION: u32 = 1;
const REQUIRED_CASES_MIN: usize = 30;
const REQUIRED_CASES_MAX: usize = 50;
#[cfg(not(feature = "local-ml"))]
const LITE_DB_ENV: &str = "ENTROPIA_SOIP_1961_DB";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenManifest {
    schema_version: u32,
    collection: GoldenCollection,
    expected_corpus: ExpectedCorpus,
    default_k: usize,
    cases: Vec<GoldenCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenCollection {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedCorpus {
    image_assets: usize,
    non_empty_ocr: usize,
    embeddings: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenCase {
    id: String,
    question: String,
    category: GoldenCategory,
    slices: Vec<GoldenSlice>,
    expected_asset_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GoldenCategory {
    LaborHistory,
    UnionActivity,
    PublicAffairs,
    FishingIndustry,
    CommunityAndServices,
    NoAnswer,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum GoldenSlice {
    NaturalLanguage,
    ProperName,
    DateOrArchiveReference,
    MultiAssetEvidence,
    NoAnswer,
}

impl GoldenManifest {
    fn parse(json: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(json)
            .map_err(|error| format!("Invalid RAG golden manifest JSON: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != GOLDEN_SCHEMA_VERSION {
            return Err(format!(
                "Unsupported RAG golden schema version {}",
                self.schema_version
            ));
        }
        if uuid::Uuid::parse_str(&self.collection.id).is_err()
            || self.collection.name.trim().is_empty()
        {
            return Err("Golden collection identity is invalid".to_string());
        }
        if self.expected_corpus.image_assets == 0
            || self.expected_corpus.non_empty_ocr != self.expected_corpus.image_assets
            || self.expected_corpus.embeddings != self.expected_corpus.image_assets
        {
            return Err("Golden corpus coverage must be complete and non-zero".to_string());
        }
        if self.default_k == 0 {
            return Err("Golden default_k must be greater than zero".to_string());
        }
        if !(REQUIRED_CASES_MIN..=REQUIRED_CASES_MAX).contains(&self.cases.len()) {
            return Err(format!(
                "Golden manifest must contain {REQUIRED_CASES_MIN}..={REQUIRED_CASES_MAX} cases"
            ));
        }

        let mut case_ids = HashSet::with_capacity(self.cases.len());
        let mut questions = HashSet::with_capacity(self.cases.len());
        let mut covered_slices = HashSet::new();
        for case in &self.cases {
            if case.id.trim().is_empty() || !case_ids.insert(case.id.as_str()) {
                return Err(format!(
                    "Golden case id is empty or duplicated: {}",
                    case.id
                ));
            }
            let normalized_question = case.question.trim().to_lowercase();
            if normalized_question.is_empty() || !questions.insert(normalized_question) {
                return Err(format!(
                    "Golden question is empty or duplicated: {}",
                    case.id
                ));
            }
            if case.slices.is_empty() || !case.slices.contains(&GoldenSlice::NaturalLanguage) {
                return Err(format!(
                    "Golden case {} must carry the natural_language slice",
                    case.id
                ));
            }
            let unique_slices: HashSet<_> = case.slices.iter().copied().collect();
            if unique_slices.len() != case.slices.len() {
                return Err(format!("Golden case {} repeats a slice", case.id));
            }
            covered_slices.extend(unique_slices);

            let is_no_answer = case.slices.contains(&GoldenSlice::NoAnswer);
            if is_no_answer != matches!(case.category, GoldenCategory::NoAnswer)
                || is_no_answer != case.expected_asset_ids.is_empty()
            {
                return Err(format!(
                    "Golden case {} has inconsistent no-answer metadata",
                    case.id
                ));
            }
            if case.slices.contains(&GoldenSlice::MultiAssetEvidence)
                != (case.expected_asset_ids.len() > 1)
            {
                return Err(format!(
                    "Golden case {} has inconsistent multi-asset metadata",
                    case.id
                ));
            }

            let mut expected_ids = HashSet::with_capacity(case.expected_asset_ids.len());
            for asset_id in &case.expected_asset_ids {
                if uuid::Uuid::parse_str(asset_id).is_err()
                    || !expected_ids.insert(asset_id.as_str())
                {
                    return Err(format!(
                        "Golden case {} has an invalid or duplicated asset id",
                        case.id
                    ));
                }
            }
        }

        for required in [
            GoldenSlice::NaturalLanguage,
            GoldenSlice::ProperName,
            GoldenSlice::DateOrArchiveReference,
            GoldenSlice::MultiAssetEvidence,
            GoldenSlice::NoAnswer,
        ] {
            if !covered_slices.contains(&required) {
                return Err(format!("Golden manifest does not cover slice {required:?}"));
            }
        }
        Ok(())
    }
}

fn recall_at_k(ranking: &[String], relevant: &HashSet<&str>, k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let hits = ranking
        .iter()
        .take(k)
        .filter(|asset_id| relevant.contains(asset_id.as_str()))
        .count();
    hits as f64 / relevant.len() as f64
}

fn reciprocal_rank(ranking: &[String], relevant: &HashSet<&str>) -> f64 {
    ranking
        .iter()
        .position(|asset_id| relevant.contains(asset_id.as_str()))
        .map(|rank0| 1.0 / (rank0 + 1) as f64)
        .unwrap_or(0.0)
}

fn ndcg_at_k(ranking: &[String], relevant: &HashSet<&str>, k: usize) -> f64 {
    if relevant.is_empty() || k == 0 {
        return 0.0;
    }
    let dcg: f64 = ranking
        .iter()
        .take(k)
        .enumerate()
        .filter(|(_, asset_id)| relevant.contains(asset_id.as_str()))
        .map(|(rank0, _)| 1.0 / ((rank0 + 2) as f64).log2())
        .sum();
    let ideal_hits = relevant.len().min(k);
    let ideal_dcg: f64 = (0..ideal_hits)
        .map(|rank0| 1.0 / ((rank0 + 2) as f64).log2())
        .sum();
    dcg / ideal_dcg
}

#[cfg(not(feature = "local-ml"))]
#[derive(Debug, Clone, Copy, Serialize, Default)]
struct RankingMetrics {
    recall_at_k: f64,
    reciprocal_rank: f64,
    ndcg_at_k: f64,
}

#[cfg(not(feature = "local-ml"))]
impl RankingMetrics {
    fn calculate(ranking: &[String], expected_asset_ids: &[String], k: usize) -> Self {
        let relevant: HashSet<_> = expected_asset_ids.iter().map(String::as_str).collect();
        Self {
            recall_at_k: recall_at_k(ranking, &relevant, k),
            reciprocal_rank: reciprocal_rank(ranking, &relevant),
            ndcg_at_k: ndcg_at_k(ranking, &relevant, k),
        }
    }

    fn add(&mut self, other: Self) {
        self.recall_at_k += other.recall_at_k;
        self.reciprocal_rank += other.reciprocal_rank;
        self.ndcg_at_k += other.ndcg_at_k;
    }

    fn divide(self, denominator: usize) -> Self {
        let denominator = denominator as f64;
        Self {
            recall_at_k: self.recall_at_k / denominator,
            reciprocal_rank: self.reciprocal_rank / denominator,
            ndcg_at_k: self.ndcg_at_k / denominator,
        }
    }
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct QuerySnapshot {
    case_id: String,
    category: GoldenCategory,
    slices: Vec<GoldenSlice>,
    expected_asset_ids: Vec<String>,
    rankings: RankingSnapshot,
    metrics: Option<MetricSet>,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct RankingSnapshot {
    vector: Vec<String>,
    lexical: Vec<String>,
    fused: Vec<String>,
    reranked: Vec<String>,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Clone, Copy, Serialize)]
struct MetricSet {
    vector: RankingMetrics,
    lexical: RankingMetrics,
    fused: RankingMetrics,
    reranked: RankingMetrics,
}

#[cfg(not(feature = "local-ml"))]
impl MetricSet {
    fn calculate(rankings: &RankingSnapshot, expected_asset_ids: &[String], k: usize) -> Self {
        Self {
            vector: RankingMetrics::calculate(&rankings.vector, expected_asset_ids, k),
            lexical: RankingMetrics::calculate(&rankings.lexical, expected_asset_ids, k),
            fused: RankingMetrics::calculate(&rankings.fused, expected_asset_ids, k),
            reranked: RankingMetrics::calculate(&rankings.reranked, expected_asset_ids, k),
        }
    }
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct AggregateSnapshot {
    evaluated_answerable_cases: usize,
    no_answer_cases: usize,
    vector: RankingMetrics,
    lexical: RankingMetrics,
    fused: RankingMetrics,
    reranked: RankingMetrics,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct BaselineSnapshot {
    schema_version: u32,
    generated_at_unix_seconds: u64,
    collection: GoldenCollectionSnapshot,
    corpus: CorpusSnapshot,
    k: usize,
    cases: Vec<QuerySnapshot>,
    aggregate: AggregateSnapshot,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct GoldenCollectionSnapshot {
    id: String,
    name: String,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct CorpusSnapshot {
    image_assets: usize,
    non_empty_ocr: usize,
    embeddings: usize,
}

#[cfg(not(feature = "local-ml"))]
fn open_verified_corpus(manifest: &GoldenManifest) -> Result<rusqlite::Connection, String> {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};

    let path = std::env::var_os(LITE_DB_ENV)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Set {LITE_DB_ENV} to the Lite database path"))?;
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Failed to open the Lite database read-only: {error}"))?;
    conn.pragma_update(None, "query_only", "ON")
        .map_err(|error| format!("Failed to enforce SQLite query-only mode: {error}"))?;

    let collection_name = conn
        .query_row(
            "SELECT name FROM collections WHERE id = ?1",
            [&manifest.collection.id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Failed to verify golden collection identity: {error}"))?;
    if collection_name.as_deref() != Some(manifest.collection.name.as_str()) {
        return Err("Golden collection identity does not match the Lite database".to_string());
    }

    let (image_assets, non_empty_ocr, embeddings): (i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(EXISTS(
                        SELECT 1 FROM extractions e
                        WHERE e.asset_id = a.id
                          AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0
                    )), 0),
                    COALESCE(SUM(EXISTS(
                        SELECT 1 FROM vec_assets v
                        WHERE v.asset_id = a.id
                          AND LENGTH(v.embedding) > 0
                    )), 0)
             FROM assets a
             JOIN items i ON i.id = a.item_id
             WHERE i.collection_id = ?1 AND a.type = 'image'",
            [&manifest.collection.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| format!("Failed to verify golden corpus coverage: {error}"))?;
    let observed = (
        usize::try_from(image_assets).unwrap_or_default(),
        usize::try_from(non_empty_ocr).unwrap_or_default(),
        usize::try_from(embeddings).unwrap_or_default(),
    );
    let expected = (
        manifest.expected_corpus.image_assets,
        manifest.expected_corpus.non_empty_ocr,
        manifest.expected_corpus.embeddings,
    );
    if observed != expected {
        return Err(format!(
            "Golden corpus coverage mismatch: expected {expected:?}, observed {observed:?}"
        ));
    }

    for case in &manifest.cases {
        for asset_id in &case.expected_asset_ids {
            let valid: bool = conn
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1
                        FROM assets a
                        JOIN items i ON i.id = a.item_id
                        WHERE a.id = ?1
                          AND i.collection_id = ?2
                          AND a.type = 'image'
                          AND EXISTS(
                              SELECT 1 FROM extractions e
                              WHERE e.asset_id = a.id
                                AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0
                          )
                          AND EXISTS(
                              SELECT 1 FROM vec_assets v
                              WHERE v.asset_id = a.id
                                AND LENGTH(v.embedding) > 0
                          )
                    )",
                    rusqlite::params![asset_id, manifest.collection.id],
                    |row| row.get(0),
                )
                .map_err(|error| format!("Failed to verify golden labels: {error}"))?;
            if !valid {
                return Err(format!(
                    "Golden case {} references an unavailable corpus asset",
                    case.id
                ));
            }
        }
    }

    Ok(conn)
}

#[cfg(not(feature = "local-ml"))]
fn top_k(mut ranking: Vec<String>, k: usize) -> Vec<String> {
    ranking.truncate(k);
    ranking
}

#[cfg(not(feature = "local-ml"))]
fn aggregate_snapshots(cases: &[QuerySnapshot]) -> AggregateSnapshot {
    let mut aggregate = AggregateSnapshot {
        evaluated_answerable_cases: 0,
        no_answer_cases: 0,
        vector: RankingMetrics::default(),
        lexical: RankingMetrics::default(),
        fused: RankingMetrics::default(),
        reranked: RankingMetrics::default(),
    };
    for case in cases {
        match case.metrics {
            Some(metrics) => {
                aggregate.evaluated_answerable_cases += 1;
                aggregate.vector.add(metrics.vector);
                aggregate.lexical.add(metrics.lexical);
                aggregate.fused.add(metrics.fused);
                aggregate.reranked.add(metrics.reranked);
            }
            None => aggregate.no_answer_cases += 1,
        }
    }
    let count = aggregate.evaluated_answerable_cases;
    aggregate.vector = aggregate.vector.divide(count);
    aggregate.lexical = aggregate.lexical.divide(count);
    aggregate.fused = aggregate.fused.divide(count);
    aggregate.reranked = aggregate.reranked.divide(count);
    aggregate
}

#[cfg(not(feature = "local-ml"))]
fn print_metrics(cases: &[QuerySnapshot], aggregate: &AggregateSnapshot, k: usize) {
    println!("case       vector R/M/N      lexical R/M/N     fused R/M/N       reranked R/M/N");
    for case in cases {
        if let Some(metrics) = case.metrics {
            println!(
                "{:<10} {:>4.2}/{:>4.2}/{:>4.2}  {:>4.2}/{:>4.2}/{:>4.2}  {:>4.2}/{:>4.2}/{:>4.2}  {:>4.2}/{:>4.2}/{:>4.2}",
                case.case_id,
                metrics.vector.recall_at_k,
                metrics.vector.reciprocal_rank,
                metrics.vector.ndcg_at_k,
                metrics.lexical.recall_at_k,
                metrics.lexical.reciprocal_rank,
                metrics.lexical.ndcg_at_k,
                metrics.fused.recall_at_k,
                metrics.fused.reciprocal_rank,
                metrics.fused.ndcg_at_k,
                metrics.reranked.recall_at_k,
                metrics.reranked.reciprocal_rank,
                metrics.reranked.ndcg_at_k,
            );
        } else {
            println!(
                "{:<10} no-answer case (excluded from relevance metrics)",
                case.case_id
            );
        }
    }
    println!(
        "mean@{k}     {:>4.2}/{:>4.2}/{:>4.2}  {:>4.2}/{:>4.2}/{:>4.2}  {:>4.2}/{:>4.2}/{:>4.2}  {:>4.2}/{:>4.2}/{:>4.2}",
        aggregate.vector.recall_at_k,
        aggregate.vector.reciprocal_rank,
        aggregate.vector.ndcg_at_k,
        aggregate.lexical.recall_at_k,
        aggregate.lexical.reciprocal_rank,
        aggregate.lexical.ndcg_at_k,
        aggregate.fused.recall_at_k,
        aggregate.fused.reciprocal_rank,
        aggregate.fused.ndcg_at_k,
        aggregate.reranked.recall_at_k,
        aggregate.reranked.reciprocal_rank,
        aggregate.reranked.ndcg_at_k,
    );
}

#[cfg(not(feature = "local-ml"))]
fn cargo_target_dir() -> Result<std::path::PathBuf, String> {
    std::path::Path::new(env!("OUT_DIR"))
        .ancestors()
        .nth(4)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "Could not resolve Cargo target directory from OUT_DIR".to_string())
}

#[cfg(not(feature = "local-ml"))]
#[tokio::test]
#[ignore = "contacts OpenRouter and requires a private read-only Lite database"]
async fn rag_baseline() -> Result<(), String> {
    let manifest = GoldenManifest::parse(GOLDEN_MANIFEST_JSON)?;

    // This gate intentionally precedes settings resolution, engine initialization,
    // query embedding, and reranking. A stale or wrong corpus cannot spend money.
    let conn = open_verified_corpus(&manifest)?;
    let params = super::params::rag_params_from_settings(&conn);
    if params.top_k != manifest.default_k {
        return Err(format!(
            "Golden default_k {} does not match the production rag_top_k {}",
            manifest.default_k, params.top_k
        ));
    }

    let embedding_config = crate::nlp::embeddings::config_from_settings(&conn)?;
    let api_key = embedding_config.api_key.clone();
    let embedding_engine = crate::nlp::embeddings::get_or_init_engine(embedding_config)?;
    let reranker_model = super::reranker::resolve_reranker_model(&conn);
    let k = manifest.default_k;
    let mut snapshots = Vec::with_capacity(manifest.cases.len());

    for case in &manifest.cases {
        let embedding_engine = std::sync::Arc::clone(&embedding_engine);
        let question = case.question.clone();
        let query_embedding =
            tokio::task::spawn_blocking(move || embedding_engine.embed_text(&question))
                .await
                .map_err(|error| {
                    format!("RAG baseline query embedding worker failed: {error}")
                })??;
        let vector = super::retrieval::vector_leg(
            &conn,
            &query_embedding,
            params.candidates_per_leg,
            params.min_similarity,
        )?;
        let lexical =
            super::retrieval::lexical_leg(&conn, &case.question, params.candidates_per_leg)?;
        let fused = super::retrieval::rrf_fuse(
            &[vector.clone(), lexical.clone()],
            params.fusion_candidate_limit,
            params.rrf_k as f64,
        );
        let candidates = super::retrieval::hybrid_retrieve_candidates(
            &conn,
            &case.question,
            Some(&query_embedding),
            &params,
        )?;
        let reranked = super::reranker::rerank_candidates(
            &case.question,
            candidates,
            &api_key,
            &reranker_model,
            params.rerank_depth,
        )
        .await;

        let rankings = RankingSnapshot {
            vector: top_k(vector, k),
            lexical: top_k(lexical, k),
            fused: top_k(fused.into_iter().map(|(asset_id, _)| asset_id).collect(), k),
            reranked: top_k(
                reranked
                    .into_iter()
                    .map(|candidate| candidate.record.asset_id)
                    .collect(),
                k,
            ),
        };
        let metrics = (!case.expected_asset_ids.is_empty())
            .then(|| MetricSet::calculate(&rankings, &case.expected_asset_ids, k));
        snapshots.push(QuerySnapshot {
            case_id: case.id.clone(),
            category: case.category,
            slices: case.slices.clone(),
            expected_asset_ids: case.expected_asset_ids.clone(),
            rankings,
            metrics,
        });
    }

    let aggregate = aggregate_snapshots(&snapshots);
    print_metrics(&snapshots, &aggregate, k);
    let snapshot = BaselineSnapshot {
        schema_version: GOLDEN_SCHEMA_VERSION,
        generated_at_unix_seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("System clock is before the Unix epoch: {error}"))?
            .as_secs(),
        collection: GoldenCollectionSnapshot {
            id: manifest.collection.id,
            name: manifest.collection.name,
        },
        corpus: CorpusSnapshot {
            image_assets: manifest.expected_corpus.image_assets,
            non_empty_ocr: manifest.expected_corpus.non_empty_ocr,
            embeddings: manifest.expected_corpus.embeddings,
        },
        k,
        cases: snapshots,
        aggregate,
    };
    let output_dir = cargo_target_dir()?.join("rag-baseline");
    std::fs::create_dir_all(&output_dir)
        .map_err(|error| format!("Failed to create baseline snapshot directory: {error}"))?;
    let output_path = output_dir.join("soip-1961.json");
    let json = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| format!("Failed to serialize baseline snapshot: {error}"))?;
    std::fs::write(&output_path, json)
        .map_err(|error| format!("Failed to write baseline snapshot: {error}"))?;
    println!("snapshot={}", output_path.display());
    Ok(())
}

#[test]
fn golden_manifest_loads_and_covers_required_slices() {
    let manifest = GoldenManifest::parse(GOLDEN_MANIFEST_JSON).expect("golden manifest is valid");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.collection.name, "SOIP 1961");
    assert_eq!(manifest.expected_corpus.image_assets, 56);
    assert_eq!(manifest.cases.len(), 40);
}

#[test]
fn malformed_manifest_is_rejected() {
    let malformed =
        GOLDEN_MANIFEST_JSON.replacen("\"schema_version\": 1", "\"schema_version\": 2", 1);
    assert!(GoldenManifest::parse(&malformed).is_err());

    let inconsistent_no_answer = GOLDEN_MANIFEST_JSON.replacen(
        "\"expected_asset_ids\": []",
        "\"expected_asset_ids\": [\"not-a-uuid\"]",
        1,
    );
    assert!(GoldenManifest::parse(&inconsistent_no_answer).is_err());
}

#[test]
fn recall_at_k_matches_worked_example() {
    let ranking = ["x", "a", "b", "c"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let relevant = HashSet::from(["a", "b", "z"]);
    assert!((recall_at_k(&ranking, &relevant, 3) - (2.0 / 3.0)).abs() < 1e-12);
}

#[test]
fn reciprocal_rank_matches_worked_example() {
    let ranking = ["x", "a", "b"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let relevant = HashSet::from(["a", "b"]);
    assert!((reciprocal_rank(&ranking, &relevant) - 0.5).abs() < 1e-12);
}

#[test]
fn binary_ndcg_at_k_matches_worked_example() {
    let ranking = ["x", "a", "b"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let relevant = HashSet::from(["a", "b", "z"]);
    let expected = 1.130_929_753_571_457_5 / 2.130_929_753_571_457_8;
    assert!((ndcg_at_k(&ranking, &relevant, 3) - expected).abs() < 1e-12);
}
