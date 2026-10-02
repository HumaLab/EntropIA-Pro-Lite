//! The Zotero side of a copy, behind a trait so the state machine runs against a
//! fake and the real thing is tested against a canned local HTTP server.
//!
//! Writes go through Zotero's own connector, the way its browser extension saves
//! a page (verified against the connector source shipped in Zotero 9.0.6,
//! `xpcom/server/server_connector.js` and `saveSession.js`):
//!
//! 1. `POST /connector/saveItems` `{sessionID, uri, items}` creates the item in
//!    whatever the Zotero window has selected and answers `201`.
//! 2. `POST /connector/updateSession` `{sessionID, target: "L<libraryID>"}`
//!    moves the session's items to that library root (`moveToLibrary`) and
//!    selects it, so the item ends up where the person chose.
//! 3. `POST /connector/saveAttachment?sessionID=` with `X-Metadata`
//!    `{sessionID, parentItemID, title, url}` and the raw bytes attaches a file
//!    to an item of that same session.
//!
//! Reads use the local API (`/api/users/0/...`, `/api/groups/<id>/...`), which is
//! GET-only. `/connector/getSelectedCollection` lists the editable libraries as
//! `level: 0` targets named like the libraries.

use std::time::Duration;

use serde_json::{json, Value};

use crate::writing::zotero::{connector::BASE_URL, Library};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// Nothing answered: Zotero is closed, or something else holds the port.
    Unreachable,
    /// Zotero answered and said no.
    Rejected { status: u16, detail: String },
    /// Zotero answered with something this build cannot read.
    Invalid(String),
}

/// One editable library as the connector names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetLibrary {
    /// `L<libraryID>`: what `updateSession` takes.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Targets {
    /// The library the Zotero window has selected, as `L<libraryID>`.
    pub selected_library: String,
    pub libraries: Vec<TargetLibrary>,
}

pub trait ZoteroPort {
    fn ping(&self) -> Result<(), PortError>;
    fn targets(&self) -> Result<Targets, PortError>;
    /// Every group the user belongs to, as `(id, name)` (local API).
    fn groups(&self) -> Result<Vec<(String, String)>, PortError>;
    /// The group's name (local API), `None` when Zotero has no such group.
    fn group_name(&self, group_id: &str) -> Result<Option<String>, PortError>;
    /// Raw items of the library that may carry `url` (the caller filters).
    fn find_items(&self, library: &Library, url: &str) -> Result<Vec<Value>, PortError>;
    fn children(&self, library: &Library, key: &str) -> Result<Vec<Value>, PortError>;
    fn save_item(&self, session: &str, uri: &str, item: Value) -> Result<(), PortError>;
    fn move_session(&self, session: &str, target: &str) -> Result<(), PortError>;
    /// `true` when the file was attached, `false` when Zotero answered that the
    /// library takes no files.
    fn attach_pdf(
        &self,
        session: &str,
        parent_item: &str,
        title: &str,
        url: &str,
        bytes: Vec<u8>,
    ) -> Result<bool, PortError>;
}

/// The real thing: Zotero on this machine.
pub struct ConnectorPort {
    base: String,
}

impl ConnectorPort {
    pub fn local() -> Self {
        Self::new(BASE_URL)
    }

    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn client(&self, timeout: Duration) -> Result<reqwest::blocking::Client, PortError> {
        reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| PortError::Invalid(format!("http client: {error}")))
    }

    fn reach(error: reqwest::Error) -> PortError {
        if error.is_connect() || error.is_timeout() {
            PortError::Unreachable
        } else {
            PortError::Invalid(error.to_string())
        }
    }

    fn rejected(response: reqwest::blocking::Response) -> PortError {
        let status = response.status().as_u16();
        let detail = response.text().unwrap_or_default();
        PortError::Rejected {
            status,
            detail: detail.chars().take(200).collect(),
        }
    }

    fn post_json(&self, path: &str, body: &Value, expect: &[u16]) -> Result<(), PortError> {
        let response = self
            .client(Duration::from_secs(30))?
            .post(format!("{}{path}", self.base))
            .json(body)
            .send()
            .map_err(Self::reach)?;
        if expect.contains(&response.status().as_u16()) {
            Ok(())
        } else {
            Err(Self::rejected(response))
        }
    }

    fn get_json(&self, url: &str) -> Result<Option<Value>, PortError> {
        let response = self
            .client(Duration::from_secs(10))?
            .get(url)
            .send()
            .map_err(Self::reach)?;
        match response.status().as_u16() {
            200 => response
                .json::<Value>()
                .map(Some)
                .map_err(|error| PortError::Invalid(format!("unreadable answer: {error}"))),
            404 => Ok(None),
            _ => Err(Self::rejected(response)),
        }
    }

    fn library_path(&self, library: &Library) -> String {
        format!(
            "{}/api/{}/{}",
            self.base,
            library.library_type.api_segment(),
            library.library_id
        )
    }
}

impl ZoteroPort for ConnectorPort {
    fn ping(&self) -> Result<(), PortError> {
        let response = self
            .client(Duration::from_secs(3))?
            .get(format!("{}/connector/ping", self.base))
            .send()
            .map_err(|_| PortError::Unreachable)?;
        let body = response.text().map_err(|_| PortError::Unreachable)?;
        if body.contains("Zotero is running") {
            Ok(())
        } else {
            Err(PortError::Unreachable)
        }
    }

    fn targets(&self) -> Result<Targets, PortError> {
        let response = self
            .client(Duration::from_secs(10))?
            .post(format!("{}/connector/getSelectedCollection", self.base))
            .json(&json!({}))
            .send()
            .map_err(Self::reach)?;
        if !response.status().is_success() {
            return Err(Self::rejected(response));
        }
        let body: Value = response
            .json()
            .map_err(|error| PortError::Invalid(format!("unreadable targets: {error}")))?;
        let selected = body
            .get("libraryID")
            .and_then(Value::as_u64)
            .ok_or_else(|| PortError::Invalid("the selected library is missing".into()))?;
        let libraries = body
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| PortError::Invalid("the library list is missing".into()))?
            .iter()
            .filter(|target| target.get("level").and_then(Value::as_u64) == Some(0))
            .filter_map(|target| {
                Some(TargetLibrary {
                    id: target.get("id")?.as_str()?.to_string(),
                    name: target.get("name")?.as_str()?.to_string(),
                })
            })
            .collect();
        Ok(Targets {
            selected_library: format!("L{selected}"),
            libraries,
        })
    }

    fn groups(&self) -> Result<Vec<(String, String)>, PortError> {
        let url = format!("{}/api/users/0/groups?format=json&limit=100", self.base);
        let Some(Value::Array(items)) = self.get_json(&url)? else {
            return Ok(Vec::new());
        };
        Ok(items
            .iter()
            .filter_map(|group| {
                let id = group.get("id")?.as_u64()?;
                let name = group.pointer("/data/name")?.as_str()?;
                Some((id.to_string(), name.to_string()))
            })
            .collect())
    }

    fn group_name(&self, group_id: &str) -> Result<Option<String>, PortError> {
        let url = format!("{}/api/groups/{}", self.base, urlencoding::encode(group_id));
        Ok(self.get_json(&url)?.and_then(|body| {
            body.pointer("/data/name")
                .and_then(Value::as_str)
                .map(str::to_string)
        }))
    }

    fn find_items(&self, library: &Library, url: &str) -> Result<Vec<Value>, PortError> {
        let request = format!(
            "{}/items?format=json&limit=100&qmode=fields&q={}",
            self.library_path(library),
            urlencoding::encode(url)
        );
        match self.get_json(&request)? {
            Some(Value::Array(items)) => Ok(items),
            Some(_) => Err(PortError::Invalid(
                "the search answer was not a list".into(),
            )),
            None => Err(PortError::Rejected {
                status: 404,
                detail: "the library does not exist in Zotero".into(),
            }),
        }
    }

    fn children(&self, library: &Library, key: &str) -> Result<Vec<Value>, PortError> {
        let request = format!(
            "{}/items/{}/children?format=json&limit=100",
            self.library_path(library),
            urlencoding::encode(key)
        );
        match self.get_json(&request)? {
            Some(Value::Array(items)) => Ok(items),
            Some(_) => Err(PortError::Invalid(
                "the children answer was not a list".into(),
            )),
            None => Ok(Vec::new()),
        }
    }

    fn save_item(&self, session: &str, uri: &str, item: Value) -> Result<(), PortError> {
        self.post_json(
            "/connector/saveItems",
            &json!({ "sessionID": session, "uri": uri, "items": [item] }),
            &[201],
        )
    }

    fn move_session(&self, session: &str, target: &str) -> Result<(), PortError> {
        self.post_json(
            "/connector/updateSession",
            &json!({ "sessionID": session, "target": target, "tags": [] }),
            &[200],
        )
    }

    fn attach_pdf(
        &self,
        session: &str,
        parent_item: &str,
        title: &str,
        url: &str,
        bytes: Vec<u8>,
    ) -> Result<bool, PortError> {
        let metadata = json!({
            "sessionID": session,
            "parentItemID": parent_item,
            "title": title,
            "url": url,
        });
        let response = self
            .client(Duration::from_secs(120))?
            .post(format!(
                "{}/connector/saveAttachment?sessionID={}",
                self.base,
                urlencoding::encode(session)
            ))
            .header("Content-Type", "application/pdf")
            .header("X-Metadata", metadata.to_string())
            .body(bytes)
            .send()
            .map_err(Self::reach)?;
        match response.status().as_u16() {
            201 => Ok(true),
            200 => Ok(false),
            _ => Err(Self::rejected(response)),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_server {
    //! A throwaway HTTP server: canned answers by method and path prefix, and a
    //! record of everything it was asked.
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    pub struct Seen {
        pub method: String,
        pub target: String,
        pub headers: HashMap<String, String>,
        pub body: Vec<u8>,
    }

    pub struct Canned {
        pub method: &'static str,
        pub prefix: &'static str,
        pub status: u16,
        pub body: String,
    }

    pub struct Server {
        pub base: String,
        pub seen: Arc<Mutex<Vec<Seen>>>,
    }

    pub fn canned(method: &'static str, prefix: &'static str, status: u16, body: &str) -> Canned {
        Canned {
            method,
            prefix,
            status,
            body: body.to_string(),
        }
    }

    pub fn serve(answers: Vec<Canned>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let head_end;
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        head_end = at + 4;
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
                let mut lines = head.lines();
                let request_line = lines.next().unwrap_or("").to_string();
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let headers: HashMap<String, String> = lines
                    .filter_map(|line| line.split_once(':'))
                    .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                let length: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let mut body = raw[head_end..].to_vec();
                while body.len() < length {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&buf[..n]);
                }
                log.lock().unwrap().push(Seen {
                    method: method.clone(),
                    target: target.clone(),
                    headers,
                    body,
                });
                let answer = answers
                    .iter()
                    .find(|a| a.method == method && target.starts_with(a.prefix));
                let (status, text) = match answer {
                    Some(a) => (a.status, a.body.clone()),
                    None => (404, String::new()),
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        Server { base, seen }
    }

    /// An address nothing listens on.
    pub fn dead_base() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        base
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::*;
    use super::*;
    use crate::writing::zotero::Library;
    use serde_json::json;

    #[test]
    fn ping_reads_the_running_banner() {
        let server = serve(vec![canned(
            "GET",
            "/connector/ping",
            200,
            "Zotero is running",
        )]);
        assert_eq!(ConnectorPort::new(&server.base).ping(), Ok(()));
        let silent = serve(vec![canned(
            "GET",
            "/connector/ping",
            200,
            "something else",
        )]);
        assert_eq!(
            ConnectorPort::new(&silent.base).ping(),
            Err(PortError::Unreachable)
        );
    }

    #[test]
    fn nothing_listening_is_unreachable_not_an_error_of_ours() {
        let port = ConnectorPort::new(&dead_base());
        assert_eq!(port.ping(), Err(PortError::Unreachable));
        assert_eq!(port.targets().unwrap_err(), PortError::Unreachable);
    }

    #[test]
    fn targets_name_the_editable_libraries_and_the_selected_one() {
        let body = json!({
            "libraryID": 3, "libraryName": "prueba", "id": 9, "name": "Col",
            "targets": [
                {"id": "L1", "name": "My Library", "level": 0},
                {"id": "C4", "name": "Folder", "level": 1},
                {"id": "L3", "name": "prueba", "level": 0},
                {"id": "C9", "name": "Col", "level": 1}
            ]
        })
        .to_string();
        let server = serve(vec![canned(
            "POST",
            "/connector/getSelectedCollection",
            200,
            &body,
        )]);
        let targets = ConnectorPort::new(&server.base).targets().unwrap();
        assert_eq!(targets.selected_library, "L3");
        assert_eq!(
            targets.libraries,
            vec![
                TargetLibrary {
                    id: "L1".into(),
                    name: "My Library".into()
                },
                TargetLibrary {
                    id: "L3".into(),
                    name: "prueba".into()
                },
            ]
        );
    }

    #[test]
    fn an_unreadable_target_answer_is_invalid() {
        let server = serve(vec![canned(
            "POST",
            "/connector/getSelectedCollection",
            200,
            "[]",
        )]);
        assert!(matches!(
            ConnectorPort::new(&server.base).targets(),
            Err(PortError::Invalid(_))
        ));
    }

    #[test]
    fn saving_an_item_posts_the_session_and_one_item() {
        let server = serve(vec![canned("POST", "/connector/saveItems", 201, "")]);
        let item = json!({"id": "e1", "itemType": "webpage", "title": "T"});
        ConnectorPort::new(&server.base)
            .save_item("sess-1", "https://a.test/x", item.clone())
            .unwrap();
        let seen = server.seen.lock().unwrap();
        let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert_eq!(body["sessionID"], "sess-1");
        assert_eq!(body["uri"], "https://a.test/x");
        assert_eq!(body["items"], json!([item]));
        assert!(seen[0].headers["content-type"].starts_with("application/json"));
    }

    #[test]
    fn a_refused_save_says_what_zotero_answered() {
        let server = serve(vec![canned("POST", "/connector/saveItems", 500, "")]);
        let error = ConnectorPort::new(&server.base)
            .save_item("s", "u", json!({}))
            .unwrap_err();
        assert!(
            matches!(error, PortError::Rejected { status: 500, .. }),
            "{error:?}"
        );
    }

    #[test]
    fn moving_the_session_names_the_target_library() {
        let server = serve(vec![canned("POST", "/connector/updateSession", 200, "{}")]);
        ConnectorPort::new(&server.base)
            .move_session("sess-1", "L3")
            .unwrap();
        let seen = server.seen.lock().unwrap();
        let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert_eq!(
            body,
            json!({"sessionID": "sess-1", "target": "L3", "tags": []})
        );
    }

    #[test]
    fn a_missing_session_is_a_rejection() {
        let server = serve(vec![canned(
            "POST",
            "/connector/updateSession",
            400,
            "{\"error\":\"SESSION_NOT_FOUND\"}",
        )]);
        let error = ConnectorPort::new(&server.base)
            .move_session("gone", "L1")
            .unwrap_err();
        assert!(matches!(error, PortError::Rejected { status: 400, .. }));
    }

    #[test]
    fn the_pdf_travels_as_raw_bytes_with_its_metadata_in_a_header() {
        let server = serve(vec![canned("POST", "/connector/saveAttachment", 201, "")]);
        let attached = ConnectorPort::new(&server.base)
            .attach_pdf(
                "sess-1",
                "e1",
                "Captura web abababab.pdf",
                "https://a.test/x.pdf",
                b"%PDF-1.4 hi".to_vec(),
            )
            .unwrap();
        assert!(attached);
        let seen = server.seen.lock().unwrap();
        assert!(seen[0]
            .target
            .starts_with("/connector/saveAttachment?sessionID=sess-1"));
        assert_eq!(seen[0].headers["content-type"], "application/pdf");
        let meta: serde_json::Value = serde_json::from_str(&seen[0].headers["x-metadata"]).unwrap();
        assert_eq!(
            meta,
            json!({"sessionID": "sess-1", "parentItemID": "e1",
                   "title": "Captura web abababab.pdf", "url": "https://a.test/x.pdf"})
        );
        assert_eq!(seen[0].body, b"%PDF-1.4 hi");
    }

    #[test]
    fn a_library_that_does_not_take_files_is_reported_not_attached() {
        let server = serve(vec![canned(
            "POST",
            "/connector/saveAttachment",
            200,
            "Library files are not editable.",
        )]);
        let attached = ConnectorPort::new(&server.base)
            .attach_pdf("s", "e", "t", "u", vec![1])
            .unwrap();
        assert!(!attached);
    }

    #[test]
    fn items_are_searched_in_the_named_library_with_a_bounded_query() {
        let server = serve(vec![canned(
            "GET",
            "/api/groups/6680944/items",
            200,
            "[{\"key\":\"K\"}]",
        )]);
        let hits = ConnectorPort::new(&server.base)
            .find_items(&Library::group("6680944"), "https://a.test/x?y=1")
            .unwrap();
        assert_eq!(hits, vec![json!({"key": "K"})]);
        let seen = server.seen.lock().unwrap();
        assert!(seen[0].target.contains("format=json"));
        assert!(seen[0].target.contains("limit=100"));
        assert!(seen[0].target.contains("qmode=fields"));
        assert!(seen[0]
            .target
            .contains("q=https%3A%2F%2Fa.test%2Fx%3Fy%3D1"));
    }

    #[test]
    fn the_personal_library_is_searched_under_users() {
        let server = serve(vec![canned("GET", "/api/users/0/items", 200, "[]")]);
        let hits = ConnectorPort::new(&server.base)
            .find_items(&Library::personal(), "u")
            .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn a_disabled_local_api_is_a_rejection_the_run_can_explain() {
        let server = serve(vec![canned("GET", "/api/users/0/items", 403, "")]);
        let error = ConnectorPort::new(&server.base)
            .find_items(&Library::personal(), "u")
            .unwrap_err();
        assert!(matches!(error, PortError::Rejected { status: 403, .. }));
    }

    #[test]
    fn children_and_group_names_come_from_the_local_api() {
        let server = serve(vec![
            canned(
                "GET",
                "/api/users/0/items/ABCD2345/children",
                200,
                "[{\"data\":{\"title\":\"a\"}}]",
            ),
            canned(
                "GET",
                "/api/groups/7",
                200,
                "{\"data\":{\"name\":\"prueba\"}}",
            ),
            canned("GET", "/api/groups/8", 404, ""),
        ]);
        let port = ConnectorPort::new(&server.base);
        assert_eq!(
            port.children(&Library::personal(), "ABCD2345")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(port.group_name("7").unwrap().as_deref(), Some("prueba"));
        assert_eq!(port.group_name("8").unwrap(), None);
    }

    #[test]
    fn the_users_groups_are_listed_by_id_and_name() {
        let body = json!([
            {"id": 6680944, "data": {"id": 6680944, "name": "prueba"}},
            {"id": 7, "data": {"name": "otra"}},
            {"id": 8, "data": {}}
        ])
        .to_string();
        let server = serve(vec![canned("GET", "/api/users/0/groups", 200, &body)]);
        let groups = ConnectorPort::new(&server.base).groups().unwrap();
        assert_eq!(
            groups,
            vec![
                ("6680944".to_string(), "prueba".to_string()),
                ("7".to_string(), "otra".to_string())
            ]
        );
    }
}
