use std::num::NonZeroU32;
use std::path::PathBuf;
use std::pin::pin;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

/// Tokens evaluated per prefill decode call. llama.cpp processes the prompt in
/// batches of this size, so a 128K-token prompt no longer requires a
/// 128K-token batch allocation (which the old single-shot prefill did).
const PREFILL_BATCH_TOKENS: usize = 512;

#[derive(Debug, PartialEq, Eq)]
struct ResearchChatPromptPlan<'a> {
    role: &'static str,
    content: &'a str,
    add_generation_prompt: bool,
    add_bos: AddBos,
}

fn research_chat_prompt_plan(instruction: &str) -> ResearchChatPromptPlan<'_> {
    ResearchChatPromptPlan {
        role: "user",
        content: instruction,
        add_generation_prompt: true,
        add_bos: AddBos::Never,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SamplingPlan {
    Greedy,
    Temperature { temperature: f32, seed: u32 },
}

fn sampling_plan(temperature: f32, seed: u32) -> Result<SamplingPlan, String> {
    if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
        return Err(format!(
            "RAG temperature must be finite and between 0.0 and 2.0, got {temperature}"
        ));
    }
    if temperature == 0.0 {
        Ok(SamplingPlan::Greedy)
    } else {
        Ok(SamplingPlan::Temperature { temperature, seed })
    }
}

fn sampler_from_plan(plan: SamplingPlan) -> LlamaSampler {
    match plan {
        SamplingPlan::Greedy => LlamaSampler::greedy(),
        SamplingPlan::Temperature { temperature, seed } => {
            LlamaSampler::chain_simple([LlamaSampler::temp(temperature), LlamaSampler::dist(seed)])
        }
    }
}

/// Half-open `[start, end)` ranges covering `total` tokens in chunks of at
/// most `batch`. Empty input (or a zero batch) yields no ranges.
fn prefill_chunk_ranges(total: usize, batch: usize) -> Vec<(usize, usize)> {
    if total == 0 || batch == 0 {
        return Vec::new();
    }
    (0..total)
        .step_by(batch)
        .map(|start| (start, (start + batch).min(total)))
        .collect()
}

/// Configuration for the LLM engine.
pub struct LlmConfig {
    pub model_path: PathBuf,
    pub n_ctx: u32,
    pub n_threads: Option<i32>,
    pub seed: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::new(),
            n_ctx: 4096,
            n_threads: None,
            seed: 1234,
        }
    }
}

/// Wraps llama.cpp via llama-cpp-2 crate. Loads a GGUF model once and runs
/// inference on demand. Text-only only.
pub struct LlmEngine {
    backend: LlamaBackend,
    model: LlamaModel,
    config: LlmConfig,
}

impl LlmEngine {
    fn preview_for_log(text: &str, max_chars: usize) -> String {
        let sanitized = text.replace('\r', "\\r").replace('\n', "\\n");
        let mut chars = sanitized.chars();
        let preview: String = chars.by_ref().take(max_chars).collect();
        if chars.next().is_some() {
            format!("{preview}…")
        } else {
            preview
        }
    }

    pub(crate) fn sanitize_text_output(raw: &str) -> String {
        let mut text = raw.trim();

        for marker in ["<end_of_turn>", "<start_of_turn>", "<eos>"] {
            if let Some(idx) = text.find(marker) {
                text = &text[..idx];
            }
        }

        text = text.trim();

        if text.starts_with("```") {
            let without_opening = text
                .strip_prefix("```")
                .unwrap_or(text)
                .trim_start_matches("text")
                .trim_start_matches("txt")
                .trim_start_matches("markdown")
                .trim_start_matches("json")
                .trim_start_matches("JSON")
                .trim();
            text = without_opening
                .strip_suffix("```")
                .unwrap_or(without_opening)
                .trim();
        }

        let lower = text.to_lowercase();
        for prefix in [
            "texto corregido:",
            "texto corregido y unificado:",
            "corrección ocr:",
            "correccion ocr:",
            "resultado corregido:",
        ] {
            if lower.starts_with(prefix) {
                text = text[prefix.len()..].trim();
                break;
            }
        }

        if text.len() >= 2 {
            let first = text.chars().next().unwrap_or_default();
            let last = text.chars().last().unwrap_or_default();
            let quoted = matches!((first, last), ('"', '"') | ('\'', '\''));
            if quoted {
                let inner = &text[1..text.len() - 1];
                if inner.contains('\n') || inner.len() > 80 {
                    text = inner.trim();
                }
            }
        }

        text.trim().to_string()
    }

    fn sanitize_json_array_output(raw: &str) -> String {
        let text = Self::sanitize_text_output(raw);

        if let Some(start) = text.find('[') {
            if let Some(end_rel) = text[start..].rfind(']') {
                return text[start..=start + end_rel].trim().to_string();
            }
        }

        text
    }

    /// Load a GGUF model from disk in text-only mode.
    pub fn init(config: LlmConfig) -> Result<Self, String> {
        if !config.model_path.exists() {
            return Err(format!(
                "Model file not found: {}",
                config.model_path.display()
            ));
        }

        let mut backend =
            LlamaBackend::init().map_err(|e| format!("Failed to init llama backend: {e}"))?;

        // Silence verbose llama.cpp / ggml native logs (tensor loading, KV cache,
        // reserve spam, etc.). We keep our own `[llm-local] ...` diagnostics.
        backend.void_logs();

        let model_params = pin!(LlamaModelParams::default());

        let model = LlamaModel::load_from_file(&backend, &config.model_path, &model_params)
            .map_err(|e| format!("Failed to load model {}: {e}", config.model_path.display()))?;

        eprintln!(
            "[llm-local] Model loaded: {} (n_ctx={})",
            config.model_path.display(),
            config.n_ctx
        );
        eprintln!("[llm-local] Running in text-only mode");

        Ok(Self {
            backend,
            model,
            config,
        })
    }

    /// Returns the configured context window size.
    pub fn n_ctx(&self) -> u32 {
        self.config.n_ctx
    }

    /// Run raw text generation with the given prompt and an explicit context
    /// window. Returns the generated text exactly as decoded from llama.cpp
    /// (minus surrounding trim only). `n_ctx` is per-request: the cached engine
    /// keeps the model loaded, but each generation builds its own context, so
    /// evidence-heavy flows (RAG) can request a much larger window than the
    /// small default used by OCR/NER flows.
    fn generate_raw(
        &self,
        prompt: &str,
        max_tokens: i32,
        n_ctx: u32,
        log_prefix: &str,
        add_bos: AddBos,
        sampling: SamplingPlan,
    ) -> Result<String, String> {
        let tokens = self
            .model
            .str_to_token(prompt, add_bos)
            .map_err(|e| format!("Failed to tokenize prompt: {e}"))?;

        let n_prompt = tokens.len() as i32;
        let prompt_chars = prompt.chars().count();

        let available = n_ctx as i32 - n_prompt;
        if available <= 0 {
            return Err(format!(
                "Prompt ({} tokens) exceeds context window ({}). \
                 Truncate input text before generating.",
                n_prompt, n_ctx
            ));
        }
        let effective_max_tokens = max_tokens.min(available);
        if effective_max_tokens < max_tokens {
            eprintln!(
                "{log_prefix} Reducing max_tokens from {} to {} \
                 (prompt={}/n_ctx={})",
                max_tokens, effective_max_tokens, n_prompt, n_ctx
            );
        }

        let mut ctx_params = LlamaContextParams::default()
            .with_n_ctx(Some(NonZeroU32::new(n_ctx.max(1)).unwrap()))
            .with_n_batch(PREFILL_BATCH_TOKENS as u32)
            .with_n_ubatch(PREFILL_BATCH_TOKENS as u32);

        if let Some(threads) = self.config.n_threads {
            ctx_params = ctx_params.with_n_threads(threads);
            ctx_params = ctx_params.with_n_threads_batch(threads);
        }

        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| format!("Failed to create context: {e}"))?;

        let ctx_n_batch = ctx.n_batch();
        let ctx_n_ubatch = ctx.n_ubatch();
        let ctx_n_ctx = ctx.n_ctx();

        eprintln!(
            "{log_prefix} generate request: prompt_chars={}, prompt_tokens={}, requested_max_tokens={}, effective_max_tokens={}, n_ctx={}, n_batch={}, n_ubatch={}",
            prompt_chars, n_prompt, max_tokens, effective_max_tokens, ctx_n_ctx, ctx_n_batch, ctx_n_ubatch
        );

        let n_len = n_prompt + effective_max_tokens;

        // Segmented prefill: feed the prompt in bounded chunks so prompts far
        // larger than the llama batch size (e.g. ~116K tokens inside a 131K
        // context) decode without a giant single batch. Only the final token
        // of the whole prompt requests logits.
        let total = tokens.len();
        let mut batch = LlamaBatch::new(PREFILL_BATCH_TOKENS.min(total).max(1), 1);
        for (start, end) in prefill_chunk_ranges(total, PREFILL_BATCH_TOKENS) {
            batch.clear();
            let is_last_chunk = end == total;
            for pos in start..end {
                let wants_logits = is_last_chunk && pos == total - 1;
                batch
                    .add(tokens[pos], pos as i32, &[0], wants_logits)
                    .map_err(|e| format!("Failed to add token to batch: {e}"))?;
            }
            ctx.decode(&mut batch)
                .map_err(|e| format!("Failed to decode prompt: {e}"))?;
        }

        let mut sampler = sampler_from_plan(sampling);

        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let mut output = String::new();
        let mut n_cur = total as i32;

        while n_cur <= n_len {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);

            if self.model.is_eog_token(token) {
                break;
            }

            let piece = self
                .model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|e| format!("Failed to decode token: {e}"))?;
            output.push_str(&piece);

            batch.clear();
            batch
                .add(token, n_cur, &[0], true)
                .map_err(|e| format!("Failed to add token to batch: {e}"))?;

            ctx.decode(&mut batch)
                .map_err(|e| format!("Failed to decode: {e}"))?;

            n_cur += 1;
        }

        Ok(output.trim().to_string())
    }

    /// Run text generation with the given prompt. Returns the sanitized generated text
    /// (excluding the prompt). `max_tokens` limits the output length.
    pub fn generate(
        &self,
        prompt: &str,
        max_tokens: i32,
        log_prefix: &str,
    ) -> Result<String, String> {
        let raw = self.generate_raw(
            prompt,
            max_tokens,
            self.config.n_ctx,
            log_prefix,
            AddBos::Always,
            SamplingPlan::Greedy,
        )?;
        Ok(Self::sanitize_text_output(&raw))
    }

    /// Same as [`generate`](Self::generate) but with an explicit per-request
    /// context window instead of the engine default. Used by the RAG chat,
    /// whose evidence budget needs the full 131K-token window.
    pub fn generate_chat_with_ctx(
        &self,
        instruction: &str,
        max_tokens: i32,
        n_ctx: u32,
        temperature: f32,
        log_prefix: &str,
    ) -> Result<String, String> {
        let plan = research_chat_prompt_plan(instruction);
        let template = self
            .model
            .chat_template(None)
            .map_err(|error| format!("Failed to load model chat template: {error}"))?;
        let message = LlamaChatMessage::new(plan.role.to_string(), plan.content.to_string())
            .map_err(|error| format!("Failed to create Research Chat message: {error}"))?;
        let prompt = self
            .model
            .apply_chat_template(&template, &[message], plan.add_generation_prompt)
            .map_err(|error| format!("Failed to apply model chat template: {error}"))?;
        let sampling = sampling_plan(temperature, self.config.seed)?;
        let raw = self.generate_raw(
            &prompt,
            max_tokens,
            n_ctx,
            log_prefix,
            plan.add_bos,
            sampling,
        )?;
        Ok(Self::sanitize_text_output(&raw))
    }

    /// Generate OCR-corrected text and log raw vs sanitized output when the
    /// sanitization pass materially changes the model response.
    pub fn generate_ocr_correction(
        &self,
        prompt: &str,
        max_tokens: i32,
        log_prefix: &str,
    ) -> Result<String, String> {
        let raw = self.generate_raw(
            prompt,
            max_tokens,
            self.config.n_ctx,
            log_prefix,
            AddBos::Always,
            SamplingPlan::Greedy,
        )?;
        let sanitized = Self::sanitize_text_output(&raw);

        if raw.trim() != sanitized {
            eprintln!(
                "[llm-local][correction] sanitized model output: raw_len={}, sanitized_len={}, raw_preview=\"{}\", sanitized_preview=\"{}\"",
                raw.chars().count(),
                sanitized.chars().count(),
                Self::preview_for_log(&raw, 220),
                Self::preview_for_log(&sanitized, 220),
            );
        }

        Ok(sanitized)
    }

    /// Generate semantic triples as JSON.
    ///
    /// IMPORTANT: this intentionally avoids llama.cpp GBNF grammars.
    /// With Gemma 4 + llama.cpp 0.1.145, constrained decoding can abort the
    /// whole process with `GGML_ASSERT(!stacks.empty()) failed` inside
    /// `llama-grammar.cpp`. We prefer unconstrained generation plus robust
    /// JSON extraction/parsing over a hard process crash.
    pub fn generate_triples(
        &self,
        prompt: &str,
        max_tokens: i32,
        log_prefix: &str,
    ) -> Result<String, String> {
        let raw = self.generate_raw(
            prompt,
            max_tokens,
            self.config.n_ctx,
            log_prefix,
            AddBos::Always,
            SamplingPlan::Greedy,
        )?;
        let sanitized = Self::sanitize_json_array_output(&raw);

        if raw.trim() != sanitized {
            eprintln!(
                "[llm-local][triples] sanitized model output: raw_len={}, sanitized_len={}, raw_preview=\"{}\", sanitized_preview=\"{}\"",
                raw.chars().count(),
                sanitized.chars().count(),
                Self::preview_for_log(&raw, 220),
                Self::preview_for_log(&sanitized, 220),
            );
        }

        Ok(sanitized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefill_chunk_ranges_empty_inputs_yield_no_ranges() {
        assert!(prefill_chunk_ranges(0, PREFILL_BATCH_TOKENS).is_empty());
        assert!(prefill_chunk_ranges(100, 0).is_empty());
    }

    #[test]
    fn prefill_chunk_ranges_single_chunk_when_prompt_fits_batch() {
        assert_eq!(prefill_chunk_ranges(1, PREFILL_BATCH_TOKENS), vec![(0, 1)]);
        assert_eq!(
            prefill_chunk_ranges(PREFILL_BATCH_TOKENS, PREFILL_BATCH_TOKENS),
            vec![(0, PREFILL_BATCH_TOKENS)]
        );
    }

    #[test]
    fn prefill_chunk_ranges_split_remainder_into_final_short_chunk() {
        let ranges = prefill_chunk_ranges(PREFILL_BATCH_TOKENS + 1, PREFILL_BATCH_TOKENS);
        assert_eq!(
            ranges,
            vec![
                (0, PREFILL_BATCH_TOKENS),
                (PREFILL_BATCH_TOKENS, PREFILL_BATCH_TOKENS + 1)
            ]
        );
    }

    #[test]
    fn prefill_chunk_ranges_cover_large_prompt_without_gaps_or_overlap() {
        let total = 116_736;
        let ranges = prefill_chunk_ranges(total, PREFILL_BATCH_TOKENS);

        assert_eq!(ranges.first().copied(), Some((0, PREFILL_BATCH_TOKENS)));
        assert_eq!(ranges.last().map(|(_, end)| *end), Some(total));
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "chunks must be contiguous");
        }
        let covered: usize = ranges.iter().map(|(start, end)| end - start).sum();
        assert_eq!(covered, total);
        assert!(ranges
            .iter()
            .all(|(start, end)| end - start <= PREFILL_BATCH_TOKENS));
    }

    #[test]
    fn research_chat_plan_uses_model_template_without_adding_a_second_bos() {
        let plan = research_chat_prompt_plan("raw RAG instruction");

        assert_eq!(plan.role, "user");
        assert_eq!(plan.content, "raw RAG instruction");
        assert!(plan.add_generation_prompt);
        assert_eq!(plan.add_bos, AddBos::Never);
    }

    #[test]
    fn research_chat_sampling_plan_honors_validated_rag_temperature() {
        assert_eq!(
            sampling_plan(0.0, 17),
            Ok(SamplingPlan::Greedy),
            "zero temperature must be deterministic"
        );
        assert_eq!(
            sampling_plan(0.7, 17),
            Ok(SamplingPlan::Temperature {
                temperature: 0.7,
                seed: 17,
            })
        );
        assert!(sampling_plan(-0.1, 17).is_err());
        assert!(sampling_plan(2.1, 17).is_err());
        assert!(sampling_plan(f32::NAN, 17).is_err());
    }
}
