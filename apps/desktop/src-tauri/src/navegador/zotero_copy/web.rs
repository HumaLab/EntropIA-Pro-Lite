//! Completing an item that already exists in Zotero through the Web API.
//!
//! The local connector can only create, so an item found by address used to be
//! linked and left as it was. With a verified Web API key that can write to the
//! library, the fields the item lacks are filled in: `GET` the item, patch only
//! what is empty there and present in our source, and send it with
//! `If-Unmodified-Since-Version` so a change made in Zotero meanwhile is never
//! overwritten (`412`: read again once, try once more, then report).

use std::time::Duration;

use serde_json::{Map, Value};

use super::plan::{owned_from_zotero, plan_merge, Owned};
use crate::zotero_web::{classify_key_response, KeyCheck, API_BASE};

/// A library as the Web API addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebLibrary {
    User(u64),
    Group(String),
}

impl WebLibrary {
    fn path(&self) -> String {
        match self {
            Self::User(id) => format!("users/{id}"),
            Self::Group(id) => format!("groups/{id}"),
        }
    }
}

/// Why a Web API call did not work. Never carries the key or a response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebError {
    Unreachable,
    Rejected(u16),
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebItem {
    pub version: u64,
    pub data: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Patched {
    Done,
    /// `412`: the item changed since it was read.
    Stale,
}

/// The Web API side of a copy, behind a trait so completion runs against a
/// script and the real thing against a canned local HTTP server.
pub trait WebPort: Send + Sync {
    fn key_info(&self, key: &str) -> Result<KeyCheck, WebError>;
    fn get_item(&self, key: &str, library: &WebLibrary, item: &str) -> Result<WebItem, WebError>;
    fn patch_item(
        &self,
        key: &str,
        library: &WebLibrary,
        item: &str,
        version: u64,
        body: &Value,
    ) -> Result<Patched, WebError>;
}

/// api.zotero.org, over blocking HTTP (the copy drain is synchronous).
pub struct WebApiPort {
    base: String,
}

impl WebApiPort {
    pub fn live() -> Self {
        Self::new(API_BASE)
    }

    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn client(&self) -> Result<reqwest::blocking::Client, WebError> {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| WebError::Invalid("http client".into()))
    }

    fn send(
        &self,
        request: reqwest::blocking::RequestBuilder,
        key: &str,
    ) -> Result<reqwest::blocking::Response, WebError> {
        request
            .header("Zotero-API-Key", key)
            .header("Zotero-API-Version", "3")
            .send()
            .map_err(|_| WebError::Unreachable)
    }
}

impl WebPort for WebApiPort {
    fn key_info(&self, key: &str) -> Result<KeyCheck, WebError> {
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Ok(KeyCheck::InvalidKey);
        }
        let client = self.client()?;
        let response = self.send(client.get(format!("{}/keys/current", self.base)), key)?;
        let status = response.status().as_u16();
        let body = response.text().map_err(|_| WebError::Unreachable)?;
        classify_key_response(status, &body).map_err(WebError::Invalid)
    }

    fn get_item(&self, key: &str, library: &WebLibrary, item: &str) -> Result<WebItem, WebError> {
        let client = self.client()?;
        let url = format!("{}/{}/items/{item}", self.base, library.path());
        let response = self.send(client.get(url), key)?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(WebError::Rejected(status));
        }
        let json: Value = response
            .json()
            .map_err(|_| WebError::Invalid("unreadable item".into()))?;
        let data = json
            .get("data")
            .cloned()
            .ok_or_else(|| WebError::Invalid("item without data".into()))?;
        let version = json
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| WebError::Invalid("item without version".into()))?;
        Ok(WebItem { version, data })
    }

    fn patch_item(
        &self,
        key: &str,
        library: &WebLibrary,
        item: &str,
        version: u64,
        body: &Value,
    ) -> Result<Patched, WebError> {
        let client = self.client()?;
        let url = format!("{}/{}/items/{item}", self.base, library.path());
        let request = client
            .patch(url)
            .header("If-Unmodified-Since-Version", version.to_string())
            .json(body);
        let status = self.send(request, key)?.status().as_u16();
        match status {
            200 | 204 => Ok(Patched::Done),
            412 => Ok(Patched::Stale),
            status => Err(WebError::Rejected(status)),
        }
    }
}

/// The fields our source has and the item lacks, with the values to write.
///
/// A field is filled only when it is empty in Zotero; a non-empty one is never
/// touched, whatever it holds. Creators are never part of it: a web source names
/// no author. `websiteTitle` exists only on a web page.
pub fn missing_fields(ours: &Owned, data: &Value) -> Vec<(&'static str, String)> {
    let theirs = owned_from_zotero(data);
    let is_webpage = data.get("itemType").and_then(Value::as_str) == Some("webpage");
    plan_merge(ours, &theirs, &Owned::default())
        .fill
        .into_iter()
        .filter(|name| *name != "websiteTitle" || is_webpage)
        .filter_map(|name| Some((name, ours.get(name)?.to_string())))
        .collect()
}

pub fn patch_body(fields: &[(&'static str, String)]) -> Value {
    let mut body = Map::new();
    for (name, value) in fields {
        body.insert((*name).to_string(), Value::String(value.clone()));
    }
    Value::Object(body)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    Completed(Vec<&'static str>),
    NothingMissing,
    /// The item kept changing under us: nothing was written.
    Conflict,
}

/// Fills in what the item lacks. A `412` reads the item again and tries once
/// more; a second one is a [`Completion::Conflict`].
pub fn complete_item(
    web: &dyn WebPort,
    key: &str,
    library: &WebLibrary,
    item: &str,
    ours: &Owned,
) -> Result<Completion, WebError> {
    for attempt in 0..2 {
        let current = web.get_item(key, library, item)?;
        let fields = missing_fields(ours, &current.data);
        if fields.is_empty() {
            return Ok(Completion::NothingMissing);
        }
        match web.patch_item(key, library, item, current.version, &patch_body(&fields))? {
            Patched::Done => {
                return Ok(Completion::Completed(
                    fields.iter().map(|(name, _)| *name).collect(),
                ))
            }
            Patched::Stale if attempt == 1 => return Ok(Completion::Conflict),
            Patched::Stale => {}
        }
    }
    Ok(Completion::Conflict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navegador::zotero_copy::plan::Owned;
    use crate::navegador::zotero_copy::port::test_server::{canned, dead_base, serve};
    use serde_json::json;
    use std::sync::Mutex;

    const KEY: &str = "AbCdEf0123456789abcdEFgh";

    fn ours() -> Owned {
        Owned {
            title: Some("A title".into()),
            url: Some("https://a.test/x".into()),
            access_date: Some("2026-10-01T10:00:00Z".into()),
            website_title: Some("A site".into()),
        }
    }

    fn item(extra: Value) -> Value {
        let mut data = json!({
            "key": "ITEM1234", "itemType": "webpage",
            "title": "A title", "url": "https://a.test/x",
        });
        for (name, value) in extra.as_object().unwrap() {
            data[name] = value.clone();
        }
        data
    }

    #[test]
    fn only_fields_empty_in_zotero_are_filled() {
        let missing = missing_fields(&ours(), &item(json!({})));
        let names: Vec<&str> = missing.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, vec!["accessDate", "websiteTitle"]);
        assert_eq!(missing[0].1, "2026-10-01T10:00:00Z");
        assert_eq!(missing[1].1, "A site");
    }

    #[test]
    fn a_blank_field_in_zotero_counts_as_empty() {
        let data = item(json!({"accessDate": "  ", "websiteTitle": ""}));
        assert_eq!(missing_fields(&ours(), &data).len(), 2);
    }

    #[test]
    fn a_non_empty_field_is_never_overwritten_even_when_it_differs() {
        let data = item(json!({
            "title": "My own title",
            "accessDate": "2020-01-01T00:00:00Z",
            "websiteTitle": "Mine"
        }));
        assert!(missing_fields(&ours(), &data).is_empty());
    }

    #[test]
    fn a_website_title_is_only_filled_on_a_web_page() {
        let data = item(json!({"itemType": "journalArticle"}));
        let names: Vec<&str> = missing_fields(&ours(), &data)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(names, vec!["accessDate"]);
    }

    #[test]
    fn the_patch_body_holds_those_fields_and_nothing_else() {
        let data = item(json!({"creators": [{"creatorType": "author", "name": "Someone"}]}));
        let body = patch_body(&missing_fields(&ours(), &data));
        let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["accessDate", "websiteTitle"]);
        assert!(body.get("creators").is_none());
    }

    /// A scripted Web API: answers in order, every call logged.
    struct FakeWeb {
        gets: Mutex<Vec<Result<WebItem, WebError>>>,
        patches: Mutex<Vec<Result<Patched, WebError>>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeWeb {
        fn new(
            gets: Vec<Result<WebItem, WebError>>,
            patches: Vec<Result<Patched, WebError>>,
        ) -> Self {
            Self {
                gets: Mutex::new(gets),
                patches: Mutex::new(patches),
                calls: Mutex::new(vec![]),
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn web_item(version: u64, extra: Value) -> Result<WebItem, WebError> {
        Ok(WebItem {
            version,
            data: item(extra),
        })
    }

    impl WebPort for FakeWeb {
        fn key_info(&self, _: &str) -> Result<crate::zotero_web::KeyCheck, WebError> {
            unreachable!("completion does not verify the key")
        }
        fn get_item(&self, _: &str, _: &WebLibrary, item: &str) -> Result<WebItem, WebError> {
            self.calls.lock().unwrap().push(format!("get {item}"));
            self.gets.lock().unwrap().remove(0)
        }
        fn patch_item(
            &self,
            _: &str,
            _: &WebLibrary,
            item: &str,
            version: u64,
            body: &Value,
        ) -> Result<Patched, WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("patch {item} v{version} {body}"));
            self.patches.lock().unwrap().remove(0)
        }
    }

    fn complete(web: &FakeWeb) -> Result<Completion, WebError> {
        complete_item(web, KEY, &WebLibrary::User(7), "ITEM1234", &ours())
    }

    #[test]
    fn nothing_missing_means_no_write_at_all() {
        let full = json!({"accessDate": "2020-01-01T00:00:00Z", "websiteTitle": "Mine"});
        let web = FakeWeb::new(vec![web_item(4, full)], vec![]);
        assert_eq!(complete(&web).unwrap(), Completion::NothingMissing);
        assert_eq!(web.calls(), vec!["get ITEM1234"]);
    }

    #[test]
    fn missing_fields_are_patched_against_the_version_that_was_read() {
        let web = FakeWeb::new(vec![web_item(4, json!({}))], vec![Ok(Patched::Done)]);
        assert_eq!(
            complete(&web).unwrap(),
            Completion::Completed(vec!["accessDate", "websiteTitle"])
        );
        let calls = web.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].starts_with("patch ITEM1234 v4 "), "{}", calls[1]);
        assert!(calls[1].contains("\"accessDate\":\"2026-10-01T10:00:00Z\""));
        assert!(!calls[1].contains("\"title\""));
    }

    #[test]
    fn a_stale_version_reads_again_once_and_retries_once() {
        let web = FakeWeb::new(
            vec![
                web_item(4, json!({})),
                // Someone filled the site name meanwhile.
                web_item(5, json!({"websiteTitle": "Theirs"})),
            ],
            vec![Ok(Patched::Stale), Ok(Patched::Done)],
        );
        assert_eq!(
            complete(&web).unwrap(),
            Completion::Completed(vec!["accessDate"])
        );
        let calls = web.calls();
        assert_eq!(calls[0], "get ITEM1234");
        assert!(calls[1].starts_with("patch ITEM1234 v4 "));
        assert_eq!(calls[2], "get ITEM1234");
        assert!(calls[3].starts_with("patch ITEM1234 v5 "));
        assert!(!calls[3].contains("websiteTitle"), "their value is kept");
    }

    #[test]
    fn a_second_stale_version_is_a_conflict_not_a_loop() {
        let web = FakeWeb::new(
            vec![web_item(4, json!({})), web_item(5, json!({}))],
            vec![Ok(Patched::Stale), Ok(Patched::Stale)],
        );
        assert_eq!(complete(&web).unwrap(), Completion::Conflict);
        assert_eq!(web.calls().len(), 4);
    }

    #[test]
    fn a_stale_version_after_which_nothing_is_missing_is_not_a_conflict() {
        let full = json!({"accessDate": "2020-01-01T00:00:00Z", "websiteTitle": "Theirs"});
        let web = FakeWeb::new(
            vec![web_item(4, json!({})), web_item(5, full)],
            vec![Ok(Patched::Stale)],
        );
        assert_eq!(complete(&web).unwrap(), Completion::NothingMissing);
    }

    #[test]
    fn a_refusal_is_an_error_not_a_retry() {
        let web = FakeWeb::new(
            vec![web_item(4, json!({}))],
            vec![Err(WebError::Rejected(403))],
        );
        assert_eq!(complete(&web).unwrap_err(), WebError::Rejected(403));
        assert_eq!(web.calls().len(), 2);
    }

    const ITEM_JSON: &str = r#"{"key":"ITEM1234","version":9,"data":{"key":"ITEM1234","version":9,"itemType":"webpage","title":"A title","url":"https://a.test/x"}}"#;

    #[test]
    fn the_real_port_reads_an_item_and_its_version() {
        let server = serve(vec![canned(
            "GET",
            "/users/7/items/ITEM1234",
            200,
            ITEM_JSON,
        )]);
        let got = WebApiPort::new(&server.base)
            .get_item(KEY, &WebLibrary::User(7), "ITEM1234")
            .unwrap();
        assert_eq!(got.version, 9);
        assert_eq!(got.data["title"], "A title");
        let seen = server.seen.lock().unwrap();
        assert_eq!(
            seen[0].headers.get("zotero-api-key").map(String::as_str),
            Some(KEY)
        );
        assert_eq!(
            seen[0]
                .headers
                .get("zotero-api-version")
                .map(String::as_str),
            Some("3")
        );
        assert!(!seen[0].target.contains(KEY));
    }

    #[test]
    fn the_real_port_patches_a_group_item_with_the_version_precondition() {
        let server = serve(vec![canned(
            "PATCH",
            "/groups/6680944/items/ITEM1234",
            204,
            "",
        )]);
        let done = WebApiPort::new(&server.base)
            .patch_item(
                KEY,
                &WebLibrary::Group("6680944".into()),
                "ITEM1234",
                9,
                &json!({"accessDate": "2026-10-01T10:00:00Z"}),
            )
            .unwrap();
        assert_eq!(done, Patched::Done);
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen[0].method, "PATCH");
        assert_eq!(
            seen[0]
                .headers
                .get("if-unmodified-since-version")
                .map(String::as_str),
            Some("9")
        );
        assert_eq!(
            seen[0].headers.get("zotero-api-key").map(String::as_str),
            Some(KEY)
        );
        let body: Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert_eq!(body, json!({"accessDate": "2026-10-01T10:00:00Z"}));
    }

    #[test]
    fn the_real_port_maps_412_to_stale_and_other_statuses_to_a_refusal() {
        let patch = |status: u16| {
            let server = serve(vec![canned("PATCH", "/users/7/items/", status, "no")]);
            WebApiPort::new(&server.base).patch_item(
                KEY,
                &WebLibrary::User(7),
                "ITEM1234",
                1,
                &json!({"title": "x"}),
            )
        };
        assert_eq!(patch(412), Ok(Patched::Stale));
        assert_eq!(patch(403), Err(WebError::Rejected(403)));
        assert_eq!(patch(400), Err(WebError::Rejected(400)));
    }

    #[test]
    fn the_real_port_reports_an_unreachable_zotero_without_the_key() {
        let error = WebApiPort::new(&dead_base())
            .get_item(KEY, &WebLibrary::User(7), "ITEM1234")
            .unwrap_err();
        assert_eq!(error, WebError::Unreachable);
        assert!(!format!("{error:?}").contains(KEY));
    }
}
