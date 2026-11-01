//! Endpoint-level tests for the read-only tools (T3, D10).
//!
//! They drive the real loopback endpoint with reqwest against a fixture archive
//! built by the real migrations, so what they assert is what an assistant gets:
//! HTTP status, `isError`, the structured payload, the audit file on disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use super::access::{tests::MemorySecrets, AgentAccess, Scope};
use super::audit::AUDIT_FILE;
use super::mcp::{
    ERROR_FORBIDDEN_SCOPE, ERROR_INVALID_ARGUMENT, ERROR_NOT_FOUND, ERROR_UNAVAILABLE,
};
use super::server::{app_version, variant_name, AgentEndpointState, LOOPBACK, MCP_PATH, PROTOCOL};
use super::test_archive::{self, ALFA, ALFA_ITEMS, BETA, BETA_ITEMS};

struct Harness {
    root: tempfile::TempDir,
    #[allow(dead_code)]
    secrets: Arc<MemorySecrets>,
    access: Arc<AgentAccess>,
    endpoint: Arc<AgentEndpointState>,
    client: reqwest::Client,
    port: u16,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.endpoint.stop();
    }
}

impl Harness {
    fn cache(&self) -> PathBuf {
        self.root.path().join("cache")
    }

    fn audit_path(&self) -> PathBuf {
        self.cache().join("logs").join(AUDIT_FILE)
    }

    /// Pair a client and return its id and its token, which only the secret
    /// store holds in production.
    fn pair(&self, name: &str) -> (String, String) {
        let client = self.access.create_client(name).unwrap();
        let token = self.secrets.token(&client.id).expect("stored token");
        (client.id, token)
    }

    async fn rpc(&self, token: &str, body: Value) -> Value {
        let response = self
            .client
            .post(format!("http://{LOOPBACK}:{}{MCP_PATH}", self.port))
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .expect("the endpoint answers");
        assert_eq!(
            response.status(),
            reqwest::StatusCode::OK,
            "the guard let the paired client through"
        );
        let body: Value = response.json().await.expect("a JSON-RPC response");
        assert!(
            body.get("error").is_none(),
            "a tool never fails at the protocol level: {body}"
        );
        body
    }

    /// `tools/call` for a paired client, returning the whole JSON-RPC response.
    async fn call(&self, token: &str, tool: &str, arguments: Value) -> Value {
        self.rpc(
            token,
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": { "name": tool, "arguments": arguments }
            }),
        )
        .await
    }

    /// `tools/call` that must have answered normally, returning the structured
    /// payload.
    async fn payload(&self, token: &str, tool: &str, arguments: Value) -> Value {
        let body = self.call(token, tool, arguments).await;
        assert_eq!(
            body["result"]["isError"], false,
            "{tool} was expected to succeed: {body}"
        );
        body["result"]["structuredContent"].clone()
    }

    /// `tools/call` that must have been refused, returning the structured error.
    async fn refusal(&self, token: &str, tool: &str, arguments: Value) -> Value {
        let body = self.call(token, tool, arguments).await;
        assert_eq!(
            body["result"]["isError"], true,
            "{tool} was expected to be refused: {body}"
        );
        body["result"]["structuredContent"].clone()
    }

    fn audit_lines(&self) -> Vec<Value> {
        std::fs::read_to_string(self.audit_path())
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("every audit line is JSON"))
            .collect()
    }
}

async fn harness() -> Harness {
    let root = tempfile::tempdir().unwrap();
    let db_path = test_archive::seeded_archive(root.path());
    let secrets = Arc::new(MemorySecrets::default());
    let access = Arc::new(AgentAccess::load(root.path(), secrets.clone()));
    access.set_enabled(true).unwrap();
    let endpoint = Arc::new(AgentEndpointState::new(
        root.path().to_path_buf(),
        db_path,
        root.path().join("cache"),
    ));
    endpoint.sync(&access).await.unwrap();
    let port = endpoint.port().expect("the endpoint is running");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    Harness {
        root,
        secrets,
        access,
        endpoint,
        client,
        port,
    }
}

/// The keys of a JSON object, sorted, so a test can assert an exact field set.
fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .unwrap_or_else(|| panic!("expected an object, got {value}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = names.iter().map(|name| (*name).to_string()).collect();
    names.sort();
    names
}

fn records(payload: &Value) -> Vec<Value> {
    payload["records"]
        .as_array()
        .unwrap_or_else(|| panic!("expected a records array: {payload}"))
        .clone()
}

fn ids(payload: &Value) -> Vec<String> {
    records(payload)
        .iter()
        .map(|record| {
            record["id"]
                .as_str()
                .expect("every record has an id")
                .to_string()
        })
        .collect()
}

#[tokio::test]
async fn entropia_status_reports_the_archive_counts_and_the_tool_list() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    let status = harness.payload(&token, "entropia_status", json!({})).await;

    assert_eq!(
        keys(&status),
        sorted(&[
            "protocol",
            "appVersion",
            "variant",
            "profile",
            "client",
            "archive",
            "tools"
        ])
    );
    assert_eq!(status["protocol"], PROTOCOL);
    assert_eq!(status["appVersion"], app_version());
    assert_eq!(status["variant"], variant_name());
    assert_eq!(status["client"]["id"], id);
    assert_eq!(status["client"]["name"], "Claude Code");
    assert_eq!(status["client"]["scopes"], json!(["read"]));
    assert_eq!(
        status["archive"],
        json!({ "collections": 2, "items": ALFA_ITEMS + BETA_ITEMS })
    );
    assert_eq!(
        status["tools"],
        json!([
            "collections_list",
            "entropia_status",
            "items_get",
            "items_list",
            "search_items"
        ])
    );
}

#[tokio::test]
async fn collections_list_reports_every_field_of_a_record() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Codex");
    let page = harness.payload(&token, "collections_list", json!({})).await;

    assert_eq!(keys(&page), sorted(&["records", "nextCursor"]));
    assert_eq!(page["nextCursor"], Value::Null);
    let records = records(&page);
    assert_eq!(records.len(), 2, "the fixture has two collections");
    assert_eq!(
        records[0],
        json!({
            "id": ALFA,
            "name": "Alfa",
            "description": "Primera colección",
            "itemCount": ALFA_ITEMS,
            "createdAt": 1_000,
            "updatedAt": 2_000,
        })
    );
    assert_eq!(
        keys(&records[0]),
        sorted(&[
            "id",
            "name",
            "description",
            "itemCount",
            "createdAt",
            "updatedAt"
        ])
    );
    assert_eq!(records[1]["id"], BETA);
    assert_eq!(records[1]["description"], Value::Null);
    assert_eq!(records[1]["itemCount"], BETA_ITEMS);
}

#[tokio::test]
async fn items_list_reports_scoped_records_with_their_revision() {
    let harness = harness().await;
    let (_id, token) = harness.pair("OpenCode");

    let every = harness.payload(&token, "items_list", json!({})).await;
    assert_eq!(keys(&every), sorted(&["records", "nextCursor"]));
    assert_eq!(every["nextCursor"], Value::Null);
    assert_eq!(
        ids(&every),
        vec![
            test_archive::item_id(ALFA, 1),
            test_archive::item_id(ALFA, 2),
            test_archive::item_id(ALFA, 3),
            test_archive::item_id(ALFA, 4),
            test_archive::item_id(ALFA, 5),
            test_archive::item_id(BETA, 1),
            test_archive::item_id(BETA, 2),
        ],
        "ordered by title, then id"
    );
    assert_eq!(
        records(&every)[0],
        json!({
            "id": test_archive::item_id(ALFA, 1),
            "title": "Documento 1",
            "collectionId": ALFA,
            "updatedAt": 3_001,
        })
    );
    assert_eq!(
        keys(&records(&every)[0]),
        sorted(&["id", "title", "collectionId", "updatedAt"])
    );

    let scoped = harness
        .payload(&token, "items_list", json!({ "collectionId": BETA }))
        .await;
    assert_eq!(
        ids(&scoped),
        vec![
            test_archive::item_id(BETA, 1),
            test_archive::item_id(BETA, 2)
        ]
    );
    assert!(records(&scoped)
        .iter()
        .all(|record| record["collectionId"] == BETA));
}

#[tokio::test]
async fn pagination_walks_every_record_once_and_ends_with_a_null_cursor() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Pi");

    let mut seen: Vec<String> = Vec::new();
    let mut cursor = Value::Null;
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages < 10, "the walk must terminate");
        let arguments = if cursor.is_null() {
            json!({ "limit": 2 })
        } else {
            json!({ "limit": 2, "cursor": cursor })
        };
        let page = harness.payload(&token, "items_list", arguments).await;
        let page_ids = ids(&page);
        assert!(page_ids.len() <= 2, "the page honours the limit");
        seen.extend(page_ids);
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(pages, 4, "seven items at two per page");
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "no record repeats: {seen:?}");
    assert_eq!(seen.len(), ALFA_ITEMS + BETA_ITEMS, "no record is skipped");

    let first = harness
        .payload(&token, "collections_list", json!({ "limit": 1 }))
        .await;
    assert_eq!(ids(&first), vec![ALFA]);
    let cursor = first["nextCursor"].as_str().expect("a next page exists");
    let second = harness
        .payload(
            &token,
            "collections_list",
            json!({ "limit": 1, "cursor": cursor }),
        )
        .await;
    assert_eq!(ids(&second), vec![BETA]);
    assert_eq!(
        second["nextCursor"],
        Value::Null,
        "the last page ends the walk"
    );
}

#[tokio::test]
async fn a_limit_outside_the_documented_range_is_refused_as_invalid_argument() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");

    for arguments in [json!({ "limit": 0 }), json!({ "limit": 201 })] {
        let error = harness
            .refusal(&token, "items_list", arguments.clone())
            .await;
        assert_eq!(
            keys(&error),
            sorted(&["error", "message"]),
            "every refusal is an error and a message: {error}"
        );
        assert_eq!(error["error"], ERROR_INVALID_ARGUMENT, "{arguments}");
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|message| message.contains("entre 1 y 200")),
            "{error}"
        );
    }

    let malformed = harness
        .refusal(&token, "items_list", json!({ "cursor": "no-es-un-cursor" }))
        .await;
    assert_eq!(malformed["error"], ERROR_INVALID_ARGUMENT);

    let at_the_maximum = harness
        .payload(&token, "items_list", json!({ "limit": 200 }))
        .await;
    assert_eq!(at_the_maximum["nextCursor"], Value::Null);
}

#[tokio::test]
async fn an_unknown_input_field_is_refused_as_invalid_argument() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Codex");

    let error = harness
        .refusal(&token, "items_list", json!({ "collectionID": BETA }))
        .await;
    assert_eq!(error["error"], ERROR_INVALID_ARGUMENT);
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("collectionID")),
        "the message names the field: {error}"
    );
    assert!(
        error.get("records").is_none(),
        "a refused call returns no data: {error}"
    );

    let status = harness
        .refusal(&token, "entropia_status", json!({ "verbose": true }))
        .await;
    assert_eq!(status["error"], ERROR_INVALID_ARGUMENT);
}

#[tokio::test]
async fn a_client_without_read_scope_is_refused_and_gets_no_data() {
    let harness = harness().await;
    let (id, token) = harness.pair("Pi");
    harness.access.set_scopes(&id, Vec::<Scope>::new()).unwrap();

    for (tool, arguments) in [
        ("entropia_status", json!({})),
        ("collections_list", json!({})),
        ("items_list", json!({})),
        (
            "items_get",
            json!({ "itemId": test_archive::item_id(ALFA, 1) }),
        ),
        ("search_items", json!({ "query": "revista" })),
    ] {
        let error = harness.refusal(&token, tool, arguments).await;
        assert_eq!(error["error"], ERROR_FORBIDDEN_SCOPE, "{tool}");
        assert_eq!(keys(&error), sorted(&["error", "message"]));
        assert!(
            error.get("records").is_none() && error.get("archive").is_none(),
            "{tool} leaked data: {error}"
        );
    }
    let lines = harness.audit_lines();
    assert_eq!(lines.len(), 5, "every refusal is audited: {lines:?}");
    assert!(
        lines
            .iter()
            .all(|line| line["status"] == "error" && line["errorCode"] == ERROR_FORBIDDEN_SCOPE),
        "and audited as a refusal, with no object ids: {lines:?}"
    );
}

#[tokio::test]
async fn an_unknown_item_id_is_not_found() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");

    let error = harness
        .refusal(&token, "items_get", json!({ "itemId": "ite_no_existe" }))
        .await;
    assert_eq!(error["error"], ERROR_NOT_FOUND);
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("ite_no_existe")),
        "{error}"
    );
    assert!(error.get("assets").is_none(), "no record, no data: {error}");
}

#[tokio::test]
async fn items_get_returns_metadata_and_assets_but_never_the_extracted_text() {
    let harness = harness().await;
    let (_id, token) = harness.pair("OpenCode");
    let item = test_archive::item_id(ALFA, 1);

    let detail = harness
        .payload(&token, "items_get", json!({ "itemId": item }))
        .await;
    assert_eq!(
        keys(&detail),
        sorted(&[
            "id",
            "title",
            "collectionId",
            "metadata",
            "createdAt",
            "updatedAt",
            "assets"
        ])
    );
    assert_eq!(detail["id"], item);
    assert_eq!(detail["title"], "Documento 1");
    assert_eq!(detail["collectionId"], ALFA);
    assert_eq!(detail["metadata"], json!({ "fuente": "prueba" }));
    assert_eq!(detail["updatedAt"], 3_001);
    assert_eq!(
        detail["assets"],
        json!([
            {
                "id": "ast_alfa_1",
                "type": "pdf",
                "path": "assets/alfa-1.pdf",
                "sortIndex": 0,
                "size": 1_234,
                "pageNumber": null,
                "createdAt": 1_500,
            },
            {
                "id": "ast_alfa_2",
                "type": "image",
                "path": "assets/alfa-1-cover.jpg",
                "sortIndex": 1,
                "size": 99,
                "pageNumber": null,
                "createdAt": 1_501,
            }
        ])
    );
    let serialized = detail.to_string();
    for forbidden in ["documento de prueba", "textContent", "extractedText"] {
        assert!(
            !serialized.contains(forbidden),
            "items_get must not return the text ({forbidden}): {serialized}"
        );
    }
}

#[tokio::test]
async fn search_items_is_scoped_to_a_collection_and_reports_the_total() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Pi");

    let every = harness
        .payload(&token, "search_items", json!({ "query": "prueba" }))
        .await;
    assert_eq!(keys(&every), sorted(&["records", "matches"]));
    assert_eq!(every["matches"], ALFA_ITEMS + BETA_ITEMS);
    assert_eq!(records(&every).len(), ALFA_ITEMS + BETA_ITEMS);
    assert_eq!(
        keys(&records(&every)[0]),
        sorted(&["id", "title", "collectionId", "updatedAt", "rank"])
    );

    let scoped = harness
        .payload(
            &token,
            "search_items",
            json!({ "query": "prueba", "collectionId": BETA }),
        )
        .await;
    assert_eq!(scoped["matches"], BETA_ITEMS);
    assert!(
        records(&scoped)
            .iter()
            .all(|record| record["collectionId"] == BETA),
        "a scoped search returns only that collection: {scoped}"
    );

    let keyword = harness
        .payload(&token, "search_items", json!({ "query": "revista" }))
        .await;
    assert_eq!(
        ids(&keyword),
        vec![
            test_archive::item_id(BETA, 1),
            test_archive::item_id(BETA, 2)
        ]
    );

    let truncated = harness
        .payload(
            &token,
            "search_items",
            json!({ "query": "prueba", "limit": 3 }),
        )
        .await;
    assert_eq!(records(&truncated).len(), 3);
    assert_eq!(
        truncated["matches"],
        ALFA_ITEMS + BETA_ITEMS,
        "the caller learns how much it did not see"
    );

    let blank = harness
        .refusal(&token, "search_items", json!({ "query": "   " }))
        .await;
    assert_eq!(blank["error"], ERROR_INVALID_ARGUMENT);
}

#[tokio::test]
async fn every_call_appends_one_audit_line_without_the_query_or_the_token() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    let item = test_archive::item_id(ALFA, 1);

    harness
        .payload(
            &token,
            "search_items",
            json!({ "query": "revista secreta" }),
        )
        .await;
    harness
        .payload(&token, "items_get", json!({ "itemId": item }))
        .await;
    harness
        .refusal(&token, "items_list", json!({ "limit": 201 }))
        .await;

    let lines = harness.audit_lines();
    assert_eq!(lines.len(), 3, "one line per call: {lines:?}");

    assert_eq!(
        keys(&lines[0]),
        sorted(&[
            "ts",
            "clientId",
            "clientName",
            "tool",
            "status",
            "objectIds",
            "queryChars"
        ])
    );
    assert_eq!(lines[0]["clientId"], id);
    assert_eq!(lines[0]["clientName"], "Claude Code");
    assert_eq!(lines[0]["tool"], "search_items");
    assert_eq!(lines[0]["status"], "ok");
    assert_eq!(
        lines[0]["objectIds"],
        json!([
            test_archive::item_id(BETA, 1),
            test_archive::item_id(BETA, 2)
        ])
    );
    assert_eq!(lines[0]["queryChars"], "revista secreta".chars().count());
    assert!(lines[0]["ts"].as_i64().is_some_and(|ts| ts > 0));

    assert_eq!(lines[1]["tool"], "items_get");
    assert_eq!(lines[1]["objectIds"], json!([item]));
    assert!(lines[1].get("queryChars").is_none(), "absent, not null");

    assert_eq!(lines[2]["tool"], "items_list");
    assert_eq!(lines[2]["status"], "error");
    assert_eq!(lines[2]["errorCode"], ERROR_INVALID_ARGUMENT);
    assert_eq!(lines[2]["objectIds"], json!([]));

    let raw = std::fs::read_to_string(harness.audit_path()).expect("the audit file exists");
    assert!(
        !raw.contains("revista secreta"),
        "the query never reaches the audit file"
    );
    assert!(!raw.contains(&token), "the token never reaches it either");
}

/// The audit trail is written under the cache the endpoint was built with, next
/// to (not inside) the archive directory.
#[tokio::test]
async fn the_audit_trail_lives_under_the_cache_directory() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Codex");
    harness.payload(&token, "collections_list", json!({})).await;

    let path = harness.audit_path();
    assert!(path.starts_with(harness.cache()));
    assert_eq!(
        path.parent().and_then(Path::file_name),
        Some(std::ffi::OsStr::new("logs"))
    );
}

/// Every tool says it only reads and touches nothing outside this machine.
/// Clients act on this: Codex cancels a non-interactive call to a tool that
/// does not declare itself read-only, because it cannot ask for approval.
#[tokio::test]
async fn every_tool_declares_itself_read_only_and_closed_world() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Codex");
    let body = harness
        .rpc(
            &token,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        )
        .await;
    let tools = body["result"]["tools"].as_array().expect("tools").clone();
    assert_eq!(tools.len(), 5);
    for tool in tools {
        let name = tool["name"].as_str().unwrap_or_default().to_string();
        let annotations = &tool["annotations"];
        assert_eq!(annotations["readOnlyHint"], true, "{name}");
        assert_eq!(annotations["openWorldHint"], false, "{name}");
    }
}

#[tokio::test]
async fn every_tool_publishes_a_strict_input_schema() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Pi");
    let body = harness
        .rpc(
            &token,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        )
        .await;
    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools/list answers with an array")
        .clone();

    let expected: [(&str, &[&str]); 5] = [
        ("collections_list", &["cursor", "limit"]),
        ("entropia_status", &[]),
        ("items_get", &["itemId"]),
        ("items_list", &["collectionId", "cursor", "limit"]),
        ("search_items", &["collectionId", "limit", "query"]),
    ];
    assert_eq!(tools.len(), expected.len(), "no tool beyond D10's list");
    for (tool, properties) in expected {
        let published = tools
            .iter()
            .find(|candidate| candidate["name"] == tool)
            .unwrap_or_else(|| panic!("{tool} is published"));
        let schema = &published["inputSchema"];
        assert_eq!(schema["type"], "object", "{tool}");
        assert_eq!(
            schema["additionalProperties"], false,
            "{tool}: the schema itself refuses unknown fields (S6)"
        );
        let mut names: Vec<&str> = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| properties.keys().map(String::as_str).collect())
            .unwrap_or_default();
        names.sort_unstable();
        let mut expected_names = properties.to_vec();
        expected_names.sort_unstable();
        assert_eq!(names, expected_names, "{tool}");
    }
}

/// The codes a refusal carries are the ones the tool contract publishes; a read
/// failure is `unavailable` (S7's "capacidad no disponible").
#[test]
fn the_read_errors_report_the_codes_the_tool_contract_publishes() {
    use super::read::ReadError;
    assert_eq!(
        ReadError::InvalidArgument(String::new()).code(),
        ERROR_INVALID_ARGUMENT
    );
    assert_eq!(ReadError::NotFound(String::new()).code(), ERROR_NOT_FOUND);
    assert_eq!(
        ReadError::Unavailable(String::new()).code(),
        ERROR_UNAVAILABLE
    );
}
