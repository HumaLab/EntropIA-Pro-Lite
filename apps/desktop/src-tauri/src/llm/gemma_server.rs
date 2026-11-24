//! The local Gemma model on the GPU, served by the managed `llama-server`
//! that GLM-OCR already uses. Callers send the prompt already in the Gemma
//! turn format (`prompt::gemma_wrap`), the same text the in-process CPU
//! engine generates from, through the raw `/completion` endpoint: the chat
//! endpoint would apply a second template.

use std::time::Duration;

/// A local answer on a slow GPU can read a long evidence prompt for minutes.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(900);

/// One raw completion request.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Completion<'a> {
    /// The full prompt, Gemma turn format included.
    pub prompt: &'a str,
    pub max_tokens: i32,
    /// `0.0` is greedy, as in the in-process engine.
    pub temperature: f32,
    pub seed: u32,
}

/// The generated text and whether it stopped at the token budget instead of
/// an end-of-generation token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompletionOutput {
    pub text: String,
    pub hit_budget: bool,
}

/// The `/completion` body. The sampler matches the in-process engine: a
/// temperature then a seeded draw (greedy at `0.0`), so every other llama.cpp
/// sampler is switched off.
pub(crate) fn completion_body(request: &Completion<'_>) -> serde_json::Value {
    serde_json::json!({
        "prompt": request.prompt,
        "n_predict": request.max_tokens,
        "temperature": request.temperature,
        "seed": request.seed,
        "top_k": 0,
        "top_p": 1.0,
        "min_p": 0.0,
        "typical_p": 1.0,
        "repeat_penalty": 1.0,
        "cache_prompt": true,
        "stream": false,
    })
}

pub(crate) fn parse_completion(payload: &serde_json::Value) -> Result<CompletionOutput, String> {
    let text = payload
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Local Gemma server answered without content".to_string())?;
    Ok(CompletionOutput {
        text: text.to_string(),
        hit_budget: payload.get("stop_type").and_then(serde_json::Value::as_str) == Some("limit"),
    })
}

pub(crate) fn parse_token_count(payload: &serde_json::Value) -> Result<usize, String> {
    payload
        .get("tokens")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .ok_or_else(|| "Local Gemma server answered /tokenize without tokens".to_string())
}

/// Blocking `/completion` call against the server at `base_url`.
pub(crate) fn complete(
    base_url: &str,
    request: &Completion<'_>,
) -> Result<CompletionOutput, String> {
    let payload = post_json(base_url, "/completion", &completion_body(request))?;
    parse_completion(&payload)
}

/// Prompt tokens as the server counts them, BOS included.
pub(crate) fn token_count(base_url: &str, text: &str) -> Result<usize, String> {
    let body = serde_json::json!({ "content": text, "add_special": true });
    parse_token_count(&post_json(base_url, "/tokenize", &body)?)
}

fn post_json(
    base_url: &str,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to create the local Gemma client: {e}"))?;
    let response = client
        .post(format!("{base_url}{path}"))
        .json(body)
        .send()
        .map_err(|e| format!("Local Gemma request {path} failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail: String = response
            .text()
            .unwrap_or_default()
            .chars()
            .take(500)
            .collect();
        return Err(format!(
            "Local Gemma request {path} failed with HTTP {status}: {detail}"
        ));
    }
    response
        .json()
        .map_err(|e| format!("Local Gemma request {path} returned invalid JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_sends_the_raw_prompt_with_the_engine_sampler() {
        let body = completion_body(&Completion {
            prompt: "<start_of_turn>user\nHola<end_of_turn>\n<start_of_turn>model\n",
            max_tokens: 128,
            temperature: 0.0,
            seed: 1234,
        });
        assert_eq!(
            body["prompt"],
            "<start_of_turn>user\nHola<end_of_turn>\n<start_of_turn>model\n"
        );
        assert_eq!(body["n_predict"], 128);
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["seed"], 1234);
        // Only temperature and the seeded draw: every other sampler is off.
        assert_eq!(body["top_k"], 0);
        assert_eq!(body["top_p"], 1.0);
        assert_eq!(body["min_p"], 0.0);
        assert_eq!(body["repeat_penalty"], 1.0);
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn a_completion_reports_whether_it_stopped_at_the_budget() {
        let cut = parse_completion(&serde_json::json!({ "content": "Hola", "stop_type": "limit" }))
            .unwrap();
        assert_eq!(
            cut,
            CompletionOutput {
                text: "Hola".into(),
                hit_budget: true
            }
        );
        let done = parse_completion(&serde_json::json!({ "content": "Hola.", "stop_type": "eos" }))
            .unwrap();
        assert!(!done.hit_budget);
    }

    #[test]
    fn a_completion_without_content_is_an_error() {
        assert!(parse_completion(&serde_json::json!({ "error": "x" })).is_err());
    }

    #[test]
    fn the_token_count_is_the_length_of_the_token_list() {
        assert_eq!(
            parse_token_count(&serde_json::json!({ "tokens": [2, 106, 1645] })).unwrap(),
            3
        );
        assert!(parse_token_count(&serde_json::json!({})).is_err());
    }
}
