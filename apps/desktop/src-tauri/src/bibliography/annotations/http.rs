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
    user_id: u64,
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
            user_id,
            backoff: Cell::new(None),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/users/{}{}", self.base, self.user_id, path)
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
        match status.as_u16() {
            401 | 403 => Err(ApiError::Credentials),
            409 | 429 | 503 => Err(ApiError::RetryAfter {
                seconds: header_u64(response.headers(), "Retry-After").unwrap_or(DEFAULT_RETRY_SECONDS),
            }),
            code if status.is_server_error() => Err(ApiError::Server { code }),
            _ => Ok(response),
        }
    }

    fn get_json(&self, url: &str) -> Result<(Value, HeaderMap), ApiError> {
        let response = self.send(self.client.get(url), false)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok((Value::Null, response.headers().clone()));
        }
        if !response.status().is_success() {
            return Err(ApiError::Server { code: response.status().as_u16() });
        }
        let headers = response.headers().clone();
        let body = response.json::<Value>().map_err(|_| ApiError::Unreachable { after_send: false })?;
        Ok((body, headers))
    }
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

fn library_version(headers: &HeaderMap) -> Result<i64, ApiError> {
    header_u64(headers, "Last-Modified-Version")
        .map(|version| version as i64)
        .ok_or(ApiError::Unreachable { after_send: false })
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
                let code = entry.get("code").and_then(Value::as_u64).unwrap_or(400) as u16;
                let message = entry.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
                if code == 412 {
                    return CreateResult::Exists;
                }
                let parent_missing = message.to_ascii_lowercase().contains("parent item")
                    && message.to_ascii_lowercase().contains("not found");
                return CreateResult::Failed { code, message, parent_missing };
            }
            CreateResult::Failed { code: 0, message: "Zotero did not answer for this object".into(), parent_missing: false }
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
        let value = body
            .as_object()
            .map(|map| {
                map.iter()
                    .filter_map(|(key, version)| Some((key.clone(), version.as_i64()?)))
                    .collect()
            })
            .unwrap_or_default();
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
            items.extend(body.as_array().into_iter().flatten().filter_map(parse_item));
            match next_link(&headers) {
                Some(next) => url = next,
                None => break,
            }
        }
        Ok(Versioned { value: items, library_version: version.unwrap_or(0) })
    }

    fn deleted_since(&self, since: i64) -> Result<Versioned<Vec<String>>, ApiError> {
        let (body, headers) = self.get_json(&self.url(&format!("/deleted?since={since}")))?;
        let value = body
            .get("items")
            .and_then(Value::as_array)
            .map(|keys| keys.iter().filter_map(|key| key.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        Ok(Versioned { value, library_version: library_version(&headers)? })
    }

    fn item(&self, key: &str) -> Result<Option<ServerItem>, ApiError> {
        let (body, _) = self.get_json(&self.url(&format!("/items/{key}?includeTrashed=1&format=json")))?;
        Ok(if body.is_null() { None } else { parse_item(&body) })
    }

    fn create(&self, items: &[Data]) -> Result<Vec<CreateResult>, ApiError> {
        let body = Value::Array(items.iter().cloned().map(Value::Object).collect());
        let response = self.send(self.client.post(self.url("/items")).json(&body), true)?;
        let status = response.status();
        let header_version = header_u64(response.headers(), "Last-Modified-Version").map(|v| v as i64);
        if !status.is_success() {
            let message = response.text().unwrap_or_default();
            return Ok(items
                .iter()
                .map(|_| CreateResult::Failed {
                    code: status.as_u16(),
                    message: message.chars().take(200).collect(),
                    parent_missing: false,
                })
                .collect());
        }
        let answer = response.json::<Value>().map_err(|_| ApiError::Unreachable { after_send: true })?;
        Ok(parse_create_results(&answer, items.len(), header_version))
    }

    fn patch(&self, key: &str, version: i64, fields: &Data) -> Result<WriteResult, ApiError> {
        let request = self
            .client
            .patch(self.url(&format!("/items/{key}")))
            .header("If-Unmodified-Since-Version", version.to_string())
            .json(&Value::Object(fields.clone()));
        write_result(self.send(request, true)?)
    }

    fn delete(&self, key: &str, version: i64) -> Result<WriteResult, ApiError> {
        let request = self
            .client
            .delete(self.url(&format!("/items/{key}")))
            .header("If-Unmodified-Since-Version", version.to_string());
        write_result(self.send(request, true)?)
    }

    fn take_backoff(&self) -> Backoff {
        self.backoff.take()
    }
}

fn write_result(response: Response) -> Result<WriteResult, ApiError> {
    let status = response.status();
    Ok(match status.as_u16() {
        200..=299 => WriteResult::Done {
            version: header_u64(response.headers(), "Last-Modified-Version")
                .map(|v| v as i64)
                .ok_or(ApiError::Unreachable { after_send: true })?,
        },
        404 => WriteResult::NotFound,
        412 => WriteResult::Stale,
        code => WriteResult::Rejected {
            code,
            message: response.text().unwrap_or_default().chars().take(200).collect(),
        },
    })
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
        assert!(matches!(results[5], CreateResult::Failed { code: 0, .. }));
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
