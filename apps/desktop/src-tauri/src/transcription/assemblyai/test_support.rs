//! Test-only helpers for the AssemblyAI module. Builds a fake
//! [`CreateTransporter`] that records every payload it sees and
//! returns canned [`CreateTranscriptOutcome`]s.
//!
//! Kept behind `#[cfg(test)]` at the call sites in the parent module
//! (the `pub use` re-export is conditional on the same flag), so this
//! file never ships in the binary.

use std::future::Future;
use std::sync::Mutex;

use super::config::AssemblyAiConfig;
use super::transporter::{CreateTranscriptOutcome, CreateTranscriptSend, CreateTransporter};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RecordedAttempt {
    pub url: String,
    pub api_key: String,
    pub payload: serde_json::Value,
}

/// Always returns the same outcome on every call. Useful for
/// "first-shot success" / "non-retryable failure" tests.
pub struct MockCreateTransporter {
    outcome: CreateTranscriptSend,
    attempts: Mutex<Vec<RecordedAttempt>>,
}

impl MockCreateTransporter {
    pub fn single(outcome: CreateTranscriptOutcome) -> Self {
        Self {
            outcome: CreateTranscriptSend::Response(outcome),
            attempts: Mutex::new(Vec::new()),
        }
    }

    pub fn single_send(outcome: CreateTranscriptSend) -> Self {
        Self {
            outcome,
            attempts: Mutex::new(Vec::new()),
        }
    }

    pub fn take_attempts(&self) -> Vec<RecordedAttempt> {
        std::mem::take(&mut *self.attempts.lock().expect("attempts lock"))
    }
}

impl CreateTransporter for MockCreateTransporter {
    fn post_create_transcript<'a>(
        &'a self,
        url: &'a str,
        api_key: &'a str,
        payload: serde_json::Value,
    ) -> impl Future<Output = CreateTranscriptSend> + Send + 'a {
        self.attempts
            .lock()
            .expect("attempts lock")
            .push(RecordedAttempt {
                url: url.to_string(),
                api_key: api_key.to_string(),
                payload,
            });
        async move { self.outcome.clone() }
    }
}

/// Returns the next outcome from `script` on each call. Panics if the
/// script runs out — that's a test bug, not a production failure mode.
pub struct ScriptedCreateTransporter {
    script: Mutex<Vec<CreateTranscriptOutcome>>,
    attempts: Mutex<Vec<RecordedAttempt>>,
}

impl ScriptedCreateTransporter {
    pub fn new(script: Vec<CreateTranscriptOutcome>) -> Self {
        Self {
            script: Mutex::new(script),
            attempts: Mutex::new(Vec::new()),
        }
    }

    pub fn take_attempts(&self) -> Vec<RecordedAttempt> {
        std::mem::take(&mut *self.attempts.lock().expect("attempts lock"))
    }
}

impl CreateTransporter for ScriptedCreateTransporter {
    fn post_create_transcript<'a>(
        &'a self,
        url: &'a str,
        api_key: &'a str,
        payload: serde_json::Value,
    ) -> impl Future<Output = CreateTranscriptSend> + Send + 'a {
        self.attempts
            .lock()
            .expect("attempts lock")
            .push(RecordedAttempt {
                url: url.to_string(),
                api_key: api_key.to_string(),
                payload,
            });
        let next = self.script.lock().expect("script lock").remove(0);
        async move { CreateTranscriptSend::Response(next) }
    }
}

/// Suppress the unused-import warning when only some of the test
/// helpers are referenced.
#[allow(dead_code)]
fn _config_marker(_: &AssemblyAiConfig) {}

/// Mirror `http::Response::builder().status(N).body(Vec<u8>) -> reqwest::Response::from(http_resp)` from the
/// sync test_support, but for `reqwest::Response` we only need this when callers want to
/// assert on the raw `Response`. We don't need it here, but expose the
/// helper for parity / future use.
#[allow(dead_code)]
pub fn _mock_response(status: u16, body: &[u8]) -> reqwest::Response {
    let http_resp = http::Response::builder()
        .status(status)
        .body(body.to_vec())
        .expect("mock response builder");
    reqwest::Response::from(http_resp)
}
