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

/// Where a Web API step went wrong, short enough to show and free of anything
/// secret: the phase, and the HTTP status or `network` / `invalid`. Never the
/// key, a URL or a response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub phase: &'static str,
    pub cause: String,
}

impl Failure {
    pub fn of(phase: &'static str, error: &WebError) -> Self {
        let cause = match error {
            WebError::Unreachable => "network".to_string(),
            WebError::Rejected(status) => status.to_string(),
            WebError::Invalid(_) => "invalid".to_string(),
        };
        Self { phase, cause }
    }

    /// `phase:cause`, e.g. `read_item:404`.
    pub fn code(&self) -> String {
        format!("{}:{}", self.phase, self.cause)
    }

    pub fn is_not_found(&self) -> bool {
        self.cause == "404"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebItem {
    pub version: u64,
    pub data: Value,
}

/// A file to attach, with the facts Zotero's upload authorization asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfFile {
    pub title: String,
    pub filename: String,
    pub md5: String,
    pub size: u64,
    pub mtime_ms: u64,
    pub bytes: Vec<u8>,
}

/// Where and how to send the bytes, as Zotero's upload authorization says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadTicket {
    pub url: String,
    pub content_type: String,
    pub prefix: String,
    pub suffix: String,
    pub upload_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authorization {
    /// `{"exists":1}`: Zotero already stores this file; nothing to upload.
    Exists,
    Upload(UploadTicket),
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

    // The file-upload flow. Defaults refuse, so a port that only edits fields
    // (and the fakes of its tests) need not know about files.
    fn children(&self, _: &str, _: &WebLibrary, _: &str) -> Result<Vec<Value>, WebError> {
        Err(WebError::Invalid("files are not supported".into()))
    }
    /// Creates the attachment item; the write token makes a repeated create a
    /// no-op. Returns its key and version.
    fn create_attachment(
        &self,
        _: &str,
        _: &WebLibrary,
        _: &Value,
        _: &str,
    ) -> Result<(String, u64), WebError> {
        Err(WebError::Invalid("files are not supported".into()))
    }
    fn authorize_upload(
        &self,
        _: &str,
        _: &WebLibrary,
        _: &str,
        _: &PdfFile,
    ) -> Result<Authorization, WebError> {
        Err(WebError::Invalid("files are not supported".into()))
    }
    fn upload_file(&self, _: &UploadTicket, _: &[u8]) -> Result<(), WebError> {
        Err(WebError::Invalid("files are not supported".into()))
    }
    fn register_upload(&self, _: &str, _: &WebLibrary, _: &str, _: &str) -> Result<(), WebError> {
        Err(WebError::Invalid("files are not supported".into()))
    }
    fn delete_item(&self, _: &str, _: &WebLibrary, _: &str, _: u64) -> Result<(), WebError> {
        Err(WebError::Invalid("files are not supported".into()))
    }
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
        if !matches!(status, 200 | 403 | 404) {
            return Err(WebError::Rejected(status));
        }
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

    fn children(
        &self,
        key: &str,
        library: &WebLibrary,
        parent: &str,
    ) -> Result<Vec<Value>, WebError> {
        let client = self.client()?;
        let url = format!(
            "{}/{}/items/{parent}/children?limit=100",
            self.base,
            library.path()
        );
        let response = self.send(client.get(url), key)?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(WebError::Rejected(status));
        }
        let json: Value = response
            .json()
            .map_err(|_| WebError::Invalid("unreadable children".into()))?;
        Ok(json
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.get("data").cloned())
                    .collect()
            })
            .unwrap_or_default())
    }

    fn create_attachment(
        &self,
        key: &str,
        library: &WebLibrary,
        body: &Value,
        token: &str,
    ) -> Result<(String, u64), WebError> {
        let client = self.client()?;
        let url = format!("{}/{}/items", self.base, library.path());
        let request = client
            .post(url)
            .header("Zotero-Write-Token", token)
            .json(body);
        let response = self.send(request, key)?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(WebError::Rejected(status));
        }
        let json: Value = response
            .json()
            .map_err(|_| WebError::Invalid("unreadable create answer".into()))?;
        if let Some(code) = json.pointer("/failed/0/code").and_then(Value::as_u64) {
            return Err(WebError::Rejected(code as u16));
        }
        let created = json
            .pointer("/successful/0")
            .ok_or_else(|| WebError::Invalid("the attachment was not created".into()))?;
        let item = created
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| WebError::Invalid("created item without key".into()))?;
        let version = created.get("version").and_then(Value::as_u64).unwrap_or(0);
        Ok((item.to_string(), version))
    }

    fn authorize_upload(
        &self,
        key: &str,
        library: &WebLibrary,
        item: &str,
        file: &PdfFile,
    ) -> Result<Authorization, WebError> {
        let client = self.client()?;
        let url = format!("{}/{}/items/{item}/file", self.base, library.path());
        let form = format!(
            "md5={}&filename={}&filesize={}&mtime={}",
            file.md5,
            urlencoding::encode(&file.filename),
            file.size,
            file.mtime_ms
        );
        let request = client
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("If-None-Match", "*")
            .body(form);
        let response = self.send(request, key)?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(WebError::Rejected(status));
        }
        let json: Value = response
            .json()
            .map_err(|_| WebError::Invalid("unreadable authorization".into()))?;
        if json.get("exists").and_then(Value::as_u64) == Some(1) {
            return Ok(Authorization::Exists);
        }
        let text = |name: &str| json.get(name).and_then(Value::as_str).map(str::to_string);
        match (
            text("url"),
            text("contentType"),
            text("prefix"),
            text("suffix"),
            text("uploadKey"),
        ) {
            (Some(url), Some(content_type), Some(prefix), Some(suffix), Some(upload_key)) => {
                Ok(Authorization::Upload(UploadTicket {
                    url,
                    content_type,
                    prefix,
                    suffix,
                    upload_key,
                }))
            }
            _ => Err(WebError::Invalid("incomplete authorization".into())),
        }
    }

    fn upload_file(&self, ticket: &UploadTicket, bytes: &[u8]) -> Result<(), WebError> {
        // The storage address belongs to Zotero's storage, not to the API: it
        // gets no key and no API headers.
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|_| WebError::Invalid("http client".into()))?;
        let mut body = Vec::with_capacity(ticket.prefix.len() + bytes.len() + ticket.suffix.len());
        body.extend_from_slice(ticket.prefix.as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(ticket.suffix.as_bytes());
        let response = client
            .post(&ticket.url)
            .header("Content-Type", &ticket.content_type)
            .body(body)
            .send()
            .map_err(|_| WebError::Unreachable)?;
        match response.status().as_u16() {
            200 | 201 | 204 => Ok(()),
            status => Err(WebError::Rejected(status)),
        }
    }

    fn register_upload(
        &self,
        key: &str,
        library: &WebLibrary,
        item: &str,
        upload_key: &str,
    ) -> Result<(), WebError> {
        let client = self.client()?;
        let url = format!("{}/{}/items/{item}/file", self.base, library.path());
        let request = client
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("If-None-Match", "*")
            .body(format!("upload={upload_key}"));
        match self.send(request, key)?.status().as_u16() {
            200 | 204 => Ok(()),
            status => Err(WebError::Rejected(status)),
        }
    }

    fn delete_item(
        &self,
        key: &str,
        library: &WebLibrary,
        item: &str,
        version: u64,
    ) -> Result<(), WebError> {
        let client = self.client()?;
        let url = format!("{}/{}/items/{item}", self.base, library.path());
        let request = client
            .delete(url)
            .header("If-Unmodified-Since-Version", version.to_string());
        match self.send(request, key)?.status().as_u16() {
            200 | 204 => Ok(()),
            status => Err(WebError::Rejected(status)),
        }
    }
}

pub fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    Md5::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfOutcome {
    Attached,
    /// An attachment with this file is already a child of the item.
    AlreadyThere,
    /// The person's Zotero storage is full (`413`).
    Quota,
    Failed(Failure),
}

fn already_a_child(children: &[Value], file: &PdfFile) -> bool {
    children.iter().any(|child| {
        if child.get("itemType").and_then(Value::as_str) != Some("attachment") {
            return false;
        }
        match child
            .get("md5")
            .and_then(Value::as_str)
            .filter(|md5| !md5.is_empty())
        {
            Some(md5) => md5.eq_ignore_ascii_case(&file.md5),
            // No digest yet (an upload that never finished): the filename is
            // named by the file's hash, so it says the same.
            None => child.get("filename").and_then(Value::as_str) == Some(file.filename.as_str()),
        }
    })
}

fn write_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Attaches `file` to `parent` through the upload flow, unless the item already
/// has it. Never fails the caller: the outcome says what happened. An attachment
/// item that could not get its file is deleted again, so no empty one is left.
pub fn attach_pdf_via_web(
    web: &dyn WebPort,
    key: &str,
    library: &WebLibrary,
    parent: &str,
    file: &PdfFile,
) -> PdfOutcome {
    match web.children(key, library, parent) {
        Ok(children) if already_a_child(&children, file) => return PdfOutcome::AlreadyThere,
        Ok(_) => {}
        Err(error) => return PdfOutcome::Failed(Failure::of("children", &error)),
    }
    let body = serde_json::json!([{
        "itemType": "attachment",
        "parentItem": parent,
        "linkMode": "imported_file",
        "title": file.title,
        "contentType": "application/pdf",
        "filename": file.filename,
    }]);
    let (attachment, version) = match web.create_attachment(key, library, &body, &write_token()) {
        Ok(created) => created,
        Err(error) => return PdfOutcome::Failed(Failure::of("create_attachment", &error)),
    };
    let uploaded = (|| -> Result<(), Failure> {
        let authorization = web
            .authorize_upload(key, library, &attachment, file)
            .map_err(|error| Failure::of("authorize", &error))?;
        match authorization {
            Authorization::Exists => Ok(()),
            Authorization::Upload(ticket) => {
                web.upload_file(&ticket, &file.bytes)
                    .map_err(|error| Failure::of("upload", &error))?;
                web.register_upload(key, library, &attachment, &ticket.upload_key)
                    .map_err(|error| Failure::of("register", &error))
            }
        }
    })();
    match uploaded {
        Ok(()) => PdfOutcome::Attached,
        Err(error) => {
            let _ = web.delete_item(key, library, &attachment, version);
            if error.cause == "413" {
                PdfOutcome::Quota
            } else {
                PdfOutcome::Failed(error)
            }
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
) -> Result<Completion, Failure> {
    for attempt in 0..2 {
        let current = web
            .get_item(key, library, item)
            .map_err(|error| Failure::of("read_item", &error))?;
        let fields = missing_fields(ours, &current.data);
        if fields.is_empty() {
            return Ok(Completion::NothingMissing);
        }
        let patched = web
            .patch_item(key, library, item, current.version, &patch_body(&fields))
            .map_err(|error| Failure::of("patch", &error))?;
        match patched {
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

    fn complete(web: &FakeWeb) -> Result<Completion, Failure> {
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
        assert_eq!(
            complete(&web).unwrap_err(),
            Failure {
                phase: "patch",
                cause: "403".into()
            }
        );
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

    // --- Attaching a PDF through the file-upload flow ---------------------------

    fn pdf() -> PdfFile {
        PdfFile {
            title: "Captura web abcd1234.pdf".into(),
            filename: "web-capture-abcd1234.pdf".into(),
            md5: md5_hex(b"%PDF-1.4 fake"),
            size: 13,
            mtime_ms: 1_700_000_000_000,
            bytes: b"%PDF-1.4 fake".to_vec(),
        }
    }

    fn ticket() -> Authorization {
        Authorization::Upload(UploadTicket {
            url: "https://storage.test/upload".into(),
            content_type: "multipart/form-data; boundary=x".into(),
            prefix: "--x\r\n".into(),
            suffix: "\r\n--x--".into(),
            upload_key: "UPKEY".into(),
        })
    }

    /// A scripted file-upload side of the Web API. Every call is logged.
    struct FakeFiles {
        children: Vec<Value>,
        create: Result<(String, u64), WebError>,
        auth: Result<Authorization, WebError>,
        upload: Result<(), WebError>,
        register: Result<(), WebError>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeFiles {
        fn working() -> Self {
            Self {
                children: vec![],
                create: Ok(("ATTKEY12".into(), 31)),
                auth: Ok(ticket()),
                upload: Ok(()),
                register: Ok(()),
                calls: Mutex::new(vec![]),
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl WebPort for FakeFiles {
        fn key_info(&self, _: &str) -> Result<crate::zotero_web::KeyCheck, WebError> {
            unreachable!()
        }
        fn get_item(&self, _: &str, _: &WebLibrary, _: &str) -> Result<WebItem, WebError> {
            unreachable!()
        }
        fn patch_item(
            &self,
            _: &str,
            _: &WebLibrary,
            _: &str,
            _: u64,
            _: &Value,
        ) -> Result<Patched, WebError> {
            unreachable!()
        }
        fn children(&self, _: &str, _: &WebLibrary, parent: &str) -> Result<Vec<Value>, WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("children {parent}"));
            Ok(self.children.clone())
        }
        fn create_attachment(
            &self,
            _: &str,
            _: &WebLibrary,
            body: &Value,
            token: &str,
        ) -> Result<(String, u64), WebError> {
            assert_eq!(token.len(), 32, "a write token makes the create idempotent");
            self.calls.lock().unwrap().push(format!("create {body}"));
            self.create.clone()
        }
        fn authorize_upload(
            &self,
            _: &str,
            _: &WebLibrary,
            item: &str,
            file: &PdfFile,
        ) -> Result<Authorization, WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("authorize {item} {} {}", file.md5, file.size));
            self.auth.clone()
        }
        fn upload_file(&self, ticket: &UploadTicket, bytes: &[u8]) -> Result<(), WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("upload {} {}", ticket.url, bytes.len()));
            self.upload.clone()
        }
        fn register_upload(
            &self,
            _: &str,
            _: &WebLibrary,
            item: &str,
            upload_key: &str,
        ) -> Result<(), WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("register {item} {upload_key}"));
            self.register.clone()
        }
        fn delete_item(
            &self,
            _: &str,
            _: &WebLibrary,
            item: &str,
            version: u64,
        ) -> Result<(), WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("delete {item} v{version}"));
            Ok(())
        }
    }

    fn attach(web: &FakeFiles) -> PdfOutcome {
        attach_pdf_via_web(web, KEY, &WebLibrary::User(7), "PARENT12", &pdf())
    }

    #[test]
    fn a_pdf_is_created_authorized_uploaded_and_registered_in_that_order() {
        let web = FakeFiles::working();
        assert_eq!(attach(&web), PdfOutcome::Attached);
        let calls = web.calls();
        assert_eq!(calls[0], "children PARENT12");
        assert!(calls[1].starts_with("create "));
        let body: Value = serde_json::from_str(calls[1].trim_start_matches("create ")).unwrap();
        let item = &body[0];
        assert_eq!(item["itemType"], "attachment");
        assert_eq!(item["linkMode"], "imported_file");
        assert_eq!(item["parentItem"], "PARENT12");
        assert_eq!(item["contentType"], "application/pdf");
        assert_eq!(item["filename"], "web-capture-abcd1234.pdf");
        assert_eq!(item["title"], "Captura web abcd1234.pdf");
        assert_eq!(
            calls[2],
            format!("authorize ATTKEY12 {} 13", md5_hex(b"%PDF-1.4 fake"))
        );
        // prefix + file + suffix
        assert_eq!(
            calls[3],
            "upload https://storage.test/upload 13".to_string()
        );
        assert_eq!(calls[4], "register ATTKEY12 UPKEY");
        assert_eq!(calls.len(), 5);
    }

    #[test]
    fn a_file_zotero_already_stores_needs_no_upload() {
        let mut web = FakeFiles::working();
        web.auth = Ok(Authorization::Exists);
        assert_eq!(attach(&web), PdfOutcome::Attached);
        let calls = web.calls();
        assert!(!calls
            .iter()
            .any(|c| c.starts_with("upload") || c.starts_with("register")));
        assert!(
            !calls.iter().any(|c| c.starts_with("delete")),
            "the attachment stays"
        );
    }

    #[test]
    fn the_same_md5_among_the_children_is_never_attached_twice() {
        let mut web = FakeFiles::working();
        web.children = vec![
            json!({"itemType": "note"}),
            json!({"itemType": "attachment", "md5": md5_hex(b"%PDF-1.4 fake").to_uppercase(), "filename": "other.pdf"}),
        ];
        assert_eq!(attach(&web), PdfOutcome::AlreadyThere);
        assert_eq!(web.calls(), vec!["children PARENT12"]);
    }

    #[test]
    fn without_an_md5_the_filename_is_what_says_it_is_there() {
        let mut web = FakeFiles::working();
        web.children =
            vec![json!({"itemType": "attachment", "filename": "web-capture-abcd1234.pdf"})];
        assert_eq!(attach(&web), PdfOutcome::AlreadyThere);
        // A different file with its own md5 does not count.
        let mut web = FakeFiles::working();
        web.children =
            vec![json!({"itemType": "attachment", "md5": "0".repeat(32), "filename": "else.pdf"})];
        assert_eq!(attach(&web), PdfOutcome::Attached);
    }

    #[test]
    fn a_full_storage_quota_is_its_own_outcome_and_leaves_no_empty_attachment() {
        let mut web = FakeFiles::working();
        web.auth = Err(WebError::Rejected(413));
        assert_eq!(attach(&web), PdfOutcome::Quota);
        let calls = web.calls();
        assert_eq!(calls.last().unwrap(), "delete ATTKEY12 v31");
        assert!(!calls.iter().any(|c| c.starts_with("upload")));
    }

    #[test]
    fn a_precondition_failure_or_any_refusal_fails_and_cleans_up() {
        for refusal in [412, 403, 500] {
            let mut web = FakeFiles::working();
            web.auth = Err(WebError::Rejected(refusal));
            assert_eq!(
                attach(&web),
                PdfOutcome::Failed(failure("authorize", &refusal.to_string())),
                "{refusal}"
            );
            assert_eq!(web.calls().last().unwrap(), "delete ATTKEY12 v31");
        }
        let mut web = FakeFiles::working();
        web.upload = Err(WebError::Rejected(500));
        assert_eq!(attach(&web), PdfOutcome::Failed(failure("upload", "500")));
        assert_eq!(web.calls().last().unwrap(), "delete ATTKEY12 v31");
        let mut web = FakeFiles::working();
        web.upload = Err(WebError::Unreachable);
        assert_eq!(
            attach(&web),
            PdfOutcome::Failed(failure("upload", "network"))
        );
        let mut web = FakeFiles::working();
        web.register = Err(WebError::Rejected(412));
        assert_eq!(attach(&web), PdfOutcome::Failed(failure("register", "412")));
        assert_eq!(web.calls().last().unwrap(), "delete ATTKEY12 v31");
    }

    fn failure(phase: &'static str, cause: &str) -> Failure {
        Failure {
            phase,
            cause: cause.to_string(),
        }
    }

    #[test]
    fn a_failure_reads_as_a_short_key_free_code() {
        assert_eq!(failure("read_item", "404").code(), "read_item:404");
        assert_eq!(
            Failure::of("children", &WebError::Unreachable).code(),
            "children:network"
        );
        assert_eq!(
            Failure::of("key_info", &WebError::Invalid(KEY.into())).code(),
            "key_info:invalid",
            "an unreadable answer never lands in the code"
        );
    }

    #[test]
    fn an_item_missing_on_the_server_is_a_read_failure_with_404() {
        let web = FakeWeb::new(vec![Err(WebError::Rejected(404))], vec![]);
        let error = complete(&web).unwrap_err();
        assert_eq!(error, failure("read_item", "404"));
        assert!(error.is_not_found());
    }

    #[test]
    fn a_failure_to_create_the_attachment_has_nothing_to_clean() {
        let mut web = FakeFiles::working();
        web.create = Err(WebError::Rejected(400));
        assert_eq!(
            attach(&web),
            PdfOutcome::Failed(failure("create_attachment", "400"))
        );
        assert!(!web.calls().iter().any(|c| c.starts_with("delete")));
    }

    #[test]
    fn the_real_port_creates_an_attachment_with_a_write_token() {
        let reply = r#"{"successful":{"0":{"key":"ATTKEY12","version":31,"data":{}}},"success":{"0":"ATTKEY12"},"failed":{}}"#;
        let server = serve(vec![canned("POST", "/users/7/items", 200, reply)]);
        let (key, version) = WebApiPort::new(&server.base)
            .create_attachment(
                KEY,
                &WebLibrary::User(7),
                &json!([{"itemType": "attachment"}]),
                &"a".repeat(32),
            )
            .unwrap();
        assert_eq!((key.as_str(), version), ("ATTKEY12", 31));
        let seen = server.seen.lock().unwrap();
        assert_eq!(
            seen[0]
                .headers
                .get("zotero-write-token")
                .map(String::as_str),
            Some("a".repeat(32).as_str())
        );
        assert_eq!(
            seen[0].headers.get("zotero-api-key").map(String::as_str),
            Some(KEY)
        );
    }

    #[test]
    fn a_failed_create_entry_is_a_refusal() {
        let reply = r#"{"successful":{},"success":{},"failed":{"0":{"code":400,"message":"bad"}}}"#;
        let server = serve(vec![canned("POST", "/users/7/items", 200, reply)]);
        let error = WebApiPort::new(&server.base)
            .create_attachment(KEY, &WebLibrary::User(7), &json!([{}]), &"a".repeat(32))
            .unwrap_err();
        assert_eq!(error, WebError::Rejected(400));
    }

    #[test]
    fn the_real_port_asks_for_upload_authorization_with_the_file_facts() {
        let reply = r#"{"url":"https://s.test/u","contentType":"multipart/form-data; boundary=x","prefix":"--x\r\n","suffix":"\r\n--x--","uploadKey":"UPKEY"}"#;
        let server = serve(vec![canned(
            "POST",
            "/groups/9/items/ATTKEY12/file",
            200,
            reply,
        )]);
        let auth = WebApiPort::new(&server.base)
            .authorize_upload(KEY, &WebLibrary::Group("9".into()), "ATTKEY12", &pdf())
            .unwrap();
        assert_eq!(auth, ticket_from_reply());
        let seen = server.seen.lock().unwrap();
        assert_eq!(
            seen[0].headers.get("if-none-match").map(String::as_str),
            Some("*")
        );
        assert_eq!(
            seen[0].headers.get("content-type").map(String::as_str),
            Some("application/x-www-form-urlencoded")
        );
        let body = String::from_utf8(seen[0].body.clone()).unwrap();
        assert!(
            body.contains(&format!("md5={}", md5_hex(b"%PDF-1.4 fake"))),
            "{body}"
        );
        assert!(body.contains("filename=web-capture-abcd1234.pdf"), "{body}");
        assert!(body.contains("filesize=13"), "{body}");
        assert!(body.contains("mtime=1700000000000"), "{body}");
    }

    fn ticket_from_reply() -> Authorization {
        Authorization::Upload(UploadTicket {
            url: "https://s.test/u".into(),
            content_type: "multipart/form-data; boundary=x".into(),
            prefix: "--x\r\n".into(),
            suffix: "\r\n--x--".into(),
            upload_key: "UPKEY".into(),
        })
    }

    #[test]
    fn exists_one_means_no_upload_and_413_maps_to_a_refusal() {
        let server = serve(vec![canned(
            "POST",
            "/users/7/items/A/file",
            200,
            r#"{"exists":1}"#,
        )]);
        let auth = WebApiPort::new(&server.base)
            .authorize_upload(KEY, &WebLibrary::User(7), "A", &pdf())
            .unwrap();
        assert_eq!(auth, Authorization::Exists);
        for status in [413, 412] {
            let server = serve(vec![canned("POST", "/users/7/items/A/file", status, "no")]);
            let error = WebApiPort::new(&server.base)
                .authorize_upload(KEY, &WebLibrary::User(7), "A", &pdf())
                .unwrap_err();
            assert_eq!(error, WebError::Rejected(status));
        }
    }

    #[test]
    fn the_file_goes_to_the_given_address_with_prefix_and_suffix_and_without_the_key() {
        let server = serve(vec![canned("POST", "/upload", 201, "")]);
        let ticket = UploadTicket {
            url: format!("{}/upload", server.base),
            content_type: "multipart/form-data; boundary=x".into(),
            prefix: "--x\r\n".into(),
            suffix: "\r\n--x--".into(),
            upload_key: "UPKEY".into(),
        };
        WebApiPort::new(&dead_base())
            .upload_file(&ticket, b"%PDF-1.4 fake")
            .unwrap();
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen[0].body, b"--x\r\n%PDF-1.4 fake\r\n--x--".to_vec());
        assert_eq!(
            seen[0].headers.get("content-type").map(String::as_str),
            Some("multipart/form-data; boundary=x")
        );
        assert!(
            !seen[0].headers.contains_key("zotero-api-key"),
            "storage never sees the key"
        );
    }

    #[test]
    fn registering_an_upload_needs_the_upload_key_and_no_existing_file() {
        let server = serve(vec![canned("POST", "/users/7/items/A/file", 204, "")]);
        WebApiPort::new(&server.base)
            .register_upload(KEY, &WebLibrary::User(7), "A", "UPKEY")
            .unwrap();
        let seen = server.seen.lock().unwrap();
        assert_eq!(
            String::from_utf8(seen[0].body.clone()).unwrap(),
            "upload=UPKEY"
        );
        assert_eq!(
            seen[0].headers.get("if-none-match").map(String::as_str),
            Some("*")
        );
    }

    #[test]
    fn the_real_port_lists_children_and_deletes_with_a_version() {
        let children = r#"[{"key":"C1","version":2,"data":{"itemType":"attachment","md5":"abc","filename":"f.pdf"}}]"#;
        let server = serve(vec![
            canned("GET", "/users/7/items/P/children", 200, children),
            canned("DELETE", "/users/7/items/A", 204, ""),
        ]);
        let port = WebApiPort::new(&server.base);
        let got = port.children(KEY, &WebLibrary::User(7), "P").unwrap();
        assert_eq!(got[0]["md5"], "abc");
        port.delete_item(KEY, &WebLibrary::User(7), "A", 31)
            .unwrap();
        let seen = server.seen.lock().unwrap();
        assert_eq!(
            seen[1]
                .headers
                .get("if-unmodified-since-version")
                .map(String::as_str),
            Some("31")
        );
    }

    #[test]
    fn the_real_port_keeps_the_status_of_an_unexpected_key_answer() {
        let server = serve(vec![canned("GET", "/keys/current", 500, "boom")]);
        let error = WebApiPort::new(&server.base).key_info(KEY).unwrap_err();
        assert_eq!(error, WebError::Rejected(500));
    }

    #[test]
    fn md5_is_the_hex_digest_zotero_expects() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }
}
