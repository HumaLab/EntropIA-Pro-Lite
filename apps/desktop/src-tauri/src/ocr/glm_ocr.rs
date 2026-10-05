use serde::{Deserialize, Serialize};

const GLM_OCR_API_URL: &str = "https://api.z.ai/api/paas/v4/layout_parsing";
const GLM_OCR_TEST_IMAGE_URL: &str = "https://cdn.bigmodel.cn/static/logo/introduction.png";

#[derive(Deserialize)]
struct GlmOcrApiErrorEnvelope {
    error: Option<GlmOcrApiError>,
    msg: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct GlmOcrApiError {
    code: Option<String>,
    message: Option<String>,
}

#[derive(Serialize)]
struct LayoutParsingRequest<'a> {
    model: &'static str,
    file: &'a str,
    /// First page to parse when `file` is a PDF (z.ai `start_page_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    start_page_id: Option<u32>,
    /// Last page to parse when `file` is a PDF (z.ai `end_page_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    end_page_id: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlmOcrResponse {
    #[allow(dead_code)]
    pub id: Option<String>,
    #[allow(dead_code)]
    pub created: Option<i64>,
    #[allow(dead_code)]
    pub model: Option<String>,
    #[serde(default)]
    pub md_results: String,
    #[serde(default)]
    pub layout_details: Vec<Vec<GlmOcrLayoutDetail>>,
    pub data_info: Option<GlmOcrDataInfo>,
    #[allow(dead_code)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlmOcrLayoutDetail {
    pub index: Option<i32>,
    pub label: Option<String>,
    #[serde(default)]
    pub bbox_2d: Vec<f32>,
    pub content: Option<String>,
    pub height: Option<u32>,
    pub width: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlmOcrDataInfo {
    #[allow(dead_code)]
    pub num_pages: Option<u32>,
    #[serde(default)]
    pub pages: Vec<GlmOcrPageInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlmOcrPageInfo {
    pub width: u32,
    pub height: u32,
}

pub struct GlmOcrClient {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
}

/// Per-attempt budgets for one layout_parsing call (plan-lote.md §7):
/// 15 s to connect, 180 s total. The queue persists `next_retry_at` and
/// frees the worker slot while waiting — it never sleeps holding it.
const GLM_CONNECT_TIMEOUT_SECS: u64 = 15;
const GLM_TOTAL_TIMEOUT_SECS: u64 = 180;

pub(crate) fn retry_after_ms(raw: Option<&str>, now: std::time::SystemTime) -> Option<i64> {
    let raw = raw?.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return i64::try_from(seconds.saturating_mul(1_000)).ok();
    }
    let deadline = httpdate::parse_http_date(raw).ok()?;
    let delay = deadline.duration_since(now).unwrap_or_default();
    i64::try_from(delay.as_millis()).ok()
}

impl GlmOcrClient {
    pub fn new(api_key: String) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("EntropIA-Desktop/0.1 (historical-research-app)")
            .connect_timeout(std::time::Duration::from_secs(GLM_CONNECT_TIMEOUT_SECS))
            .timeout(std::time::Duration::from_secs(GLM_TOTAL_TIMEOUT_SECS))
            .build()
            .expect("Failed to build reqwest client");

        Self {
            client,
            api_key,
            endpoint: GLM_OCR_API_URL.to_string(),
        }
    }

    /// Points the client at another layout_parsing endpoint (tests only).
    #[cfg(test)]
    pub(crate) fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint = endpoint;
        self
    }

    pub async fn test_connection(&self) -> Result<(), String> {
        let response = self
            .client
            .post(GLM_OCR_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&LayoutParsingRequest {
                model: "glm-ocr",
                file: GLM_OCR_TEST_IMAGE_URL,
                start_page_id: None,
                end_page_id: None,
            })
            .send()
            .await
            .map_err(|e| format!("GLM-OCR connection test failed: {e}"))?;

        Self::ensure_success(response).await.map(|_| ())
    }

    pub async fn parse_file(&self, file: &str) -> Result<GlmOcrResponse, String> {
        self.parse_file_pages(file, None).await
    }

    /// Parses a PDF data URL restricted to `pages` (1-based, inclusive), or
    /// the whole file when `None`. The response carries one `layout_details`
    /// entry per page of the parsed range.
    pub async fn parse_file_pages(
        &self,
        file: &str,
        pages: Option<(u32, u32)>,
    ) -> Result<GlmOcrResponse, String> {
        let response = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&LayoutParsingRequest {
                model: "glm-ocr",
                file,
                start_page_id: pages.map(|(first, _)| first),
                end_page_id: pages.map(|(_, last)| last),
            })
            .send()
            .await
            .map_err(|e| classify_glm_transport_error(&e))?;

        Self::ensure_success(response)
            .await?
            .json()
            .await
            .map_err(|e| format!("provider_error: failed to parse GLM-OCR response: {e}"))
    }

    async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, String> {
        let status = response.status();
        let retry_after = retry_after_ms(
            response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            std::time::SystemTime::now(),
        );
        let retry_suffix = retry_after
            .map(|delay| format!(" [retry_after_ms={delay}]"))
            .unwrap_or_default();
        if status.is_success() {
            return Ok(response);
        }

        let body = response.text().await.unwrap_or_default();
        let api_error = serde_json::from_str::<GlmOcrApiErrorEnvelope>(&body)
            .ok()
            .and_then(|parsed| {
                let nested = parsed.error.and_then(|err| match (err.code, err.message) {
                    (Some(code), Some(message)) => Some(format!("{code}: {message}")),
                    (_, Some(message)) => Some(message),
                    (Some(code), None) => Some(code),
                    _ => None,
                });
                nested.or(parsed.msg).or(parsed.message)
            })
            .unwrap_or_else(|| body.trim().to_string());

        // Coded prefixes drive the queue's retry policy: transient failures
        // retry with backoff, credential failures park as configuration, and
        // anything else fails the unit without looping.
        if status.as_u16() == 429 {
            return Err(format!(
                "rate_limited: GLM-OCR API error (429): {api_error}{retry_suffix}"
            ));
        }
        if status.is_server_error() {
            return Err(format!(
                "provider_5xx: GLM-OCR API error ({status}): {api_error}{retry_suffix}"
            ));
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(format!(
                "configuration: GLM-OCR rejected the API key ({status}): {api_error}"
            ));
        }
        Err(format!(
            "provider_error: GLM-OCR API error ({status}): {api_error}"
        ))
    }
}

/// Maps transport failures to the queue's retry vocabulary. Timeouts,
/// refused connections, and resets are transient; anything else (TLS,
/// builder misuse) fails the unit so a human looks at it.
fn classify_glm_transport_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return format!("timeout: GLM-OCR request timed out: {error}");
    }
    if error.is_connect() {
        return format!("connection: GLM-OCR connection failed: {error}");
    }
    format!("connection: GLM-OCR request failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One-shot HTTP server: answers the first request with `reply` and
    /// hands back the raw request it received.
    fn serve_once(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let endpoint = format!("http://{}/layout_parsing", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut raw = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                let read = stream.read(&mut chunk).expect("read");
                raw.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length: usize = text[..split]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if raw.len() >= split + 4 + length {
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).expect("write");
            String::from_utf8_lossy(&raw).to_string()
        });
        (endpoint, handle)
    }

    #[test]
    fn a_pdf_range_request_carries_the_file_and_the_page_fields() {
        let reply = r#"{"md_results":"x","layout_details":[[{"index":1,"label":"text","content":"uno"}],[{"index":1,"label":"text","content":"dos"}]],"data_info":{"num_pages":2}}"#;
        let (endpoint, server) = serve_once(reply);
        let client = GlmOcrClient::new("test-key".to_string()).with_endpoint(endpoint);

        let response = tauri::async_runtime::block_on(
            client.parse_file_pages("data:application/pdf;base64,AAAA", Some((3, 4))),
        )
        .expect("response");

        assert_eq!(response.layout_details.len(), 2);
        let raw = server.join().expect("server");
        let body = raw.split("\r\n\r\n").nth(1).expect("body");
        let json: serde_json::Value = serde_json::from_str(body).expect("json body");
        assert_eq!(json["model"], "glm-ocr");
        assert_eq!(json["file"], "data:application/pdf;base64,AAAA");
        assert_eq!(json["start_page_id"], 3);
        assert_eq!(json["end_page_id"], 4);
    }

    #[test]
    fn a_whole_file_request_sends_no_page_fields() {
        let reply = r#"{"md_results":"x","layout_details":[]}"#;
        let (endpoint, server) = serve_once(reply);
        let client = GlmOcrClient::new("test-key".to_string()).with_endpoint(endpoint);

        tauri::async_runtime::block_on(client.parse_file("data:application/pdf;base64,AAAA"))
            .expect("response");

        let raw = server.join().expect("server");
        let body = raw.split("\r\n\r\n").nth(1).expect("body");
        let json: serde_json::Value = serde_json::from_str(body).expect("json body");
        assert!(json.get("start_page_id").is_none() && json.get("end_page_id").is_none());
    }

    #[test]
    fn retry_after_supports_seconds_and_http_dates() {
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        assert_eq!(retry_after_ms(Some("12"), now), Some(12_000));
        let date = httpdate::fmt_http_date(now + std::time::Duration::from_secs(45));
        assert_eq!(retry_after_ms(Some(&date), now), Some(45_000));
        assert_eq!(retry_after_ms(Some("invalid"), now), None);
    }
}
