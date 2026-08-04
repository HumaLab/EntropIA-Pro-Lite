use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

const GOLDEN_MANIFEST_JSON: &str = include_str!("../../tests/fixtures/rag_golden/soip_1961.json");
const GOLDEN_SCHEMA_VERSION: u32 = 1;
const REQUIRED_CASES_MIN: usize = 30;
const REQUIRED_CASES_MAX: usize = 50;
#[cfg(not(feature = "local-ml"))]
const LITE_DB_ENV: &str = "ENTROPIA_SOIP_1961_DB";
#[cfg(not(feature = "local-ml"))]
const RAG_CHUNKS_MIGRATION_SQL: &str =
    include_str!("../../../../../packages/store/src/migrations/0029_rag_chunks.sql");

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
    challenge_type: GoldenChallenge,
    evidence_cues: Vec<String>,
    slices: Vec<GoldenSlice>,
    expected_asset_ids: Vec<String>,
    #[serde(default)]
    conversation: Option<GoldenConversation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenConversation {
    source_case_id: String,
    history: Vec<GoldenHistoryTurn>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenHistoryTurn {
    role: GoldenHistoryRole,
    content: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GoldenHistoryRole {
    User,
    Assistant,
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
enum GoldenChallenge {
    Paraphrase,
    LexicalMismatch,
    Ambiguity,
    MultiEvidence,
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
        let mut challenge_counts = HashMap::<GoldenChallenge, usize>::new();
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
            if case.conversation.is_none() {
                *challenge_counts.entry(case.challenge_type).or_default() += 1;
            }
            if case.evidence_cues.len() > 2
                || case.evidence_cues.iter().any(|cue| {
                    cue.trim().is_empty()
                        || cue.chars().count() > 32
                        || cue.contains('\r')
                        || cue.contains('\n')
                })
            {
                return Err(format!(
                    "Golden case {} has invalid bounded evidence cues",
                    case.id
                ));
            }

            let is_no_answer = case.slices.contains(&GoldenSlice::NoAnswer);
            if is_no_answer != matches!(case.category, GoldenCategory::NoAnswer)
                || is_no_answer != case.expected_asset_ids.is_empty()
            {
                return Err(format!(
                    "Golden case {} has inconsistent no-answer metadata",
                    case.id
                ));
            }
            if is_no_answer != matches!(case.challenge_type, GoldenChallenge::NoAnswer)
                || is_no_answer != case.evidence_cues.is_empty()
            {
                return Err(format!(
                    "Golden case {} has inconsistent challenge grounding",
                    case.id
                ));
            }
            if matches!(case.challenge_type, GoldenChallenge::MultiEvidence)
                != (case.expected_asset_ids.len() > 1)
            {
                return Err(format!(
                    "Golden case {} has inconsistent multi-evidence challenge",
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

        let standalone_cases = self
            .cases
            .iter()
            .filter(|case| case.conversation.is_none())
            .map(|case| (case.id.as_str(), case))
            .collect::<HashMap<_, _>>();
        let standalone_questions = standalone_cases
            .values()
            .map(|case| case.question.as_str())
            .collect::<HashSet<_>>();
        let conversational_cases = self
            .cases
            .iter()
            .filter(|case| case.conversation.is_some())
            .collect::<Vec<_>>();
        if standalone_cases.len() != 40 || !(1..=10).contains(&conversational_cases.len()) {
            return Err(
                "Golden manifest must retain 40 standalone cases and add 1..=10 conversational cases"
                    .to_string(),
            );
        }

        let mut source_case_ids = HashSet::with_capacity(conversational_cases.len());
        for case in conversational_cases {
            let conversation = case
                .conversation
                .as_ref()
                .expect("filtered conversational case must carry metadata");
            let source = standalone_cases
                .get(conversation.source_case_id.as_str())
                .ok_or_else(|| {
                    format!(
                        "Golden conversational case {} references a missing standalone source",
                        case.id
                    )
                })?;
            if !source_case_ids.insert(conversation.source_case_id.as_str()) {
                return Err(format!(
                    "Golden conversational case {} repeats a source case",
                    case.id
                ));
            }
            if case.category != source.category
                || case.challenge_type != source.challenge_type
                || case.evidence_cues != source.evidence_cues
                || case.slices != source.slices
                || case.expected_asset_ids != source.expected_asset_ids
            {
                return Err(format!(
                    "Golden conversational case {} changed its source grounding",
                    case.id
                ));
            }
            if conversation.history.is_empty()
                || conversation.history.len() > super::params::DEFAULT_HISTORY_TURNS
                || conversation.history.iter().any(|turn| {
                    turn.content.trim().is_empty()
                        || turn.content.chars().count()
                            > super::params::DEFAULT_HISTORY_TURN_MAX_CHARS
                        || !standalone_questions.contains(turn.content.as_str())
                })
                || !conversation
                    .history
                    .iter()
                    .any(|turn| turn.content == source.question)
            {
                return Err(format!(
                    "Golden conversational case {} has ungrounded or unbounded history",
                    case.id
                ));
            }
            let history = conversation
                .history
                .iter()
                .map(|turn| super::RagChatTurn {
                    role: match turn.role {
                        GoldenHistoryRole::User => "user",
                        GoldenHistoryRole::Assistant => "assistant",
                    }
                    .to_string(),
                    content: turn.content.clone(),
                })
                .collect::<Vec<_>>();
            if super::query_rewrite::build_rewrite_request(
                &case.question,
                &history,
                super::params::DEFAULT_HISTORY_TURNS,
                super::params::DEFAULT_HISTORY_TURN_MAX_CHARS,
            )
            .is_none()
            {
                return Err(format!(
                    "Golden conversational case {} is not a conditional follow-up",
                    case.id
                ));
            }
        }

        for challenge in [
            GoldenChallenge::Paraphrase,
            GoldenChallenge::LexicalMismatch,
            GoldenChallenge::Ambiguity,
            GoldenChallenge::MultiEvidence,
            GoldenChallenge::NoAnswer,
        ] {
            let count = challenge_counts
                .get(&challenge)
                .copied()
                .unwrap_or_default();
            if !(7..=9).contains(&count) {
                return Err(format!(
                    "Golden challenge {challenge:?} must contain 7..=9 cases, found {count}"
                ));
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

    fn delta(self, baseline: Self) -> Self {
        Self {
            recall_at_k: self.recall_at_k - baseline.recall_at_k,
            reciprocal_rank: self.reciprocal_rank - baseline.reciprocal_rank,
            ndcg_at_k: self.ndcg_at_k - baseline.ndcg_at_k,
        }
    }
}

#[cfg(not(feature = "local-ml"))]
#[derive(Clone, Serialize)]
struct QuerySnapshot {
    case_id: String,
    category: GoldenCategory,
    slices: Vec<GoldenSlice>,
    challenge_type: GoldenChallenge,
    evidence_cues: Vec<String>,
    expected_asset_ids: Vec<String>,
    rankings: RankingSnapshot,
    metrics: Option<MetricSet>,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Clone, Serialize)]
struct RankingSnapshot {
    vector: Vec<String>,
    lexical: Vec<String>,
    fused: Vec<String>,
    reranked: Vec<String>,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Clone, Copy, Serialize, Default)]
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

    fn add(&mut self, other: Self) {
        self.vector.add(other.vector);
        self.lexical.add(other.lexical);
        self.fused.add(other.fused);
        self.reranked.add(other.reranked);
    }

    fn divide(self, denominator: usize) -> Self {
        Self {
            vector: self.vector.divide(denominator),
            lexical: self.lexical.divide(denominator),
            fused: self.fused.divide(denominator),
            reranked: self.reranked.divide(denominator),
        }
    }

    fn delta(self, baseline: Self) -> Self {
        Self {
            vector: self.vector.delta(baseline.vector),
            lexical: self.lexical.delta(baseline.lexical),
            fused: self.fused.delta(baseline.fused),
            reranked: self.reranked.delta(baseline.reranked),
        }
    }
}

#[cfg(not(feature = "local-ml"))]
#[derive(Clone, Serialize)]
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
    retrieval_units: RetrievalUnitSnapshots,
    paired: PairedSnapshot,
    conversational: ConversationalSnapshot,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct RetrievalUnitSnapshots {
    asset: UnitSnapshot,
    chunk: UnitSnapshot,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct UnitSnapshot {
    cases: Vec<QuerySnapshot>,
    aggregate: AggregateSnapshot,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct ConversationalSnapshot {
    cases: Vec<ConversationalCaseSnapshot>,
    aggregate: ConversationalAggregateSnapshot,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct ConversationalCaseSnapshot {
    case_id: String,
    category: GoldenCategory,
    slices: Vec<GoldenSlice>,
    challenge_type: GoldenChallenge,
    evidence_cues: Vec<String>,
    expected_asset_ids: Vec<String>,
    rewrite: RewriteSnapshot,
    retrieval_units: ConversationalRetrievalUnitSnapshots,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct RewriteSnapshot {
    status: super::query_rewrite::RewriteStatus,
    elapsed_ms: u128,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct ConversationalRetrievalUnitSnapshots {
    asset: ConversationalUnitSnapshot,
    chunk: ConversationalUnitSnapshot,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct ConversationalUnitSnapshot {
    original_only: ConversationalStageSnapshot,
    original_plus_rewrite: Option<ConversationalStageSnapshot>,
    metric_gain: Option<MetricSet>,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Clone, Serialize)]
struct ConversationalStageSnapshot {
    rankings: RankingSnapshot,
    metrics: MetricSet,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct ConversationalAggregateSnapshot {
    total_cases: usize,
    applied: usize,
    skipped: usize,
    failed: usize,
    mean_rewrite_elapsed_ms: f64,
    retrieval_units: ConversationalAggregateRetrievalUnits,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct ConversationalAggregateRetrievalUnits {
    asset: ConversationalAggregateUnitSnapshot,
    chunk: ConversationalAggregateUnitSnapshot,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct ConversationalAggregateUnitSnapshot {
    evaluated_applied_cases: usize,
    metric_gain: Option<MetricSet>,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct PairedSnapshot {
    cases: Vec<PairedCaseSnapshot>,
    aggregate: PairedAggregateSnapshot,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct PairedCaseSnapshot {
    case_id: String,
    metric_deltas: Option<MetricSet>,
}

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct PairedAggregateSnapshot {
    metric_deltas: MetricSet,
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
fn writable_corpus_snapshot(source: &rusqlite::Connection) -> Result<rusqlite::Connection, String> {
    use rusqlite::{backup::Backup, Connection, OptionalExtension};

    let output_dir = cargo_target_dir()?.join("rag-baseline");
    std::fs::create_dir_all(&output_dir)
        .map_err(|error| format!("Failed to create baseline working directory: {error}"))?;
    let snapshot_path = output_dir.join("soip-1961-working.sqlite");
    for path in [
        snapshot_path.clone(),
        snapshot_path.with_extension("sqlite-wal"),
        snapshot_path.with_extension("sqlite-shm"),
    ] {
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|error| format!("Failed to reset baseline working copy: {error}"))?;
        }
    }

    let mut snapshot = Connection::open(&snapshot_path)
        .map_err(|error| format!("Failed to open baseline working copy: {error}"))?;
    {
        let backup = Backup::new(source, &mut snapshot)
            .map_err(|error| format!("Failed to initialize baseline SQLite backup: {error}"))?;
        backup
            .run_to_completion(128, std::time::Duration::from_millis(5), None)
            .map_err(|error| format!("Failed to copy baseline corpus: {error}"))?;
    }
    snapshot
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|error| format!("Failed to configure baseline working copy: {error}"))?;
    let has_chunks = snapshot
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'rag_chunks'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| format!("Failed to inspect baseline chunk schema: {error}"))?
        .is_some();
    if !has_chunks {
        snapshot
            .execute_batch(RAG_CHUNKS_MIGRATION_SQL)
            .map_err(|error| format!("Failed to migrate baseline working copy chunks: {error}"))?;
    }
    Ok(snapshot)
}

#[cfg(not(feature = "local-ml"))]
fn backfill_corpus_chunks(
    conn: &rusqlite::Connection,
    engine: &crate::nlp::embeddings::EmbeddingEngine,
    collection_id: &str,
) -> Result<(), String> {
    let assets = {
        let mut stmt = conn
            .prepare(
                "SELECT a.item_id, a.id
                 FROM assets a
                 JOIN items i ON i.id = a.item_id
                 WHERE i.collection_id = ?1
                   AND (
                     EXISTS(SELECT 1 FROM extractions e
                            WHERE e.asset_id = a.id
                              AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0)
                     OR EXISTS(SELECT 1 FROM transcriptions t
                               WHERE t.asset_id = a.id
                                 AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0)
                   )
                 ORDER BY a.created_at, a.id",
            )
            .map_err(|error| format!("Failed to prepare baseline chunk candidates: {error}"))?;
        let rows = stmt
            .query_map([collection_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("Failed to query baseline chunk candidates: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to read baseline chunk candidates: {error}"))?
    };
    for (item_id, asset_id) in assets {
        crate::nlp::embeddings::backfill_asset_rag_chunks(engine, conn, &item_id, &asset_id)?;
    }
    Ok(())
}

#[cfg(not(feature = "local-ml"))]
fn top_k(mut ranking: Vec<String>, k: usize) -> Vec<String> {
    ranking.truncate(k);
    ranking
}

#[cfg(not(feature = "local-ml"))]
fn asset_ranking(
    conn: &rusqlite::Connection,
    unit: super::retrieval::RetrievalUnit,
    ranking: Vec<String>,
    k: usize,
) -> Result<Vec<String>, String> {
    use rusqlite::OptionalExtension;

    if unit == super::retrieval::RetrievalUnit::Asset {
        return Ok(top_k(ranking, k));
    }
    let mut assets = Vec::with_capacity(k);
    let mut seen = HashSet::with_capacity(k);
    for chunk_id in ranking {
        let asset_id = conn
            .query_row(
                "SELECT asset_id FROM rag_chunks WHERE id = ?1",
                [&chunk_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Failed to map baseline chunk to asset: {error}"))?;
        if let Some(asset_id) = asset_id {
            if seen.insert(asset_id.clone()) {
                assets.push(asset_id);
                if assets.len() == k {
                    break;
                }
            }
        }
    }
    Ok(assets)
}

#[cfg(not(feature = "local-ml"))]
struct BaselineQuery<'a> {
    text: &'a str,
    embedding: &'a [f32],
}

#[cfg(not(feature = "local-ml"))]
async fn evaluate_query_variant(
    conn: &rusqlite::Connection,
    unit: super::retrieval::RetrievalUnit,
    queries: &[BaselineQuery<'_>],
    rerank_query: &str,
    params: &super::params::RagParams,
    api_key: &str,
    reranker_model: &str,
    expected_asset_ids: &[String],
) -> Result<ConversationalStageSnapshot, String> {
    let mut vector_legs = Vec::with_capacity(queries.len());
    let mut lexical_legs = Vec::with_capacity(queries.len());
    let mut all_legs = Vec::with_capacity(queries.len().saturating_mul(2));
    for query in queries {
        let vector = match unit {
            super::retrieval::RetrievalUnit::Asset => super::retrieval::vector_leg(
                conn,
                query.embedding,
                params.candidates_per_leg,
                params.min_similarity,
            )?,
            super::retrieval::RetrievalUnit::Chunk => super::retrieval::chunk_vector_leg(
                conn,
                query.embedding,
                params.candidates_per_leg,
                params.min_similarity,
            )?,
        };
        let lexical = match unit {
            super::retrieval::RetrievalUnit::Asset => {
                super::retrieval::lexical_leg(conn, query.text, params.candidates_per_leg)?
            }
            super::retrieval::RetrievalUnit::Chunk => {
                super::retrieval::chunk_lexical_leg(conn, query.text, params.candidates_per_leg)?
            }
        };
        vector_legs.push(vector.clone());
        lexical_legs.push(lexical.clone());
        all_legs.push(vector);
        all_legs.push(lexical);
    }

    let vector = super::retrieval::rrf_fuse(
        &vector_legs,
        params.fusion_candidate_limit,
        params.rrf_k as f64,
    );
    let lexical = super::retrieval::rrf_fuse(
        &lexical_legs,
        params.fusion_candidate_limit,
        params.rrf_k as f64,
    );
    let fused = super::retrieval::rrf_fuse(
        &all_legs,
        params.fusion_candidate_limit,
        params.rrf_k as f64,
    );
    let retrieval_queries = queries
        .iter()
        .map(|query| super::retrieval::RetrievalQuery {
            text: query.text,
            embedding: Some(query.embedding),
        })
        .collect::<Vec<_>>();
    let candidates =
        super::retrieval::hybrid_retrieve_candidates(conn, &retrieval_queries, params, unit)?;
    let reranked = super::reranker::rerank_candidates(
        rerank_query,
        candidates,
        api_key,
        reranker_model,
        params.rerank_depth,
    )
    .await;
    let rankings = RankingSnapshot {
        vector: asset_ranking(
            conn,
            unit,
            vector.into_iter().map(|(record_id, _)| record_id).collect(),
            params.top_k,
        )?,
        lexical: asset_ranking(
            conn,
            unit,
            lexical
                .into_iter()
                .map(|(record_id, _)| record_id)
                .collect(),
            params.top_k,
        )?,
        fused: asset_ranking(
            conn,
            unit,
            fused.into_iter().map(|(record_id, _)| record_id).collect(),
            params.top_k,
        )?,
        reranked: top_k(
            reranked
                .into_iter()
                .map(|candidate| candidate.record.asset_id)
                .collect(),
            params.top_k,
        ),
    };
    Ok(ConversationalStageSnapshot {
        metrics: MetricSet::calculate(&rankings, expected_asset_ids, params.top_k),
        rankings,
    })
}

#[cfg(not(feature = "local-ml"))]
fn average_conversational_gain(
    gains: impl Iterator<Item = Option<MetricSet>>,
) -> ConversationalAggregateUnitSnapshot {
    let mut total = MetricSet::default();
    let mut count = 0;
    for gain in gains.flatten() {
        total.add(gain);
        count += 1;
    }
    ConversationalAggregateUnitSnapshot {
        evaluated_applied_cases: count,
        metric_gain: (count > 0).then(|| total.divide(count)),
    }
}

#[cfg(not(feature = "local-ml"))]
fn aggregate_conversational_cases(
    cases: &[ConversationalCaseSnapshot],
) -> ConversationalAggregateSnapshot {
    let mut applied = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut elapsed_ms = 0_u128;
    for case in cases {
        elapsed_ms += case.rewrite.elapsed_ms;
        match case.rewrite.status {
            super::query_rewrite::RewriteStatus::Applied => applied += 1,
            super::query_rewrite::RewriteStatus::Skipped => skipped += 1,
            super::query_rewrite::RewriteStatus::Failed => failed += 1,
        }
    }
    ConversationalAggregateSnapshot {
        total_cases: cases.len(),
        applied,
        skipped,
        failed,
        mean_rewrite_elapsed_ms: elapsed_ms as f64 / cases.len() as f64,
        retrieval_units: ConversationalAggregateRetrievalUnits {
            asset: average_conversational_gain(
                cases
                    .iter()
                    .map(|case| case.retrieval_units.asset.metric_gain),
            ),
            chunk: average_conversational_gain(
                cases
                    .iter()
                    .map(|case| case.retrieval_units.chunk.metric_gain),
            ),
        },
    }
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
fn aggregate_metric_set(aggregate: &AggregateSnapshot) -> MetricSet {
    MetricSet {
        vector: aggregate.vector,
        lexical: aggregate.lexical,
        fused: aggregate.fused,
        reranked: aggregate.reranked,
    }
}

#[cfg(not(feature = "local-ml"))]
fn paired_snapshot(asset: &UnitSnapshot, chunk: &UnitSnapshot) -> PairedSnapshot {
    let cases = asset
        .cases
        .iter()
        .zip(&chunk.cases)
        .map(|(asset_case, chunk_case)| PairedCaseSnapshot {
            case_id: asset_case.case_id.clone(),
            metric_deltas: chunk_case
                .metrics
                .zip(asset_case.metrics)
                .map(|(chunk_metrics, asset_metrics)| chunk_metrics.delta(asset_metrics)),
        })
        .collect();
    PairedSnapshot {
        cases,
        aggregate: PairedAggregateSnapshot {
            metric_deltas: aggregate_metric_set(&chunk.aggregate)
                .delta(aggregate_metric_set(&asset.aggregate)),
        },
    }
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

    // Verify the private source before any paid work, then clone it with SQLite's
    // backup API. All schema changes and chunk writes happen only in target/.
    let source_conn = open_verified_corpus(&manifest)?;
    let params = super::params::rag_params_from_settings(&source_conn);
    if params.top_k != manifest.default_k {
        return Err(format!(
            "Golden default_k {} does not match the production rag_top_k {}",
            manifest.default_k, params.top_k
        ));
    }
    let embedding_config = crate::nlp::embeddings::config_from_settings(&source_conn)?;
    let reranker_api_key = embedding_config.api_key.clone();
    let embedding_engine = crate::nlp::embeddings::get_or_init_engine(embedding_config)?;
    let reranker_model = super::reranker::resolve_reranker_model(&source_conn);
    let rewrite_api_key = crate::settings::get_setting(&source_conn, "openrouter_api_key")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let rewrite_model = super::commands::resolve_answer_model(&source_conn);
    let conn = writable_corpus_snapshot(&source_conn)?;
    drop(source_conn);
    let backfill_engine = std::sync::Arc::clone(&embedding_engine);
    let backfill_collection_id = manifest.collection.id.clone();
    let conn = tokio::task::spawn_blocking(move || {
        backfill_corpus_chunks(&conn, backfill_engine.as_ref(), &backfill_collection_id)?;
        Ok::<rusqlite::Connection, String>(conn)
    })
    .await
    .map_err(|error| format!("RAG baseline chunk backfill worker failed: {error}"))??;

    let k = manifest.default_k;
    let mut asset_cases = Vec::with_capacity(40);
    let mut chunk_cases = Vec::with_capacity(40);
    for case in manifest
        .cases
        .iter()
        .filter(|case| case.conversation.is_none())
    {
        let embedding_engine = std::sync::Arc::clone(&embedding_engine);
        let question = case.question.clone();
        let query_embedding =
            tokio::task::spawn_blocking(move || embedding_engine.embed_text(&question))
                .await
                .map_err(|error| {
                    format!("RAG baseline query embedding worker failed: {error}")
                })??;
        let query = [BaselineQuery {
            text: &case.question,
            embedding: &query_embedding,
        }];

        for unit in [
            super::retrieval::RetrievalUnit::Asset,
            super::retrieval::RetrievalUnit::Chunk,
        ] {
            let stage = evaluate_query_variant(
                &conn,
                unit,
                &query,
                &case.question,
                &params,
                &reranker_api_key,
                &reranker_model,
                &case.expected_asset_ids,
            )
            .await?;
            let snapshot = QuerySnapshot {
                case_id: case.id.clone(),
                category: case.category,
                slices: case.slices.clone(),
                challenge_type: case.challenge_type,
                evidence_cues: case.evidence_cues.clone(),
                expected_asset_ids: case.expected_asset_ids.clone(),
                rankings: stage.rankings,
                metrics: (!case.expected_asset_ids.is_empty()).then_some(stage.metrics),
            };
            match unit {
                super::retrieval::RetrievalUnit::Asset => asset_cases.push(snapshot),
                super::retrieval::RetrievalUnit::Chunk => chunk_cases.push(snapshot),
            }
        }
    }

    let mut conversational_cases = Vec::with_capacity(manifest.cases.len().saturating_sub(40));
    for case in manifest
        .cases
        .iter()
        .filter(|case| case.conversation.is_some())
    {
        let conversation = case
            .conversation
            .as_ref()
            .expect("filtered conversational case must carry metadata");
        let history = conversation
            .history
            .iter()
            .map(|turn| super::RagChatTurn {
                role: match turn.role {
                    GoldenHistoryRole::User => "user",
                    GoldenHistoryRole::Assistant => "assistant",
                }
                .to_string(),
                content: turn.content.clone(),
            })
            .collect::<Vec<_>>();
        let rewrite_started = std::time::Instant::now();
        let rewrite_outcome = super::query_rewrite::rewrite_query(
            super::query_rewrite::RewriteProvider::OpenRouter {
                api_key: &rewrite_api_key,
                model: &rewrite_model,
            },
            &case.question,
            &history,
            params.history_turns,
            params.history_turn_max_chars,
        )
        .await;
        let rewrite = RewriteSnapshot {
            status: rewrite_outcome.status(),
            elapsed_ms: rewrite_started.elapsed().as_millis(),
        };
        let rewritten_query = match rewrite_outcome {
            super::query_rewrite::RewriteOutcome::Applied(query) => Some(query),
            super::query_rewrite::RewriteOutcome::Skipped
            | super::query_rewrite::RewriteOutcome::Failed => None,
        };

        let original_embedding_engine = std::sync::Arc::clone(&embedding_engine);
        let question = case.question.clone();
        let original_embedding =
            tokio::task::spawn_blocking(move || original_embedding_engine.embed_text(&question))
                .await
                .map_err(|error| {
                    format!("RAG baseline query embedding worker failed: {error}")
                })??;
        let rewritten_embedding = match rewritten_query.as_ref() {
            Some(query) => {
                let embedding_engine = std::sync::Arc::clone(&embedding_engine);
                let query = query.clone();
                Some(
                    tokio::task::spawn_blocking(move || embedding_engine.embed_text(&query))
                        .await
                        .map_err(|error| {
                            format!("RAG baseline rewritten embedding worker failed: {error}")
                        })??,
                )
            }
            None => None,
        };

        let mut asset = None;
        let mut chunk = None;
        for unit in [
            super::retrieval::RetrievalUnit::Asset,
            super::retrieval::RetrievalUnit::Chunk,
        ] {
            let original_queries = [BaselineQuery {
                text: &case.question,
                embedding: &original_embedding,
            }];
            let original_only = evaluate_query_variant(
                &conn,
                unit,
                &original_queries,
                &case.question,
                &params,
                &reranker_api_key,
                &reranker_model,
                &case.expected_asset_ids,
            )
            .await?;
            let original_plus_rewrite =
                match (rewritten_query.as_deref(), rewritten_embedding.as_deref()) {
                    (Some(rewritten), Some(embedding)) => {
                        let queries = [
                            BaselineQuery {
                                text: &case.question,
                                embedding: &original_embedding,
                            },
                            BaselineQuery {
                                text: rewritten,
                                embedding,
                            },
                        ];
                        Some(
                            evaluate_query_variant(
                                &conn,
                                unit,
                                &queries,
                                rewritten,
                                &params,
                                &reranker_api_key,
                                &reranker_model,
                                &case.expected_asset_ids,
                            )
                            .await?,
                        )
                    }
                    _ => None,
                };
            let metric_gain = original_plus_rewrite
                .as_ref()
                .map(|rewritten| rewritten.metrics.delta(original_only.metrics));
            let snapshot = ConversationalUnitSnapshot {
                original_only,
                original_plus_rewrite,
                metric_gain,
            };
            match unit {
                super::retrieval::RetrievalUnit::Asset => asset = Some(snapshot),
                super::retrieval::RetrievalUnit::Chunk => chunk = Some(snapshot),
            }
        }
        conversational_cases.push(ConversationalCaseSnapshot {
            case_id: case.id.clone(),
            category: case.category,
            slices: case.slices.clone(),
            challenge_type: case.challenge_type,
            evidence_cues: case.evidence_cues.clone(),
            expected_asset_ids: case.expected_asset_ids.clone(),
            rewrite,
            retrieval_units: ConversationalRetrievalUnitSnapshots {
                asset: asset.expect("asset conversational snapshot must be evaluated"),
                chunk: chunk.expect("chunk conversational snapshot must be evaluated"),
            },
        });
    }

    let asset = UnitSnapshot {
        aggregate: aggregate_snapshots(&asset_cases),
        cases: asset_cases,
    };
    let chunk = UnitSnapshot {
        aggregate: aggregate_snapshots(&chunk_cases),
        cases: chunk_cases,
    };
    println!("retrieval_unit=asset");
    print_metrics(&asset.cases, &asset.aggregate, k);
    println!("retrieval_unit=chunk");
    print_metrics(&chunk.cases, &chunk.aggregate, k);
    let paired = paired_snapshot(&asset, &chunk);
    let conversational = ConversationalSnapshot {
        aggregate: aggregate_conversational_cases(&conversational_cases),
        cases: conversational_cases,
    };
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
        retrieval_units: RetrievalUnitSnapshots { asset, chunk },
        paired,
        conversational,
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
    assert_eq!(manifest.cases.len(), 44);
    assert_eq!(
        manifest
            .cases
            .iter()
            .filter(|case| case.conversation.is_none())
            .count(),
        40
    );
    assert_eq!(
        manifest
            .cases
            .iter()
            .filter(|case| case.conversation.is_some())
            .count(),
        4
    );
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

    let missing_conversation_source = GOLDEN_MANIFEST_JSON.replacen(
        "\"source_case_id\": \"soip-001\"",
        "\"source_case_id\": \"missing-source\"",
        1,
    );
    assert!(GoldenManifest::parse(&missing_conversation_source).is_err());

    let invented_history = GOLDEN_MANIFEST_JSON.replacen(
        "\"content\": \"Después del encuentro nacional de dirigentes gremiales, ¿qué curso de acción se adoptó y cuándo volverían a reunirse?\"",
        "\"content\": \"Un hecho no presente en los casos fuente\"",
        1,
    );
    assert!(GoldenManifest::parse(&invented_history).is_err());
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

#[test]
fn golden_manifest_balances_hard_retrieval_challenges_and_bounds_safe_cues() {
    let raw: serde_json::Value = serde_json::from_str(GOLDEN_MANIFEST_JSON)
        .expect("golden JSON must be syntactically valid");
    let cases = raw["cases"]
        .as_array()
        .expect("golden cases must be an array");
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();

    for case in cases {
        let challenge = case["challenge_type"]
            .as_str()
            .expect("every case must declare one challenge_type");
        assert!(
            matches!(
                challenge,
                "paraphrase" | "lexical_mismatch" | "ambiguity" | "multi_evidence" | "no_answer"
            ),
            "unsupported challenge_type: {challenge}"
        );
        if case["conversation"].is_null() {
            *counts.entry(challenge).or_default() += 1;
        }

        let cues = case["evidence_cues"]
            .as_array()
            .expect("every case must declare bounded evidence_cues");
        assert!(cues.len() <= 2, "a case may expose at most two cues");
        assert!(cues.iter().all(|cue| {
            cue.as_str()
                .is_some_and(|text| !text.trim().is_empty() && text.chars().count() <= 32)
        }));

        let expected = case["expected_asset_ids"]
            .as_array()
            .expect("expected_asset_ids must be an array");
        assert_eq!(
            challenge == "no_answer",
            expected.is_empty(),
            "no-answer challenge metadata must match grounding"
        );
        assert_eq!(
            expected.is_empty(),
            cues.is_empty(),
            "only grounded cases may expose retrieval cues"
        );
        if challenge == "multi_evidence" {
            assert!(
                expected.len() > 1,
                "multi-evidence challenges require multiple grounded assets"
            );
        }
    }

    assert_eq!(cases.len(), 44);
    assert_eq!(counts.len(), 5);
    assert!(
        counts.values().all(|count| (7..=9).contains(count)),
        "challenge groups must remain balanced within 7..=9 cases: {counts:?}"
    );
}

#[cfg(not(feature = "local-ml"))]
#[test]
fn baseline_snapshot_reports_paired_and_conversational_gain_contracts() {
    fn metric_set(value: f64) -> MetricSet {
        let metric = RankingMetrics {
            recall_at_k: value,
            reciprocal_rank: value,
            ndcg_at_k: value,
        };
        MetricSet {
            vector: metric,
            lexical: metric,
            fused: metric,
            reranked: metric,
        }
    }

    fn conversation_unit(
        rankings: &RankingSnapshot,
        rewritten_metrics: Option<MetricSet>,
    ) -> ConversationalUnitSnapshot {
        let original_metrics = metric_set(0.25);
        let original_only = ConversationalStageSnapshot {
            rankings: rankings.clone(),
            metrics: original_metrics,
        };
        let original_plus_rewrite = rewritten_metrics.map(|metrics| ConversationalStageSnapshot {
            rankings: rankings.clone(),
            metrics,
        });
        let metric_gain = original_plus_rewrite
            .as_ref()
            .map(|rewritten| rewritten.metrics.delta(original_metrics));
        ConversationalUnitSnapshot {
            original_only,
            original_plus_rewrite,
            metric_gain,
        }
    }

    let rankings = RankingSnapshot {
        vector: vec!["asset-a".to_string()],
        lexical: vec!["asset-a".to_string()],
        fused: vec!["asset-a".to_string()],
        reranked: vec!["asset-a".to_string()],
    };
    let metrics = metric_set(0.0);
    let case = QuerySnapshot {
        case_id: "worked-example".to_string(),
        category: GoldenCategory::LaborHistory,
        slices: vec![GoldenSlice::NaturalLanguage],
        challenge_type: GoldenChallenge::Paraphrase,
        evidence_cues: vec!["worked cue".to_string()],
        expected_asset_ids: vec!["asset-a".to_string()],
        rankings: rankings.clone(),
        metrics: Some(metrics),
    };
    let aggregate = AggregateSnapshot {
        evaluated_answerable_cases: 1,
        no_answer_cases: 0,
        vector: RankingMetrics::default(),
        lexical: RankingMetrics::default(),
        fused: RankingMetrics::default(),
        reranked: RankingMetrics::default(),
    };
    let asset = UnitSnapshot {
        cases: vec![case.clone()],
        aggregate: aggregate.clone(),
    };
    let chunk = UnitSnapshot {
        cases: vec![case],
        aggregate,
    };
    let paired = paired_snapshot(&asset, &chunk);
    let conversational_cases = vec![
        ConversationalCaseSnapshot {
            case_id: "conversation-applied".to_string(),
            category: GoldenCategory::LaborHistory,
            slices: vec![GoldenSlice::NaturalLanguage],
            challenge_type: GoldenChallenge::Paraphrase,
            evidence_cues: vec!["worked cue".to_string()],
            expected_asset_ids: vec!["asset-a".to_string()],
            rewrite: RewriteSnapshot {
                status: super::query_rewrite::RewriteStatus::Applied,
                elapsed_ms: 10,
            },
            retrieval_units: ConversationalRetrievalUnitSnapshots {
                asset: conversation_unit(&rankings, Some(metric_set(0.75))),
                chunk: conversation_unit(&rankings, Some(metric_set(0.75))),
            },
        },
        ConversationalCaseSnapshot {
            case_id: "conversation-failed".to_string(),
            category: GoldenCategory::LaborHistory,
            slices: vec![GoldenSlice::NaturalLanguage],
            challenge_type: GoldenChallenge::Paraphrase,
            evidence_cues: vec!["worked cue".to_string()],
            expected_asset_ids: vec!["asset-a".to_string()],
            rewrite: RewriteSnapshot {
                status: super::query_rewrite::RewriteStatus::Failed,
                elapsed_ms: 30,
            },
            retrieval_units: ConversationalRetrievalUnitSnapshots {
                asset: conversation_unit(&rankings, None),
                chunk: conversation_unit(&rankings, None),
            },
        },
    ];
    let conversational = ConversationalSnapshot {
        aggregate: aggregate_conversational_cases(&conversational_cases),
        cases: conversational_cases,
    };
    let snapshot = BaselineSnapshot {
        schema_version: GOLDEN_SCHEMA_VERSION,
        generated_at_unix_seconds: 0,
        collection: GoldenCollectionSnapshot {
            id: "collection".to_string(),
            name: "SOIP 1961".to_string(),
        },
        corpus: CorpusSnapshot {
            image_assets: 1,
            non_empty_ocr: 1,
            embeddings: 1,
        },
        k: 10,
        retrieval_units: RetrievalUnitSnapshots { asset, chunk },
        paired,
        conversational,
    };

    let json = serde_json::to_value(snapshot).expect("snapshot must serialize");
    for unit in ["asset", "chunk"] {
        assert!(json["retrieval_units"][unit]["cases"].is_array());
        assert!(json["retrieval_units"][unit]["aggregate"].is_object());
        assert!(
            json["conversational"]["cases"][0]["retrieval_units"][unit]["original_only"]
                ["rankings"]
                .is_object()
        );
        assert!(json["conversational"]["cases"][0]["retrieval_units"][unit]
            ["original_plus_rewrite"]["metrics"]
            .is_object());
        assert_eq!(
            json["conversational"]["aggregate"]["retrieval_units"][unit]["metric_gain"]["fused"]
                ["recall_at_k"],
            0.5
        );
    }
    assert_eq!(json["paired"]["cases"][0]["case_id"], "worked-example");
    assert!(json["paired"]["cases"][0]["metric_deltas"].is_object());
    assert!(json["paired"]["aggregate"]["metric_deltas"].is_object());
    assert_eq!(
        json["conversational"]["cases"][1]["rewrite"]["status"],
        "failed"
    );
    assert_eq!(
        json["conversational"]["aggregate"]["mean_rewrite_elapsed_ms"],
        20.0
    );
    assert_eq!(json["conversational"]["aggregate"]["applied"], 1);
    assert_eq!(json["conversational"]["aggregate"]["failed"], 1);
    let serialized = serde_json::to_string(&json).expect("snapshot JSON must serialize");
    assert!(!serialized.contains("\"question\""));
    assert!(!serialized.contains("\"history\""));
    assert!(!serialized.contains("generated_query"));
}
