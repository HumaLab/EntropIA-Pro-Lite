//! Transport abstraction for the AssemblyAI create-transcript POST.
//!
//! Production code uses [`ReqwestCreateTransporter`]; tests inject a
//! mock that returns canned [`CreateTranscriptOutcome`]s. The transport
//! is intentionally minimal — it covers only `POST /v2/transcript`.
//! Audio uploads and transcript polling stay on the regular reqwest
//! client owned by [`super::AssemblyAiClient`] and aren't exercised
//! here.
//!
//! ## Why an explicit transport trait
//!
//! We need to test the *compatibility fallback* (try the next schema
//! when AssemblyAI returns 400 with a compatibility message, give up
//! immediately otherwise) without spinning up a real HTTP server.
//! Threading a mock through `reqwest::Client` directly would need
//! either a private `MockServer` runtime or `wiremock` / `httpmock` —
//! both add test-only deps that aren't worth it for a single function.
//! A one-method trait over `serde_json::Value` is the minimum seam
//! that lets us assert the fallback behaviour.

use std::future::Future;

/// Result of a single POST attempt: the raw status code and body.
/// Tests construct this directly; production code gets it back from
/// reqwest.
#[derive(Debug, Clone)]
pub struct CreateTranscriptOutcome {
    pub status: u16,
    pub body: String,
}

/// Result of a `post_create_transcript` call. Distinguishes a real
/// server response (which may be a 200 or a 400 we should fall back on)
/// from a transport-level failure (timeout, DNS, TLS, connection
/// reset) — the latter MUST NOT trigger another attempt, per the
/// dictation contract.
#[derive(Debug, Clone)]
pub enum CreateTranscriptSend {
    Response(CreateTranscriptOutcome),
    TransportError(String),
}

pub trait CreateTransporter: Sync {
    fn post_create_transcript<'a>(
        &'a self,
        url: &'a str,
        api_key: &'a str,
        payload: serde_json::Value,
    ) -> impl Future<Output = CreateTranscriptSend> + Send + 'a;
}

/// Production transport backed by `reqwest`.
pub struct ReqwestCreateTransporter {
    client: reqwest::Client,
}

impl ReqwestCreateTransporter {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl CreateTransporter for ReqwestCreateTransporter {
    fn post_create_transcript<'a>(
        &'a self,
        url: &'a str,
        api_key: &'a str,
        payload: serde_json::Value,
    ) -> impl Future<Output = CreateTranscriptSend> + Send + 'a {
        async move {
            match self
                .client
                .post(url)
                .header("Authorization", api_key)
                .json(&payload)
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body = response.text().await.unwrap_or_default();
                    CreateTranscriptSend::Response(CreateTranscriptOutcome { status, body })
                }
                Err(error) => CreateTranscriptSend::TransportError(error.to_string()),
            }
        }
    }
}
