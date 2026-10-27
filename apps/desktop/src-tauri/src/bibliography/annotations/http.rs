//! [`AnnotationApi`] over the Zotero Web API v3 (api.zotero.org).
//!
//! The key goes only into the `Zotero-API-Key` header; errors never carry the
//! URL, the key or a response body.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use serde_json::Value;

use super::api::*;
use super::wire::Data;

pub const API_BASE: &str = "https://api.zotero.org";

/// `Retry-After` when Zotero omits it on a 409/429/503.
const DEFAULT_RETRY_SECONDS: u64 = 60;

pub struct WebAnnotationApi {
    client: Client,
    base: String,
    key: String,
    /// `users/<id>`, or `groups/<id>` for the live tests' test group.
    library_path: String,
    backoff: Cell<Option<u64>>,
}

impl WebAnnotationApi {
    pub fn new(base: &str, key: &str, user_id: u64) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "Could not build the HTTP client".to_string())?;
        Ok(WebAnnotationApi {
            client,
            base: base.trim_end_matches('/').to_string(),
            key: key.trim().to_string(),
            library_path: format!("users/{user_id}"),
            backoff: Cell::new(None),
        })
    }

    /// The same API over a group library. Only the live tests use it: the
    /// Biblioteca annotates the personal library alone.
    #[cfg(test)]
    pub fn for_group(base: &str, key: &str, group_id: u64) -> Result<Self, String> {
        let mut api = Self::new(base, key, 0)?;
        api.library_path = format!("groups/{group_id}");
        Ok(api)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}{}", self.base, self.library_path, path)
    }

    fn send(&self, request: RequestBuilder, writes: bool) -> Result<Response, ApiError> {
        let response = request
            .header("Zotero-API-Version", "3")
            .header("Zotero-API-Key", &self.key)
            .send()
            .map_err(|error| ApiError::Unreachable { after_send: writes && !error.is_connect() })?;
        if let Some(seconds) = header_u64(response.headers(), "Backoff") {
            self.backoff.set(Some(seconds));
        }
        let status = response.status();
        let retry_after = header_u64(response.headers(), "Retry-After");
        match status.as_u16() {
            401 | 403 => Err(ApiError::Credentials),
            409 | 429 => Err(ApiError::RetryAfter {
                seconds: retry_after.unwrap_or(DEFAULT_RETRY_SECONDS),
            }),
            code if status.is_server_error() => Err(ApiError::Server { code, retry_after }),
            _ => Ok(response),
        }
    }

    /// Server text never carries the key back into an error or a row.
    fn redact(&self, text: &str) -> String {
        let clipped: String = text.chars().take(200).collect();
        if self.key.is_empty() {
            clipped
        } else {
            clipped.replace(&self.key, "[redacted]")
        }
    }

    fn get_json(&self, url: &str) -> Result<(Value, HeaderMap), ApiError> {
        let response = self.send(self.client.get(url), false)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok((Value::Null, response.headers().clone()));
        }
        if !response.status().is_success() {
            return Err(ApiError::Server { code: response.status().as_u16(), retry_after: None });
        }
        let headers = response.headers().clone();
        let body = response.json::<Value>().map_err(|_| unreadable())?;
        Ok((body, headers))
    }
}

/// A read whose answer cannot be used: nothing is concluded from it.
fn unreadable() -> ApiError {
    ApiError::Unreachable { after_send: false }
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

fn library_version(headers: &HeaderMap) -> Result<i64, ApiError> {
    header_u64(headers, "Last-Modified-Version")
        .map(|version| version as i64)
        .ok_or_else(unreadable)
}

/// The `rel="next"` target of a `Link` header.
pub fn next_link(headers: &HeaderMap) -> Option<String> {
    let link = headers.get("Link")?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let (target, params) = part.split_once(';')?;
        params
            .contains("rel=\"next\"")
            .then(|| target.trim().trim_start_matches('<').trim_end_matches('>').to_string())
    })
}

/// One item of a `format=json` answer: `{key, version, data}`.
pub fn parse_item(value: &Value) -> Option<ServerItem> {
    let version = value.get("version")?.as_i64()?;
    let data = value.get("data")?.as_object()?.clone();
    Some(ServerItem { version, data })
}

/// Zotero's "Parent item … not found" for a child whose parent it lacks.
pub fn mentions_missing_parent(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("parent item") && lower.contains("not found")
}

/// The per-object results of a POST, in request order.
pub fn parse_create_results(body: &Value, count: usize, header_version: Option<i64>) -> Vec<CreateResult> {
    (0..count)
        .map(|index| {
            let slot = index.to_string();
            if let Some(entry) = body.get("successful").and_then(|s| s.get(&slot)) {
                let version = entry
                    .get("version")
                    .and_then(Value::as_i64)
                    .or(header_version)
                    .unwrap_or(0);
                return CreateResult::Created { version };
            }
            if body.get("unchanged").and_then(|s| s.get(&slot)).is_some() {
                return CreateResult::Created { version: header_version.unwrap_or(0) };
            }
            if let Some(entry) = body.get("failed").and_then(|s| s.get(&slot)) {
                let Some(code) = entry.get("code").and_then(Value::as_u64) else {
                    return CreateResult::NoAnswer;
                };
                let code = code as u16;
                let message = entry.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
                if code == 412 {
                    return CreateResult::Exists;
                }
                let parent_missing = code == 404 || mentions_missing_parent(&message);
                return CreateResult::Failed { code, message, parent_missing };
            }
            CreateResult::NoAnswer
        })
        .collect()
}

impl AnnotationApi for WebAnnotationApi {
    fn versions_since(&self, since: i64) -> Result<Versioned<BTreeMap<String, i64>>, ApiError> {
        // format=versions is not paginated when no limit is passed.
        let url = self.url(&format!(
            "/items?itemType=annotation&since={since}&format=versions&includeTrashed=1"
        ));
        let (body, headers) = self.get_json(&url)?;
        let map = body.as_object().ok_or_else(unreadable)?;
        let mut value = BTreeMap::new();
        for (key, version) in map {
            value.insert(key.clone(), version.as_i64().ok_or_else(unreadable)?);
        }
        Ok(Versioned { value, library_version: library_version(&headers)? })
    }

    fn items(&self, keys: &[String]) -> Result<Versioned<Vec<ServerItem>>, ApiError> {
        let mut url = self.url(&format!(
            "/items?itemKey={}&includeTrashed=1&limit=50&format=json",
            keys.join(",")
        ));
        let mut items = Vec::new();
        let mut version = None;
        loop {
            let (body, headers) = self.get_json(&url)?;
            let page_version = library_version(&headers)?;
            if version.is_some_and(|v| v != page_version) {
                // The caller compares against the version listing.
                return Ok(Versioned { value: items, library_version: page_version });
            }
            version = Some(page_version);
            for entry in body.as_array().ok_or_else(unreadable)? {
                // A pull that skipped an item would move the cursor past it.
                let item = parse_item(entry).ok_or_else(unreadable)?;
                if item.data.get("key").and_then(Value::as_str).is_none() {
                    return Err(unreadable());
                }
                items.push(item);
            }
            match next_link(&headers) {
                Some(next) => url = next,
                None => break,
            }
        }
        Ok(Versioned { value: items, library_version: version.unwrap_or(0) })
    }

    fn deleted_since(&self, since: i64) -> Result<Versioned<Vec<String>>, ApiError> {
        let (body, headers) = self.get_json(&self.url(&format!("/deleted?since={since}")))?;
        let keys = body.get("items").and_then(Value::as_array).ok_or_else(unreadable)?;
        let mut value = Vec::with_capacity(keys.len());
        for key in keys {
            value.push(key.as_str().ok_or_else(unreadable)?.to_string());
        }
        Ok(Versioned { value, library_version: library_version(&headers)? })
    }

    fn item(&self, key: &str) -> Result<Option<ServerItem>, ApiError> {
        let (body, _) = self.get_json(&self.url(&format!("/items/{key}?includeTrashed=1&format=json")))?;
        // Only a real 404 says "not there"; an unreadable success says nothing.
        if body.is_null() {
            return Ok(None);
        }
        parse_item(&body).map(Some).ok_or_else(unreadable)
    }

    fn create(&self, items: &[Data]) -> Result<Vec<CreateResult>, ApiError> {
        let body = Value::Array(items.iter().cloned().map(Value::Object).collect());
        let response = self.send(self.client.post(self.url("/items")).json(&body), true)?;
        let status = response.status();
        let header_version = header_u64(response.headers(), "Last-Modified-Version").map(|v| v as i64);
        if !status.is_success() {
            let message = self.redact(&response.text().unwrap_or_default());
            let parent_missing = status == StatusCode::NOT_FOUND || mentions_missing_parent(&message);
            return Ok(items
                .iter()
                .map(|_| CreateResult::Failed {
                    code: status.as_u16(),
                    message: message.clone(),
                    parent_missing,
                })
                .collect());
        }
        let answer = response
            .json::<Value>()
            .map_err(|_| ApiError::Server { code: status.as_u16(), retry_after: None })?;
        Ok(parse_create_results(&answer, items.len(), header_version)
            .into_iter()
            .map(|result| match result {
                CreateResult::Failed { code, message, parent_missing } => {
                    CreateResult::Failed { code, message: self.redact(&message), parent_missing }
                }
                other => other,
            })
            .collect())
    }

    fn patch(&self, key: &str, version: i64, fields: &Data) -> Result<WriteResult, ApiError> {
        let request = self
            .client
            .patch(self.url(&format!("/items/{key}")))
            .header("If-Unmodified-Since-Version", version.to_string())
            .json(&Value::Object(fields.clone()));
        self.write_result(self.send(request, true)?)
    }

    fn delete(&self, key: &str, version: i64) -> Result<WriteResult, ApiError> {
        let request = self
            .client
            .delete(self.url(&format!("/items/{key}")))
            .header("If-Unmodified-Since-Version", version.to_string());
        self.write_result(self.send(request, true)?)
    }

    fn take_backoff(&self) -> Backoff {
        self.backoff.take()
    }
}

impl WebAnnotationApi {
    fn write_result(&self, response: Response) -> Result<WriteResult, ApiError> {
        let status = response.status();
        Ok(match status.as_u16() {
            200..=299 => WriteResult::Done {
                // A success without the new version cannot be acknowledged.
                version: header_u64(response.headers(), "Last-Modified-Version")
                    .map(|v| v as i64)
                    .ok_or(ApiError::Server { code: status.as_u16(), retry_after: None })?,
            },
            404 => WriteResult::NotFound,
            412 => WriteResult::Stale,
            code => WriteResult::Rejected {
                code,
                message: self.redact(&response.text().unwrap_or_default()),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn create_results_are_read_per_key() {
        let body = json!({
            "successful": {"0": {"key": "AAAA2222", "version": 12}},
            "unchanged": {"1": "BBBB3333"},
            "failed": {
                "2": {"key": "CCCC4444", "code": 412, "message": "Item CCCC4444 already exists"},
                "3": {"key": "DDDD5555", "code": 400, "message": "Parent item ABCD2345 not found"},
                "4": {"key": "EEEE6666", "code": 400, "message": "Invalid annotationPosition"}
            }
        });
        let results = parse_create_results(&body, 6, Some(12));
        assert_eq!(results[0], CreateResult::Created { version: 12 });
        assert_eq!(results[1], CreateResult::Created { version: 12 });
        assert_eq!(results[2], CreateResult::Exists);
        assert!(matches!(results[3], CreateResult::Failed { parent_missing: true, .. }));
        assert!(matches!(results[4], CreateResult::Failed { parent_missing: false, code: 400, .. }));
        assert_eq!(results[5], CreateResult::NoAnswer);
    }

    #[test]
    fn a_failed_entry_without_a_code_is_no_answer_and_404_is_a_missing_parent() {
        let body = json!({"failed": {
            "0": {"key": "AAAA2222", "message": "?"},
            "1": {"key": "BBBB3333", "code": 404, "message": "Not found"}
        }});
        let results = parse_create_results(&body, 2, None);
        assert_eq!(results[0], CreateResult::NoAnswer);
        assert!(matches!(results[1], CreateResult::Failed { parent_missing: true, .. }));
    }

    #[test]
    fn server_text_never_carries_the_key() {
        let api = WebAnnotationApi::new("http://127.0.0.1:1", "SeCrEtKeY123", 1).unwrap();
        assert_eq!(api.redact("bad key SeCrEtKeY123 here"), "bad key [redacted] here");
    }

    #[test]
    fn the_next_link_is_found_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Link",
            "<https://api.zotero.org/users/1/items?start=50>; rel=\"next\", <https://api.zotero.org/users/1/items?start=100>; rel=\"last\""
                .parse()
                .unwrap(),
        );
        assert_eq!(next_link(&headers).as_deref(), Some("https://api.zotero.org/users/1/items?start=50"));
        assert_eq!(next_link(&HeaderMap::new()), None);
    }

    #[test]
    fn an_unreachable_server_is_not_an_applied_write() {
        let api = WebAnnotationApi::new("http://127.0.0.1:1", "KEY", 1).unwrap();
        assert_eq!(api.item("AAAA2222"), Err(ApiError::Unreachable { after_send: false }));
        assert_eq!(api.delete("AAAA2222", 3), Err(ApiError::Unreachable { after_send: false }));
    }
}
