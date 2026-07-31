#[cfg(feature = "local-ml")]
use std::collections::HashMap;
#[cfg(any(test, not(feature = "local-ml")))]
use std::collections::HashSet;
#[cfg(feature = "local-ml")]
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
#[cfg(feature = "local-ml")]
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[cfg(feature = "local-ml")]
use ndarray::{Array2, ArrayViewD};
#[cfg(feature = "local-ml")]
use ort::{
    inputs,
    session::{builder::GraphOptimizationLevel, Session},
    value::TensorRef,
};
#[cfg(any(test, not(feature = "local-ml")))]
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "local-ml")]
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
#[cfg(feature = "local-ml")]
use tokenizers::{
    utils::truncation::{TruncationParams, TruncationStrategy},
    Tokenizer,
};

use super::retrieval::RrfCandidate;

const RERANK_DOCUMENT_MAX_CHARS: usize = 6000;
const LOCAL_RERANKER_DIR_NAME: &str = "bge-reranker-v2-m3";
#[cfg(feature = "local-ml")]
const LOCAL_RERANKER_MODEL_FILE: &str = "model_int8.onnx";
#[cfg(feature = "local-ml")]
const LOCAL_RERANKER_TOKENIZER_FILE: &str = "tokenizer.json";
const LOCAL_RERANKER_SOURCE_REPO: &str = "onnx-community/bge-reranker-v2-m3-ONNX";

#[cfg(not(feature = "local-ml"))]
const OPENROUTER_RERANK_URL: &str = "https://openrouter.ai/api/v1/rerank";
/// Default reranker model for the Lite (OpenRouter) path — overridable via
/// the `rag_reranker_model` setting (RAG Params tab). See
/// `resolve_reranker_model`.
#[cfg(not(feature = "local-ml"))]
pub(crate) const DEFAULT_OPENROUTER_RERANK_MODEL: &str = "cohere/rerank-4-fast";

#[cfg(feature = "local-ml")]
const LOCAL_RERANKER_RESOLVE_BASE_URL: &str =
    "https://huggingface.co/onnx-community/bge-reranker-v2-m3-ONNX/resolve/6f5ff65298512715a1e669753bc754d2bc8f367b";
#[cfg(feature = "local-ml")]
const LOCAL_RERANKER_MAX_TOKENS: usize = 2048;
#[cfg(feature = "local-ml")]
const LOCAL_RERANKER_BATCH_SIZE: usize = 4;
#[cfg(feature = "local-ml")]
const DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;
#[cfg(feature = "local-ml")]
const DOWNLOAD_TIMEOUT_SECS: u64 = 3600;

#[cfg(feature = "local-ml")]
#[derive(Clone, Copy)]
struct ArtifactSpec {
    filename: &'static str,
    source_path: &'static str,
    expected_size: u64,
    expected_sha256: &'static str,
}

#[cfg(feature = "local-ml")]
const LOCAL_RERANKER_ARTIFACTS: [ArtifactSpec; 2] = [
    ArtifactSpec {
        filename: LOCAL_RERANKER_MODEL_FILE,
        source_path: "onnx/model_int8.onnx",
        expected_size: 570_727_094,
        expected_sha256: "912fc1215c2dbff6499700534bd8d31253af01573861abbfc43afd1fab6cce5d",
    },
    ArtifactSpec {
        filename: LOCAL_RERANKER_TOKENIZER_FILE,
        source_path: "tokenizer.json",
        expected_size: 17_082_900,
        expected_sha256: "8bf8afbfd11306bd872018c53bfdf2e160a56f8edbcf49933324404791c148d3",
    },
];

#[cfg(not(feature = "local-ml"))]
#[derive(Serialize)]
struct RerankRequest<'a> {
    model: &'a str,
    query: &'a str,
    documents: &'a [String],
    top_n: usize,
}

#[cfg(any(test, not(feature = "local-ml")))]
#[derive(Debug, Deserialize)]
struct RerankResponse {
    results: Vec<RerankResult>,
}

#[cfg(any(test, not(feature = "local-ml")))]
#[derive(Debug, Deserialize)]
struct RerankResult {
    index: usize,
    relevance_score: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalRerankerModelFileInfo {
    pub filename: String,
    pub source_path: String,
    pub destination: String,
    pub expected_size_bytes: u64,
    pub actual_size_bytes: Option<u64>,
    pub valid: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalRerankerModelInfo {
    pub available: bool,
    pub can_auto_download: bool,
    pub directory: String,
    pub path: String,
    pub required_files: Vec<LocalRerankerModelFileInfo>,
    pub source_repo: String,
}

#[derive(Clone, Serialize)]
pub struct RerankerDownloadProgressPayload {
    pub pct: u8,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub file: String,
}

#[derive(Clone, Serialize)]
pub struct RerankerDownloadCompletePayload {
    pub path: String,
}

#[derive(Clone, Serialize)]
pub struct RerankerDownloadErrorPayload {
    pub error: String,
}

/// Resuelve el modelo del reranker Lite desde `rag_reranker_model`
/// (RAG Params tab). Vacío, ausente o solo-espacios cae a
/// `DEFAULT_OPENROUTER_RERANK_MODEL` — nunca falla.
#[cfg(not(feature = "local-ml"))]
pub(crate) fn resolve_reranker_model(conn: &rusqlite::Connection) -> String {
    crate::settings::get_setting(conn, "rag_reranker_model")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_OPENROUTER_RERANK_MODEL.to_string())
}

/// Reranks Lite candidates through OpenRouter. Every failure preserves the
/// complete original RRF list so final source construction remains available.
#[cfg(not(feature = "local-ml"))]
pub(crate) async fn rerank_candidates(
    question: &str,
    candidates: Vec<RrfCandidate>,
    api_key: &str,
    model: &str,
    depth: usize,
) -> Vec<RrfCandidate> {
    if candidates.is_empty() {
        return candidates;
    }
    if api_key.trim().is_empty() {
        warn_fallback("missing OpenRouter API key");
        return candidates;
    }

    // Solo la cabeza de la lista fusionada se puntúa. Cohere factura por
    // documento, así que mandar los 40 cuando `top_k` descarta 34 es plata y
    // latencia tirada — y es la misma profundidad que puntúa Pro (paridad).
    let candidates = truncate_to_depth(candidates, depth);

    let documents = candidate_documents(question, &candidates);
    let request = RerankRequest {
        model,
        query: question,
        documents: &documents,
        // Devolvemos TODO lo puntuado reordenado; `build_sources` aplica
        // `top_k`. Así ambas variantes entregan la misma lista.
        top_n: documents.len(),
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
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

    // Pedimos `top_n = documents.len()`, así que esperamos de vuelta la lista
    // completa ya recortada a la profundidad, reordenada.
    let ranked = apply_response(&candidates, response, candidates.len());
    if ranked.is_none() {
        warn_fallback("provider response failed validation");
    }
    ranked.unwrap_or(candidates)
}

#[cfg(not(feature = "local-ml"))]
fn warn_fallback(reason: &str) {
    eprintln!("[rag] Lite reranking unavailable ({reason}); preserving RRF order");
}

#[cfg(feature = "local-ml")]
#[derive(Debug, Clone, Copy)]
enum LocalRerankFailure {
    ModelUnavailable,
    InitializationFailed,
    TokenizationFailed,
    InferenceFailed,
    InvalidOutput,
}

#[cfg(feature = "local-ml")]
impl LocalRerankFailure {
    fn reason(self) -> &'static str {
        match self {
            Self::ModelUnavailable => "model assets unavailable",
            Self::InitializationFailed => "model initialization failed",
            Self::TokenizationFailed => "tokenization failed",
            Self::InferenceFailed => "inference failed",
            Self::InvalidOutput => "model output failed validation",
        }
    }
}

#[cfg(feature = "local-ml")]
struct LocalCrossEncoder {
    tokenizer: Tokenizer,
    pad_id: u32,
    session: Session,
}

#[cfg(feature = "local-ml")]
type CachedCrossEncoder = Arc<Mutex<LocalCrossEncoder>>;

#[cfg(feature = "local-ml")]
static LOCAL_RERANKER_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedCrossEncoder>>> =
    OnceLock::new();

/// Runs the Pro cross-encoder locally. Any failure returns the complete input
/// vector unchanged, including its original RRF scores.
#[cfg(feature = "local-ml")]
pub(crate) fn rerank_candidates_local(
    question: &str,
    candidates: Vec<RrfCandidate>,
    model_dir: &Path,
    depth: usize,
) -> Vec<RrfCandidate> {
    if candidates.is_empty() {
        return candidates;
    }

    // Misma profundidad que Lite: el cross-encoder es lo más caro del camino
    // crítico y puntuar 40 candidatos para citar 6 era gratuito solo en teoría.
    let candidates = truncate_to_depth(candidates, depth);

    let result = (|| {
        let documents = candidate_documents(question, &candidates);
        let engine = cached_cross_encoder(model_dir)?;
        let mut engine = engine
            .lock()
            .map_err(|_| LocalRerankFailure::InitializationFailed)?;
        let logits = engine.score(question, &documents)?;
        rank_candidates_from_logits(&candidates, &logits).ok_or(LocalRerankFailure::InvalidOutput)
    })();

    match result {
        Ok(ranked) => ranked,
        Err(reason) => {
            eprintln!(
                "[rag] Local reranking unavailable ({}); preserving RRF order",
                reason.reason()
            );
            candidates
        }
    }
}

#[cfg(feature = "local-ml")]
fn cached_cross_encoder(model_dir: &Path) -> Result<CachedCrossEncoder, LocalRerankFailure> {
    let key = std::fs::canonicalize(model_dir).unwrap_or_else(|_| model_dir.to_path_buf());
    let cache = LOCAL_RERANKER_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(engine) = cache
        .lock()
        .map_err(|_| LocalRerankFailure::InitializationFailed)?
        .get(&key)
        .cloned()
    {
        return Ok(engine);
    }

    let mut cache = cache
        .lock()
        .map_err(|_| LocalRerankFailure::InitializationFailed)?;
    if let Some(engine) = cache.get(&key).cloned() {
        return Ok(engine);
    }

    // Keep initialization single-flight. Failures are deliberately not cached,
    // so installing or repairing the model makes the next request retry.
    let loaded = Arc::new(Mutex::new(LocalCrossEncoder::load(model_dir)?));
    cache.insert(key, loaded.clone());
    Ok(loaded)
}

#[cfg(feature = "local-ml")]
impl LocalCrossEncoder {
    fn load(model_dir: &Path) -> Result<Self, LocalRerankFailure> {
        for artifact in LOCAL_RERANKER_ARTIFACTS {
            validate_artifact(&model_dir.join(artifact.filename), artifact)
                .map_err(|_| LocalRerankFailure::ModelUnavailable)?;
        }

        crate::nlp::embeddings::ensure_local_ort_init(model_dir)
            .map_err(|_| LocalRerankFailure::InitializationFailed)?;

        let tokenizer_path = model_dir.join(LOCAL_RERANKER_TOKENIZER_FILE);
        let mut tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|_| LocalRerankFailure::InitializationFailed)?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: LOCAL_RERANKER_MAX_TOKENS,
                strategy: TruncationStrategy::LongestFirst,
                ..Default::default()
            }))
            .map_err(|_| LocalRerankFailure::InitializationFailed)?;
        let pad_id = tokenizer
            .get_padding()
            .map(|padding| padding.pad_id)
            .or_else(|| tokenizer.token_to_id("<pad>"))
            .or_else(|| tokenizer.token_to_id("[PAD]"))
            .ok_or(LocalRerankFailure::InitializationFailed)?;

        let session = Session::builder()
            .map_err(|_| LocalRerankFailure::InitializationFailed)?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|_| LocalRerankFailure::InitializationFailed)?
            .commit_from_file(model_dir.join(LOCAL_RERANKER_MODEL_FILE))
            .map_err(|_| LocalRerankFailure::InitializationFailed)?;
        if !matches!(session.inputs.len(), 2 | 3) {
            return Err(LocalRerankFailure::InitializationFailed);
        }

        Ok(Self {
            tokenizer,
            pad_id,
            session,
        })
    }

    fn score(
        &mut self,
        question: &str,
        documents: &[String],
    ) -> Result<Vec<f32>, LocalRerankFailure> {
        let mut logits = Vec::with_capacity(documents.len());
        for batch in documents.chunks(LOCAL_RERANKER_BATCH_SIZE) {
            let pairs: Vec<(&str, &str)> = batch
                .iter()
                .map(|document| (question, document.as_str()))
                .collect();
            let encodings = self
                .tokenizer
                .encode_batch(pairs, true)
                .map_err(|_| LocalRerankFailure::TokenizationFailed)?;
            let (input_ids, attention_mask, type_ids) = batch_arrays(&encodings, self.pad_id)?;

            let outputs = match self.session.inputs.len() {
                2 => self
                    .session
                    .run(inputs![
                        TensorRef::from_array_view(&input_ids)
                            .map_err(|_| LocalRerankFailure::InferenceFailed)?,
                        TensorRef::from_array_view(&attention_mask)
                            .map_err(|_| LocalRerankFailure::InferenceFailed)?,
                    ])
                    .map_err(|_| LocalRerankFailure::InferenceFailed)?,
                3 => self
                    .session
                    .run(inputs![
                        TensorRef::from_array_view(&input_ids)
                            .map_err(|_| LocalRerankFailure::InferenceFailed)?,
                        TensorRef::from_array_view(&attention_mask)
                            .map_err(|_| LocalRerankFailure::InferenceFailed)?,
                        TensorRef::from_array_view(&type_ids)
                            .map_err(|_| LocalRerankFailure::InferenceFailed)?,
                    ])
                    .map_err(|_| LocalRerankFailure::InferenceFailed)?,
                _ => return Err(LocalRerankFailure::InitializationFailed),
            };
            let output = outputs[0]
                .try_extract_array::<f32>()
                .map_err(|_| LocalRerankFailure::InvalidOutput)?;
            logits.extend(validate_logits_output(output, batch.len())?);
        }

        if logits.len() != documents.len() {
            return Err(LocalRerankFailure::InvalidOutput);
        }
        Ok(logits)
    }
}

#[cfg(feature = "local-ml")]
fn batch_arrays(
    encodings: &[tokenizers::Encoding],
    pad_id: u32,
) -> Result<(Array2<i64>, Array2<i64>, Array2<i64>), LocalRerankFailure> {
    let sequence_length = encodings
        .iter()
        .map(|encoding| encoding.get_ids().len())
        .max()
        .filter(|length| *length > 0 && *length <= LOCAL_RERANKER_MAX_TOKENS)
        .ok_or(LocalRerankFailure::TokenizationFailed)?;
    let value_count = encodings.len().saturating_mul(sequence_length);
    let mut input_ids = vec![pad_id as i64; value_count];
    let mut attention_mask = vec![0_i64; value_count];
    let mut type_ids = vec![0_i64; value_count];

    for (row, encoding) in encodings.iter().enumerate() {
        let ids = encoding.get_ids();
        let masks = encoding.get_attention_mask();
        let types = encoding.get_type_ids();
        if ids.is_empty()
            || ids.len() > LOCAL_RERANKER_MAX_TOKENS
            || masks.len() != ids.len()
            || types.len() != ids.len()
        {
            return Err(LocalRerankFailure::TokenizationFailed);
        }
        let start = row * sequence_length;
        for index in 0..ids.len() {
            input_ids[start + index] = ids[index] as i64;
            attention_mask[start + index] = masks[index] as i64;
            type_ids[start + index] = types[index] as i64;
        }
    }

    let shape = (encodings.len(), sequence_length);
    Ok((
        Array2::from_shape_vec(shape, input_ids)
            .map_err(|_| LocalRerankFailure::TokenizationFailed)?,
        Array2::from_shape_vec(shape, attention_mask)
            .map_err(|_| LocalRerankFailure::TokenizationFailed)?,
        Array2::from_shape_vec(shape, type_ids)
            .map_err(|_| LocalRerankFailure::TokenizationFailed)?,
    ))
}

#[cfg(feature = "local-ml")]
fn validate_logits_output(
    output: ArrayViewD<'_, f32>,
    expected: usize,
) -> Result<Vec<f32>, LocalRerankFailure> {
    let logits: Vec<f32> = match output.shape() {
        [batch] if *batch == expected => output.iter().copied().collect(),
        [batch, width] if *batch == expected && *width == 1 => output.iter().copied().collect(),
        _ => return Err(LocalRerankFailure::InvalidOutput),
    };
    if logits.len() != expected || logits.iter().any(|logit| !logit.is_finite()) {
        return Err(LocalRerankFailure::InvalidOutput);
    }
    Ok(logits)
}

#[cfg(any(test, feature = "local-ml"))]
fn rank_candidates_from_logits(
    candidates: &[RrfCandidate],
    logits: &[f32],
) -> Option<Vec<RrfCandidate>> {
    if logits.len() != candidates.len() || logits.iter().any(|logit| !logit.is_finite()) {
        return None;
    }

    let mut ranked: Vec<(usize, RrfCandidate)> = candidates
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, mut candidate)| {
            candidate.score = stable_sigmoid(logits[index] as f64);
            (index, candidate)
        })
        .collect();
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left_index.cmp(right_index))
    });
    Some(ranked.into_iter().map(|(_, candidate)| candidate).collect())
}

#[cfg(any(test, feature = "local-ml"))]
fn stable_sigmoid(logit: f64) -> f64 {
    if logit >= 0.0 {
        1.0 / (1.0 + (-logit).exp())
    } else {
        let exp = logit.exp();
        exp / (1.0 + exp)
    }
}

/// Recorta la lista fusionada a los `depth` mejores candidatos por RRF.
///
/// Descartar la cola es seguro porque `params` garantiza
/// `depth >= top_k` y `build_sources` nunca mira más allá de `top_k`. Si el
/// reranking falla, la lista recortada sigue en orden RRF, así que la
/// degradación elegante se preserva.
fn truncate_to_depth(mut candidates: Vec<RrfCandidate>, depth: usize) -> Vec<RrfCandidate> {
    let depth = depth.max(1);
    if candidates.len() > depth {
        candidates.truncate(depth);
    }
    candidates
}

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

fn resolve_local_reranker_model_dir_from(
    override_dir: Option<&Path>,
    app_data_dir: Option<&Path>,
) -> PathBuf {
    override_dir.map(Path::to_path_buf).unwrap_or_else(|| {
        app_data_dir
            .map(|root| {
                root.join("models")
                    .join("rerankers")
                    .join(LOCAL_RERANKER_DIR_NAME)
            })
            .unwrap_or_else(|| {
                PathBuf::from("models")
                    .join("rerankers")
                    .join(LOCAL_RERANKER_DIR_NAME)
            })
    })
}

pub fn resolve_local_reranker_model_dir(app_data_dir: Option<&Path>) -> PathBuf {
    let override_dir = std::env::var_os("ENTROPIA_LOCAL_RERANKER_MODEL_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    resolve_local_reranker_model_dir_from(override_dir.as_deref(), app_data_dir)
}

#[cfg(feature = "local-ml")]
pub fn get_local_reranker_model_info(model_dir: PathBuf) -> LocalRerankerModelInfo {
    std::fs::create_dir_all(&model_dir).ok();
    let required_files: Vec<_> = LOCAL_RERANKER_ARTIFACTS
        .into_iter()
        .map(|artifact| inspect_artifact(&model_dir, artifact))
        .collect();
    let available = required_files.iter().all(|file| file.valid);
    LocalRerankerModelInfo {
        available,
        can_auto_download: true,
        directory: model_dir.to_string_lossy().to_string(),
        path: model_dir
            .join(LOCAL_RERANKER_MODEL_FILE)
            .to_string_lossy()
            .to_string(),
        required_files,
        source_repo: LOCAL_RERANKER_SOURCE_REPO.to_string(),
    }
}

#[cfg(not(feature = "local-ml"))]
pub fn get_local_reranker_model_info(model_dir: PathBuf) -> LocalRerankerModelInfo {
    LocalRerankerModelInfo {
        available: false,
        can_auto_download: false,
        directory: model_dir.to_string_lossy().to_string(),
        path: String::new(),
        required_files: Vec::new(),
        source_repo: LOCAL_RERANKER_SOURCE_REPO.to_string(),
    }
}

#[cfg(feature = "local-ml")]
fn inspect_artifact(directory: &Path, artifact: ArtifactSpec) -> LocalRerankerModelFileInfo {
    let destination = directory.join(artifact.filename);
    let actual_size_bytes = std::fs::metadata(&destination)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len());
    let valid = validate_artifact(&destination, artifact).is_ok();
    LocalRerankerModelFileInfo {
        filename: artifact.filename.to_string(),
        source_path: artifact.source_path.to_string(),
        destination: destination.to_string_lossy().to_string(),
        expected_size_bytes: artifact.expected_size,
        actual_size_bytes,
        valid,
    }
}

#[cfg(feature = "local-ml")]
fn validate_artifact(path: &Path, artifact: ArtifactSpec) -> Result<(), String> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| format!("Missing reranker artifact: {}", artifact.filename))?;
    if !metadata.is_file() || metadata.len() != artifact.expected_size || metadata.len() == 0 {
        return Err(format!(
            "Reranker artifact has an invalid size: {}",
            artifact.filename
        ));
    }
    let actual_sha256 = file_sha256(path)?;
    if actual_sha256 != artifact.expected_sha256 {
        return Err(format!(
            "Reranker artifact checksum mismatch: {}",
            artifact.filename
        ));
    }
    Ok(())
}

#[cfg(feature = "local-ml")]
fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|_| "Failed to open reranker artifact for validation".to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_SIZE];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "Failed to read reranker artifact for validation".to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(feature = "local-ml")]
pub fn download_local_reranker_model_files(
    model_dir: &Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    std::fs::create_dir_all(model_dir)
        .map_err(|_| "Failed to create local reranker model directory".to_string())?;

    for artifact in LOCAL_RERANKER_ARTIFACTS {
        let destination = model_dir.join(artifact.filename);
        if validate_artifact(&destination, artifact).is_ok() {
            continue;
        }
        let url = format!(
            "{LOCAL_RERANKER_RESOLVE_BASE_URL}/{}?download=true",
            artifact.source_path
        );
        download_artifact(&url, &destination, artifact, app_handle)?;
    }

    for artifact in LOCAL_RERANKER_ARTIFACTS {
        validate_artifact(&model_dir.join(artifact.filename), artifact)?;
    }
    let _ = app_handle.emit(
        "reranker:download_complete",
        RerankerDownloadCompletePayload {
            path: model_dir.to_string_lossy().to_string(),
        },
    );
    Ok(())
}

#[cfg(feature = "local-ml")]
fn download_artifact(
    url: &str,
    destination: &Path,
    artifact: ArtifactSpec,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let temporary = destination.with_extension("download.tmp");
    let _ = std::fs::remove_file(&temporary);
    let result = (|| {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
            .build()
            .map_err(|_| "Failed to initialize reranker download client".to_string())?;
        let mut response = client.get(url).send().map_err(|_| {
            format!(
                "Failed to download reranker artifact: {}",
                artifact.filename
            )
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "Reranker artifact download returned an error: {}",
                artifact.filename
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size != artifact.expected_size)
        {
            return Err(format!(
                "Reranker artifact download has an invalid size: {}",
                artifact.filename
            ));
        }

        let mut file = std::fs::File::create(&temporary)
            .map_err(|_| "Failed to create reranker download file".to_string())?;
        let mut hasher = Sha256::new();
        let mut downloaded_bytes = 0_u64;
        let mut last_reported_pct = 0_u8;
        let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_SIZE];
        loop {
            let read = response
                .read(&mut buffer)
                .map_err(|_| "Failed while reading reranker download".to_string())?;
            if read == 0 {
                break;
            }
            if downloaded_bytes.saturating_add(read as u64) > artifact.expected_size {
                return Err(format!(
                    "Reranker artifact download exceeded its expected size: {}",
                    artifact.filename
                ));
            }
            file.write_all(&buffer[..read])
                .map_err(|_| "Failed while writing reranker download".to_string())?;
            hasher.update(&buffer[..read]);
            downloaded_bytes = downloaded_bytes.saturating_add(read as u64);
            let pct =
                ((downloaded_bytes.saturating_mul(100)) / artifact.expected_size).min(100) as u8;
            if pct > last_reported_pct && (pct >= last_reported_pct.saturating_add(5) || pct == 100)
            {
                last_reported_pct = pct;
                let _ = app_handle.emit(
                    "reranker:download_progress",
                    RerankerDownloadProgressPayload {
                        pct,
                        downloaded_bytes,
                        total_bytes: artifact.expected_size,
                        file: artifact.filename.to_string(),
                    },
                );
            }
        }
        file.flush()
            .map_err(|_| "Failed to flush reranker download".to_string())?;
        drop(file);

        if downloaded_bytes != artifact.expected_size || downloaded_bytes == 0 {
            return Err(format!(
                "Downloaded reranker artifact has an invalid size: {}",
                artifact.filename
            ));
        }
        let actual_sha256 = format!("{:x}", hasher.finalize());
        if actual_sha256 != artifact.expected_sha256 {
            return Err(format!(
                "Downloaded reranker artifact checksum mismatch: {}",
                artifact.filename
            ));
        }

        if destination.exists() {
            std::fs::remove_file(destination)
                .map_err(|_| "Failed to replace invalid reranker artifact".to_string())?;
        }
        std::fs::rename(&temporary, destination)
            .map_err(|_| "Failed to finalize reranker artifact".to_string())?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(feature = "local-ml"))]
pub fn download_local_reranker_model_files(
    _model_dir: &Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let error = "Local reranker downloads are unavailable in this build.".to_string();
    let _ = app_handle.emit(
        "reranker:download_error",
        RerankerDownloadErrorPayload {
            error: error.clone(),
        },
    );
    Err(error)
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

    // ── Rerank depth (F4) ────────────────────────────────────────────────────

    #[test]
    fn truncate_to_depth_keeps_the_rrf_head_in_order() {
        let candidates: Vec<RrfCandidate> = (0..10)
            .map(|index| {
                candidate(
                    &format!("asset-{index}"),
                    "texto".to_string(),
                    1.0 - index as f64 / 10.0,
                )
            })
            .collect();

        let truncated = truncate_to_depth(candidates, 4);

        assert_eq!(truncated.len(), 4);
        let ids: Vec<&str> = truncated
            .iter()
            .map(|c| c.record.asset_id.as_str())
            .collect();
        assert_eq!(ids, vec!["asset-0", "asset-1", "asset-2", "asset-3"]);
    }

    #[test]
    fn truncate_to_depth_is_a_noop_when_there_are_fewer_candidates_than_depth() {
        let candidates = vec![candidate("only", "texto".to_string(), 1.0)];
        assert_eq!(truncate_to_depth(candidates, 16).len(), 1);
    }

    #[test]
    fn truncate_to_depth_never_empties_the_list() {
        // Defensa: `params` ya garantiza depth >= top_k >= 1, pero una
        // profundidad 0 no debe convertir la degradación elegante en cero
        // fuentes.
        let candidates = vec![
            candidate("a", "texto".to_string(), 1.0),
            candidate("b", "texto".to_string(), 0.5),
        ];
        assert_eq!(truncate_to_depth(candidates, 0).len(), 1);
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

    #[test]
    fn local_scores_sort_descending_and_ties_keep_rrf_order() {
        let candidates = vec![
            candidate("first", "first".to_string(), 0.9),
            candidate("second", "second".to_string(), 0.8),
            candidate("third", "third".to_string(), 0.7),
            candidate("fourth", "fourth".to_string(), 0.6),
        ];
        let ranked = rank_candidates_from_logits(&candidates, &[0.0, 3.0, 3.0, -2.0])
            .expect("valid local logits");
        let ids: Vec<&str> = ranked
            .iter()
            .map(|candidate| candidate.record.asset_id.as_str())
            .collect();

        assert_eq!(ids, vec!["second", "third", "first", "fourth"]);
        assert!(ranked[0].score > ranked[2].score);
        assert_eq!(ranked[0].score, ranked[1].score);
    }

    #[test]
    fn stable_sigmoid_handles_extreme_logits() {
        assert_eq!(stable_sigmoid(1000.0), 1.0);
        assert_eq!(stable_sigmoid(-1000.0), 0.0);
        assert_eq!(stable_sigmoid(0.0), 0.5);
        assert!(stable_sigmoid(20.0).is_finite());
        assert!(stable_sigmoid(-20.0).is_finite());
    }

    #[test]
    fn local_score_count_and_values_are_validated() {
        let candidates = vec![candidate("first", "first".to_string(), 0.9)];
        assert!(rank_candidates_from_logits(&candidates, &[]).is_none());
        assert!(rank_candidates_from_logits(&candidates, &[f32::NAN]).is_none());
    }

    #[test]
    fn model_dir_resolution_is_environment_independent() {
        let root = Path::new("app-data");
        assert_eq!(
            resolve_local_reranker_model_dir_from(None, Some(root)),
            root.join("models")
                .join("rerankers")
                .join(LOCAL_RERANKER_DIR_NAME)
        );
        assert_eq!(
            resolve_local_reranker_model_dir_from(Some(Path::new("override")), Some(root)),
            PathBuf::from("override")
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn output_requires_one_finite_logit_per_candidate() {
        let vector = ndarray::Array1::from_vec(vec![1.0_f32, -1.0]).into_dyn();
        assert_eq!(
            validate_logits_output(vector.view(), 2).expect("vector logits"),
            vec![1.0, -1.0]
        );
        let matrix = ndarray::Array2::from_shape_vec((2, 1), vec![1.0_f32, -1.0])
            .expect("matrix")
            .into_dyn();
        assert!(validate_logits_output(matrix.view(), 2).is_ok());
        let wrong_shape = ndarray::Array2::zeros((1, 2)).into_dyn();
        assert!(validate_logits_output(wrong_shape.view(), 2).is_err());
        let non_finite = ndarray::Array1::from_vec(vec![f32::INFINITY]).into_dyn();
        assert!(validate_logits_output(non_finite.view(), 1).is_err());
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn artifact_validation_rejects_wrong_size_and_checksum() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("fixture.bin");
        let artifact = ArtifactSpec {
            filename: "fixture.bin",
            source_path: "fixture.bin",
            expected_size: 3,
            expected_sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };

        std::fs::write(&path, b"ab").expect("short fixture");
        assert!(validate_artifact(&path, artifact).is_err());
        std::fs::write(&path, b"abd").expect("wrong checksum fixture");
        assert!(validate_artifact(&path, artifact).is_err());
        std::fs::write(&path, b"abc").expect("valid fixture");
        assert!(validate_artifact(&path, artifact).is_ok());
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn model_info_rejects_tampered_temp_assets() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join(LOCAL_RERANKER_MODEL_FILE), b"bad").expect("model fixture");
        std::fs::write(temp.path().join(LOCAL_RERANKER_TOKENIZER_FILE), b"bad")
            .expect("tokenizer fixture");

        let info = get_local_reranker_model_info(temp.path().to_path_buf());

        assert!(!info.available);
        assert_eq!(info.required_files.len(), 2);
        assert!(info.required_files.iter().all(|file| !file.valid));
        assert!(info
            .required_files
            .iter()
            .all(|file| file.actual_size_bytes == Some(3)));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn missing_local_model_preserves_complete_rrf_ranking() {
        let temp = tempfile::tempdir().expect("tempdir");
        let candidates = vec![
            candidate("first", "first".to_string(), 0.6),
            candidate("second", "second".to_string(), 0.5),
            candidate("third", "third".to_string(), 0.4),
        ];

        // Profundidad por encima de la cantidad de candidatos: el fallback debe
        // preservar la lista RRF completa.
        let ranked = rerank_candidates_local("query", candidates, temp.path(), 16);

        let ranked_contract: Vec<(&str, f64)> = ranked
            .iter()
            .map(|candidate| (candidate.record.asset_id.as_str(), candidate.score))
            .collect();
        assert_eq!(
            ranked_contract,
            vec![("first", 0.6), ("second", 0.5), ("third", 0.4)]
        );
    }

    /// F4: sin modelo local el reranking degrada a orden RRF, pero la
    /// PROFUNDIDAD se aplica igual — es lo que iguala a Pro con Lite.
    #[cfg(feature = "local-ml")]
    #[test]
    fn local_rerank_applies_depth_even_when_it_degrades_to_rrf_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        let candidates = vec![
            candidate("first", "first".to_string(), 0.6),
            candidate("second", "second".to_string(), 0.5),
            candidate("third", "third".to_string(), 0.4),
        ];

        let ranked = rerank_candidates_local("query", candidates, temp.path(), 2);

        let ids: Vec<&str> = ranked
            .iter()
            .map(|candidate| candidate.record.asset_id.as_str())
            .collect();
        assert_eq!(ids, vec!["first", "second"]);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn model_download_source_is_pinned_to_verified_revision() {
        assert!(!LOCAL_RERANKER_RESOLVE_BASE_URL.ends_with("/main"));
        assert!(
            LOCAL_RERANKER_RESOLVE_BASE_URL.ends_with("/6f5ff65298512715a1e669753bc754d2bc8f367b")
        );
    }

    #[cfg(not(feature = "local-ml"))]
    #[test]
    fn lite_model_info_reports_local_reranker_unavailable() {
        let info = get_local_reranker_model_info(PathBuf::from("unused"));
        assert!(!info.available);
        assert!(!info.can_auto_download);
        assert!(info.required_files.is_empty());
    }

    #[cfg(not(feature = "local-ml"))]
    fn conn_with_settings(pairs: &[(&str, &str)]) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("app_settings schema creation failed");
        for (key, value) in pairs {
            conn.execute(
                "INSERT INTO app_settings(key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .expect("setting insert failed");
        }
        conn
    }

    #[cfg(not(feature = "local-ml"))]
    #[test]
    fn resolve_reranker_model_defaults_when_setting_is_unset() {
        let conn = conn_with_settings(&[]);
        assert_eq!(
            resolve_reranker_model(&conn),
            DEFAULT_OPENROUTER_RERANK_MODEL
        );
    }

    #[cfg(not(feature = "local-ml"))]
    #[test]
    fn resolve_reranker_model_uses_the_configured_override() {
        let conn = conn_with_settings(&[("rag_reranker_model", "  cohere/rerank-3.5  ")]);
        assert_eq!(resolve_reranker_model(&conn), "cohere/rerank-3.5");
    }

    #[cfg(not(feature = "local-ml"))]
    #[test]
    fn resolve_reranker_model_blank_override_falls_back_to_default() {
        let conn = conn_with_settings(&[("rag_reranker_model", "   ")]);
        assert_eq!(
            resolve_reranker_model(&conn),
            DEFAULT_OPENROUTER_RERANK_MODEL
        );
    }
}
