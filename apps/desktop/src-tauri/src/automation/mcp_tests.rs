//! Endpoint-level tests for the read-only tools (T3, D10).
//!
//! They drive the real loopback endpoint with reqwest against a fixture archive
//! built by the real migrations, so what they assert is what an assistant gets:
//! HTTP status, `isError`, the structured payload, the audit file on disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use super::access::{tests::MemorySecrets, AgentAccess, Grants, Scope};
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

    fn db_path(&self) -> PathBuf {
        self.root.path().join("entropia.sqlite")
    }

    /// Prepare an OCR draft over one collection through the same service the
    /// Procesamiento tab uses, returning its batch id.
    fn prepare(&self, request_id: &str, collection: &str) -> String {
        let conn = crate::processing::commands::open_ready(&self.db_path()).unwrap();
        crate::processing::commands::prepare_batch(
            &conn,
            request_id,
            &[collection.to_string()],
            &[],
            &["ocr".to_string()],
        )
        .unwrap()
        .batch_id
    }

    /// Move a prepared job to `ready`, the state planning leaves it in, and
    /// return its revision.
    fn mark_ready(&self, job_id: &str) -> i64 {
        let conn = crate::processing::commands::open_ready(&self.db_path()).unwrap();
        conn.execute(
            "UPDATE processing_batches SET state = 'ready', planning_done = 1 WHERE id = ?1",
            [job_id],
        )
        .unwrap();
        crate::processing::commands::batch_snapshot(&conn, job_id)
            .unwrap()
            .revision
    }

    /// A job the owner started and the scheduler picked up.
    fn mark_running(&self, job_id: &str) {
        let conn = crate::processing::commands::open_ready(&self.db_path()).unwrap();
        conn.execute(
            "UPDATE processing_batches SET state = 'running', desired_state = 'run', planning_done = 1 WHERE id = ?1",
            [job_id],
        )
        .unwrap();
    }

    fn desired_state(&self, job_id: &str) -> String {
        let conn = crate::processing::commands::open_ready(&self.db_path()).unwrap();
        crate::processing::commands::batch_snapshot(&conn, job_id)
            .unwrap()
            .desired_state
    }

    /// The owner's answer in the app.
    fn owner_decides(&self, approval_id: &str, approve: bool) {
        let conn = crate::processing::commands::open_ready(&self.db_path()).unwrap();
        self.endpoint
            .approvals()
            .decide(&conn, approval_id, approve)
            .unwrap();
    }

    /// Give a paired client the processing grants the owner sets in Asistentes.
    fn grant(&self, client_id: &str, local_write: bool, remote_spend: bool) {
        self.access
            .set_grants(
                client_id,
                Grants {
                    local_write,
                    remote_spend,
                },
            )
            .unwrap();
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
            "approvals_status",
            "collections_list",
            "documents_get_text",
            "entropia_status",
            "items_get",
            "items_list",
            "jobs_get",
            "jobs_list",
            "processing_control",
            "processing_prepare",
            "processing_start",
            "search_items",
            "search_passages"
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
/// does not declare itself read-only, because it cannot ask for approval. So
/// every reading tool says it is read-only, and a tool that changes the archive
/// says it does, without claiming to destroy anything.
#[tokio::test]
async fn every_tool_declares_whether_it_changes_the_archive_and_stays_closed_world() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Codex");
    let body = harness
        .rpc(
            &token,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        )
        .await;
    let tools = body["result"]["tools"].as_array().expect("tools").clone();
    assert_eq!(tools.len(), 13);
    let writes = [
        "processing_prepare",
        "processing_start",
        "processing_control",
    ];
    for tool in tools {
        let name = tool["name"].as_str().unwrap_or_default().to_string();
        let annotations = &tool["annotations"];
        let writes_archive = writes.contains(&name.as_str());
        assert_eq!(annotations["readOnlyHint"], !writes_archive, "{name}");
        if writes_archive {
            assert_eq!(annotations["destructiveHint"], false, "{name}");
        }
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

    let expected: [(&str, &[&str]); 13] = [
        ("approvals_status", &["approvalId"]),
        ("collections_list", &["cursor", "limit"]),
        (
            "documents_get_text",
            &["assetId", "cursor", "itemId", "maxChars"],
        ),
        ("entropia_status", &[]),
        ("items_get", &["itemId"]),
        ("items_list", &["collectionId", "cursor", "limit"]),
        (
            "jobs_get",
            &[
                "jobId",
                "taskKind",
                "taskState",
                "tasksCursor",
                "tasksLimit",
            ],
        ),
        ("jobs_list", &["cursor", "limit", "states"]),
        (
            "processing_control",
            &["action", "failedOnly", "jobId", "revision", "taskId"],
        ),
        (
            "processing_prepare",
            &["collectionIds", "itemIds", "operations", "requestId"],
        ),
        ("processing_start", &["jobId", "revision"]),
        ("search_items", &["collectionId", "limit", "query"]),
        (
            "search_passages",
            &["collectionId", "itemId", "limit", "query"],
        ),
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

#[tokio::test]
async fn jobs_list_pages_the_users_batches_with_every_field_once() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");
    let first = harness.prepare("req-a", ALFA);
    let second = harness.prepare("req-b", BETA);

    let page = harness
        .payload(&token, "jobs_list", json!({ "limit": 1 }))
        .await;
    assert_eq!(keys(&page), sorted(&["records", "nextCursor"]));
    let record = &records(&page)[0];
    assert_eq!(
        keys(record),
        sorted(&[
            "id",
            "state",
            "desiredState",
            "operations",
            "revision",
            "priority",
            "createdAt",
            "updatedAt",
            "activeUnits",
            "failedUnits",
            "succeededUnits",
        ])
    );
    assert_eq!(record["state"], "preparing");
    assert_eq!(record["operations"], json!(["ocr"]));
    let cursor = page["nextCursor"]
        .as_str()
        .expect("one more page")
        .to_string();

    let rest = harness
        .payload(&token, "jobs_list", json!({ "limit": 1, "cursor": cursor }))
        .await;
    assert_eq!(rest["nextCursor"], Value::Null, "two batches, two pages");
    let mut seen = vec![ids(&page)[0].clone(), ids(&rest)[0].clone()];
    seen.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(seen, expected, "each batch exactly once");
}

#[tokio::test]
async fn jobs_get_reports_the_batch_its_scope_and_its_tasks() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");
    let batch = harness.prepare("req-a", ALFA);

    let detail = harness
        .payload(&token, "jobs_get", json!({ "jobId": batch }))
        .await;
    assert_eq!(keys(&detail), sorted(&["job", "tasks", "nextTasksCursor"]));
    let job = &detail["job"];
    assert_eq!(job["id"], batch);
    assert_eq!(job["state"], "preparing");
    assert_eq!(job["desiredState"], "pause");
    assert_eq!(job["operations"], json!(["ocr"]));
    assert_eq!(job["membersTotal"], 2);
    assert_eq!(job["collections"], json!([{ "id": ALFA, "name": "Alfa" }]));
    assert_eq!(detail["tasks"], json!([]), "no task exists before planning");
    assert_eq!(detail["nextTasksCursor"], Value::Null);

    let last = harness.audit_lines().last().cloned().expect("audited");
    assert_eq!(last["tool"], "jobs_get");
    assert_eq!(last["status"], "ok");
    assert_eq!(last["objectIds"], json!([batch]));
}

#[tokio::test]
async fn jobs_get_names_an_unknown_job_not_found() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");
    let refusal = harness
        .refusal(&token, "jobs_get", json!({ "jobId": "batch-missing" }))
        .await;
    assert_eq!(refusal["error"], ERROR_NOT_FOUND);
}

#[tokio::test]
async fn jobs_tools_refuse_unknown_states_and_out_of_range_limits() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");
    let batch = harness.prepare("req-a", ALFA);
    for (tool, arguments) in [
        ("jobs_list", json!({ "states": ["exploded"] })),
        ("jobs_list", json!({ "limit": 0 })),
        ("jobs_list", json!({ "cursor": "not-a-cursor" })),
        ("jobs_get", json!({ "jobId": batch, "tasksLimit": 201 })),
        (
            "jobs_get",
            json!({ "jobId": batch, "taskState": "exploded" }),
        ),
    ] {
        let refusal = harness.refusal(&token, tool, arguments.clone()).await;
        assert_eq!(
            refusal["error"], ERROR_INVALID_ARGUMENT,
            "{tool} {arguments}"
        );
    }
}

#[tokio::test]
async fn jobs_tools_need_the_read_scope() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.access.set_scopes(&id, vec![]).unwrap();
    let refusal = harness.refusal(&token, "jobs_list", json!({})).await;
    assert_eq!(refusal["error"], ERROR_FORBIDDEN_SCOPE);
}

fn prepare_args(request_id: &str) -> Value {
    json!({ "requestId": request_id, "collectionIds": [ALFA], "operations": ["ocr"] })
}

#[tokio::test]
async fn processing_prepare_needs_the_local_write_grant_and_creates_nothing_without_it() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");
    let refusal = harness
        .refusal(&token, "processing_prepare", prepare_args("r1"))
        .await;
    assert_eq!(refusal["error"], ERROR_FORBIDDEN_SCOPE);
    let jobs = harness.payload(&token, "jobs_list", json!({})).await;
    assert!(records(&jobs).is_empty(), "nothing was prepared");
    let last = harness
        .audit_lines()
        .into_iter()
        .rev()
        .find(|line| line["tool"] == "processing_prepare")
        .expect("audited");
    assert_eq!(last["status"], "error");
    assert_eq!(last["errorCode"], ERROR_FORBIDDEN_SCOPE);
}

#[tokio::test]
async fn processing_prepare_creates_a_paused_draft_and_a_replay_returns_it() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, false);

    let first = harness
        .payload(&token, "processing_prepare", prepare_args("r1"))
        .await;
    assert_eq!(
        keys(&first),
        sorted(&[
            "jobId",
            "created",
            "files",
            "operations",
            "state",
            "revision",
            "costNote",
            "nextStep",
        ])
    );
    assert_eq!(first["created"], true);
    assert_eq!(first["files"], 2);
    assert_eq!(first["operations"], json!(["ocr"]));
    assert_eq!(first["state"], "preparing");
    let job_id = first["jobId"].as_str().expect("a job id").to_string();

    let replay = harness
        .payload(&token, "processing_prepare", prepare_args("r1"))
        .await;
    assert_eq!(
        replay["jobId"], job_id,
        "a retried request returns the same job"
    );
    assert_eq!(replay["created"], false);
    let jobs = harness.payload(&token, "jobs_list", json!({})).await;
    assert_eq!(ids(&jobs), vec![job_id.clone()], "never a second batch");

    let conn = crate::processing::commands::open_ready(&harness.db_path()).unwrap();
    assert!(
        crate::processing::repository::find_request(&conn, &format!("agent:{id}:r1"))
            .unwrap()
            .is_some(),
        "the request id is stored under the client that sent it"
    );
    let last = harness
        .audit_lines()
        .into_iter()
        .rev()
        .find(|line| line["tool"] == "processing_prepare")
        .expect("audited");
    assert_eq!(last["objectIds"], json!([job_id]));
}

#[tokio::test]
async fn two_assistants_with_the_same_request_id_get_separate_jobs() {
    let harness = harness().await;
    let (first_id, first_token) = harness.pair("Claude Code");
    let (second_id, second_token) = harness.pair("Codex");
    harness.grant(&first_id, true, false);
    harness.grant(&second_id, true, false);
    let first = harness
        .payload(&first_token, "processing_prepare", prepare_args("r1"))
        .await;
    let second = harness
        .payload(&second_token, "processing_prepare", prepare_args("r1"))
        .await;
    assert_ne!(first["jobId"], second["jobId"]);
    assert_eq!(second["created"], true);
}

#[tokio::test]
async fn processing_prepare_takes_items_and_refuses_bad_requests() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, false);
    let by_item = harness
        .payload(
            &token,
            "processing_prepare",
            json!({
                "requestId": "r-items",
                "itemIds": [test_archive::item_id(ALFA, 1)],
                "operations": ["ocr"]
            }),
        )
        .await;
    assert_eq!(by_item["files"], 2);

    let long_id = "x".repeat(129);
    for arguments in [
        json!({ "requestId": "r-items", "collectionIds": [BETA], "operations": ["ocr"] }),
        json!({ "requestId": "r2", "collectionIds": [ALFA], "operations": ["shell"] }),
        json!({ "requestId": "r3", "operations": ["ocr"] }),
        json!({ "requestId": "", "collectionIds": [ALFA], "operations": ["ocr"] }),
        json!({ "requestId": long_id, "collectionIds": [ALFA], "operations": ["ocr"] }),
        json!({ "requestId": "r4", "collectionIds": ["col_missing"], "operations": ["ocr"] }),
    ] {
        let refusal = harness
            .refusal(&token, "processing_prepare", arguments.clone())
            .await;
        assert_eq!(refusal["error"], ERROR_INVALID_ARGUMENT, "{arguments}");
    }
    let jobs = harness.payload(&token, "jobs_list", json!({})).await;
    assert_eq!(
        records(&jobs).len(),
        1,
        "no refused request created a batch"
    );
}

/// Lite calls OpenRouter for every text operation; the Procesamiento tab
/// blocks them without the key, and so does the tool (S7).
#[cfg(not(feature = "local-ml"))]
#[tokio::test]
async fn lite_refuses_paid_text_operations_without_the_openrouter_key() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, false);
    for operation in ["embeddings", "ner", "triples"] {
        let refusal = harness
            .refusal(
                &token,
                "processing_prepare",
                json!({ "requestId": operation, "collectionIds": [ALFA], "operations": [operation] }),
            )
            .await;
        assert_eq!(refusal["error"], "provider_not_configured", "{operation}");
    }
    let jobs = harness.payload(&token, "jobs_list", json!({})).await;
    assert!(records(&jobs).is_empty());
    harness
        .payload(&token, "processing_prepare", prepare_args("ocr-only"))
        .await;
}

#[tokio::test]
async fn processing_start_needs_the_spend_grant_and_asks_nothing_without_it() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, false);
    let job = harness.prepare("req-a", ALFA);
    let revision = harness.mark_ready(&job);
    let refusal = harness
        .refusal(
            &token,
            "processing_start",
            json!({ "jobId": job, "revision": revision }),
        )
        .await;
    assert_eq!(refusal["error"], ERROR_FORBIDDEN_SCOPE);
    assert!(harness.endpoint.approvals().pending().is_empty());
    assert_eq!(harness.desired_state(&job), "pause");
}

#[tokio::test]
async fn processing_start_asks_the_owner_once_and_runs_only_after_approval() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, true);
    let job = harness.prepare("req-a", ALFA);
    let revision = harness.mark_ready(&job);
    let arguments = json!({ "jobId": job, "revision": revision });

    let asked = harness
        .payload(&token, "processing_start", arguments.clone())
        .await;
    assert_eq!(asked["status"], "pending");
    assert_eq!(asked["jobId"], job);
    assert_eq!(asked["revision"], revision);
    assert_eq!(asked["action"], json!({ "kind": "start" }));
    assert_eq!(asked["clientName"], "Claude Code");
    assert_eq!(asked["files"], 2);
    assert_eq!(harness.desired_state(&job), "pause", "nothing runs yet");
    let approval_id = asked["id"].as_str().expect("an approval id").to_string();

    let again = harness.payload(&token, "processing_start", arguments).await;
    assert_eq!(again["id"], approval_id, "one request, one dialog");
    assert_eq!(harness.endpoint.approvals().pending().len(), 1);

    let status = harness
        .payload(
            &token,
            "approvals_status",
            json!({ "approvalId": approval_id }),
        )
        .await;
    assert_eq!(status["status"], "pending");

    harness.owner_decides(&approval_id, true);
    let status = harness
        .payload(
            &token,
            "approvals_status",
            json!({ "approvalId": approval_id }),
        )
        .await;
    assert_eq!(status["status"], "approved");
    assert_eq!(harness.desired_state(&job), "run");
}

#[tokio::test]
async fn processing_start_refuses_a_stale_revision_an_unready_job_and_an_unknown_one() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, true);
    let drafting = harness.prepare("req-a", ALFA);
    let ready = harness.prepare("req-b", ALFA);
    let revision = harness.mark_ready(&ready);
    let draft_revision = {
        let detail = harness
            .payload(&token, "jobs_get", json!({ "jobId": drafting }))
            .await;
        detail["job"]["revision"].as_i64().unwrap()
    };
    for (arguments, code) in [
        (
            json!({ "jobId": ready, "revision": revision + 5 }),
            "revision_conflict",
        ),
        (
            json!({ "jobId": drafting, "revision": draft_revision }),
            "invalid_state",
        ),
        (
            json!({ "jobId": "batch-missing", "revision": 0 }),
            ERROR_NOT_FOUND,
        ),
    ] {
        let refusal = harness
            .refusal(&token, "processing_start", arguments.clone())
            .await;
        assert_eq!(refusal["error"], code, "{arguments}");
    }
    assert!(harness.endpoint.approvals().pending().is_empty());
}

#[tokio::test]
async fn approvals_status_never_shows_another_assistants_request() {
    let harness = harness().await;
    let (first_id, first_token) = harness.pair("Claude Code");
    let (_second_id, second_token) = harness.pair("Codex");
    harness.grant(&first_id, true, true);
    let job = harness.prepare("req-a", ALFA);
    let revision = harness.mark_ready(&job);
    let asked = harness
        .payload(
            &first_token,
            "processing_start",
            json!({ "jobId": job, "revision": revision }),
        )
        .await;
    let refusal = harness
        .refusal(
            &second_token,
            "approvals_status",
            json!({ "approvalId": asked["id"] }),
        )
        .await;
    assert_eq!(refusal["error"], ERROR_NOT_FOUND);
}

#[tokio::test]
async fn pause_and_cancel_need_the_local_grant_and_apply_at_once() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    let job = harness.prepare("req-a", ALFA);
    let refusal = harness
        .refusal(
            &token,
            "processing_control",
            json!({ "jobId": job, "action": "cancel" }),
        )
        .await;
    assert_eq!(refusal["error"], ERROR_FORBIDDEN_SCOPE);
    assert_eq!(harness.desired_state(&job), "pause");

    harness.grant(&id, true, false);
    let cancelled = harness
        .payload(
            &token,
            "processing_control",
            json!({ "jobId": job, "action": "cancel" }),
        )
        .await;
    assert_eq!(cancelled["outcome"], "done");
    assert_eq!(cancelled["job"]["desiredState"], "cancel");
    assert_eq!(cancelled["approval"], Value::Null);
    assert_eq!(harness.desired_state(&job), "cancel");
    assert!(
        harness.endpoint.approvals().pending().is_empty(),
        "no dialog"
    );
}

#[tokio::test]
async fn resume_and_retry_need_the_spend_grant_and_wait_for_the_owner() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, false);
    let job = harness.prepare("req-a", ALFA);
    harness.mark_running(&job);
    let paused = harness
        .payload(
            &token,
            "processing_control",
            json!({ "jobId": job, "action": "pause" }),
        )
        .await;
    let revision = paused["job"]["revision"].as_i64().unwrap();
    let resume = json!({ "jobId": job, "action": "resume", "revision": revision });
    let refusal = harness
        .refusal(&token, "processing_control", resume.clone())
        .await;
    assert_eq!(refusal["error"], ERROR_FORBIDDEN_SCOPE, "resuming spends");

    harness.grant(&id, true, true);
    let asked = harness.payload(&token, "processing_control", resume).await;
    assert_eq!(asked["outcome"], "approval_pending");
    assert_eq!(asked["approval"]["action"], json!({ "kind": "resume" }));
    assert_eq!(harness.desired_state(&job), "pause", "nothing resumed yet");

    let retry = harness
        .payload(
            &token,
            "processing_control",
            json!({ "jobId": job, "action": "retry", "revision": revision, "failedOnly": true }),
        )
        .await;
    assert_eq!(retry["outcome"], "approval_pending");
    assert_eq!(
        retry["approval"]["action"],
        json!({ "kind": "retry", "task_id": null, "failed_only": true })
    );
    assert_eq!(harness.endpoint.approvals().pending().len(), 2);
}

#[tokio::test]
async fn resume_is_refused_on_a_job_that_never_started() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, true);
    let draft = harness.prepare("req-a", ALFA);
    let ready = harness.prepare("req-b", ALFA);
    let ready_revision = harness.mark_ready(&ready);
    let draft_revision = harness
        .payload(&token, "jobs_get", json!({ "jobId": draft }))
        .await["job"]["revision"]
        .as_i64()
        .unwrap();
    for (job, revision) in [(draft, draft_revision), (ready, ready_revision)] {
        let refusal = harness
            .refusal(
                &token,
                "processing_control",
                json!({ "jobId": job, "action": "resume", "revision": revision }),
            )
            .await;
        assert_eq!(refusal["error"], "invalid_state", "{job}");
        assert!(
            refusal["message"]
                .as_str()
                .unwrap()
                .contains("processing_start"),
            "the refusal names the right tool: {refusal}"
        );
    }
    assert!(harness.endpoint.approvals().pending().is_empty());
}

#[tokio::test]
async fn processing_prepare_refuses_an_unknown_operation_instead_of_dropping_it() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, false);
    let refusal = harness
        .refusal(
            &token,
            "processing_prepare",
            json!({ "requestId": "r1", "collectionIds": [ALFA], "operations": ["ocr", "shell"] }),
        )
        .await;
    assert_eq!(refusal["error"], ERROR_INVALID_ARGUMENT);
    assert!(refusal["message"].as_str().unwrap().contains("shell"));
    let jobs = harness.payload(&token, "jobs_list", json!({})).await;
    assert!(records(&jobs).is_empty());
}

#[tokio::test]
async fn processing_control_refuses_unknown_actions_and_unfenced_spending() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.grant(&id, true, true);
    let job = harness.prepare("req-a", ALFA);
    for arguments in [
        json!({ "jobId": job, "action": "explode" }),
        json!({ "jobId": job, "action": "resume" }),
        json!({ "jobId": job, "action": "retry", "revision": 1 }),
    ] {
        let refusal = harness
            .refusal(&token, "processing_control", arguments.clone())
            .await;
        assert_eq!(refusal["error"], ERROR_INVALID_ARGUMENT, "{arguments}");
    }
    assert!(harness.endpoint.approvals().pending().is_empty());
}

/// Page texts for the first Alfa item: the PDF page has OCR text with
/// characters outside ASCII, the cover image only a transcription (the
/// extraction row is empty), so both sources and the fallback are covered.
const PAGE_ONE: &str = "Página uno: café, ñandú y acción.\nSegunda línea del documento.";
const COVER: &str = "Lectura en voz alta de la tapa.";

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}

impl Harness {
    fn seed_page_texts(&self) {
        let conn = crate::db::open::open_archive_connection(&self.db_path()).unwrap();
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, confidence, created_at)
             VALUES ('ext_1', 'ast_alfa_1', ?1, 'ocr', 0.9, 5000)",
            [PAGE_ONE],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, confidence, created_at)
             VALUES ('ext_2', 'ast_alfa_2', '', 'native', NULL, 5001)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcriptions (id, asset_id, text_content, language, duration_ms, model, segments, confidence, created_at)
             VALUES ('trn_1', 'ast_alfa_2', ?1, 'es', 4200, 'test', ?2, NULL, 5002)",
            rusqlite::params![
                COVER,
                r#"[{"start_ms":0,"end_ms":4200,"text":"Lectura en voz alta de la tapa."}]"#
            ],
        )
        .unwrap();
    }
}

#[tokio::test]
async fn documents_get_text_returns_every_page_with_its_source_and_hash() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");
    harness.seed_page_texts();
    let item = test_archive::item_id(ALFA, 1);
    let text = harness
        .payload(&token, "documents_get_text", json!({ "itemId": item }))
        .await;

    assert_eq!(keys(&text), sorted(&["item", "pages", "nextCursor"]));
    assert_eq!(text["item"], json!({ "id": item, "title": "Documento 1" }));
    assert_eq!(
        text["nextCursor"],
        Value::Null,
        "everything fit in one call"
    );
    let pages = text["pages"].as_array().expect("pages").clone();
    assert_eq!(pages.len(), 2);

    let first = &pages[0];
    assert_eq!(
        keys(first),
        sorted(&[
            "assetId",
            "type",
            "pageNumber",
            "source",
            "method",
            "textHash",
            "totalChars",
            "startChar",
            "endChar",
            "text",
            "segments",
        ])
    );
    assert_eq!(first["assetId"], "ast_alfa_1");
    assert_eq!(first["source"], "extraction");
    assert_eq!(first["method"], "ocr");
    assert_eq!(first["text"], PAGE_ONE);
    assert_eq!(first["textHash"], sha256_hex(PAGE_ONE));
    assert_eq!(first["totalChars"], PAGE_ONE.chars().count());
    assert_eq!(first["startChar"], 0);
    assert_eq!(first["endChar"], PAGE_ONE.chars().count());
    assert_eq!(first["segments"], Value::Null);

    let cover = &pages[1];
    assert_eq!(cover["assetId"], "ast_alfa_2");
    assert_eq!(
        cover["source"], "transcription",
        "an empty extraction falls back"
    );
    assert_eq!(cover["text"], COVER);
    assert_eq!(cover["textHash"], sha256_hex(COVER));
    assert_eq!(
        cover["segments"],
        json!([{ "startMs": 0, "endMs": 4200, "text": COVER }])
    );

    let last = harness.audit_lines().last().cloned().expect("audited");
    assert_eq!(last["tool"], "documents_get_text");
    assert_eq!(last["objectIds"], json!([item]));
}

#[tokio::test]
async fn documents_get_text_pages_by_characters_and_never_splits_a_character() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");
    harness.seed_page_texts();
    let item = test_archive::item_id(ALFA, 1);
    let mut cursor: Option<String> = None;
    let mut collected: std::collections::BTreeMap<String, String> = Default::default();
    let mut calls = 0;
    loop {
        let mut arguments = json!({ "itemId": item, "maxChars": 7 });
        if let Some(cursor) = &cursor {
            arguments["cursor"] = json!(cursor);
        }
        let page = harness
            .payload(&token, "documents_get_text", arguments)
            .await;
        for window in page["pages"].as_array().unwrap() {
            let text = window["text"].as_str().unwrap();
            assert!(text.chars().count() <= 7, "a window respects maxChars");
            let asset = window["assetId"].as_str().unwrap().to_string();
            let entry = collected.entry(asset).or_default();
            assert_eq!(
                window["startChar"].as_u64().unwrap() as usize,
                entry.chars().count(),
                "windows are contiguous"
            );
            entry.push_str(text);
        }
        calls += 1;
        match page["nextCursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
        assert!(calls < 100, "the cursor always advances");
    }
    assert_eq!(collected["ast_alfa_1"], PAGE_ONE);
    assert_eq!(collected["ast_alfa_2"], COVER);
}

#[tokio::test]
async fn documents_get_text_reads_one_asset_and_refuses_bad_requests() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.seed_page_texts();
    let item = test_archive::item_id(ALFA, 1);
    let one = harness
        .payload(
            &token,
            "documents_get_text",
            json!({ "itemId": item, "assetId": "ast_alfa_2" }),
        )
        .await;
    let pages = one["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["assetId"], "ast_alfa_2");

    let empty = harness
        .payload(
            &token,
            "documents_get_text",
            json!({ "itemId": test_archive::item_id(ALFA, 2) }),
        )
        .await;
    assert_eq!(
        empty["pages"],
        json!([]),
        "an item without files has no pages"
    );

    for (arguments, code) in [
        (json!({ "itemId": "ite_missing" }), ERROR_NOT_FOUND),
        (
            json!({ "itemId": test_archive::item_id(BETA, 1), "assetId": "ast_alfa_1" }),
            ERROR_NOT_FOUND,
        ),
        (
            json!({ "itemId": item, "maxChars": 0 }),
            ERROR_INVALID_ARGUMENT,
        ),
        (
            json!({ "itemId": item, "maxChars": 20001 }),
            ERROR_INVALID_ARGUMENT,
        ),
        (
            json!({ "itemId": item, "cursor": "nope" }),
            ERROR_INVALID_ARGUMENT,
        ),
    ] {
        let refusal = harness
            .refusal(&token, "documents_get_text", arguments.clone())
            .await;
        assert_eq!(refusal["error"], code, "{arguments}");
    }

    harness.access.set_scopes(&id, vec![]).unwrap();
    let refusal = harness
        .refusal(&token, "documents_get_text", json!({ "itemId": item }))
        .await;
    assert_eq!(refusal["error"], ERROR_FORBIDDEN_SCOPE);
}

impl Harness {
    /// Two passages of the first Alfa item: one cut from the current page
    /// text, one left over from a transcription that has since been replaced.
    fn seed_passages(&self) {
        self.seed_page_texts();
        let conn = crate::db::open::open_archive_connection(&self.db_path()).unwrap();
        let current: String = PAGE_ONE.chars().take(33).collect();
        for (id, asset, kind, source, ordinal, text, start, end, hash) in [
            (
                "chk_1",
                "ast_alfa_1",
                "extraction",
                "ext_1",
                0_i64,
                current.clone(),
                0_i64,
                33_i64,
                sha256_hex(PAGE_ONE),
            ),
            (
                "chk_2",
                "ast_alfa_2",
                "transcription",
                "trn_old",
                0,
                "Tapa antigua: acción descartada.".to_string(),
                0,
                32,
                sha256_hex("texto que ya no existe"),
            ),
        ] {
            conn.execute(
                "INSERT INTO rag_chunks (id, asset_id, item_id, source_kind, source_id, chunk_ordinal, text_content, start_char, end_char, source_text_hash, chunking_contract, embedding, embedding_model, embedding_contract, dimensions)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'chunks-v1', x'00000000', 'test', 'test', 1)",
                rusqlite::params![
                    id,
                    asset,
                    test_archive::item_id(ALFA, 1),
                    kind,
                    source,
                    ordinal,
                    text,
                    start,
                    end,
                    hash
                ],
            )
            .unwrap();
        }
    }
}

#[tokio::test]
async fn search_passages_returns_citable_passages_and_flags_stale_ones() {
    let harness = harness().await;
    let (_id, token) = harness.pair("Claude Code");
    harness.seed_passages();
    let found = harness
        .payload(&token, "search_passages", json!({ "query": "acción" }))
        .await;
    assert_eq!(keys(&found), sorted(&["records", "matches", "coverage"]));
    assert_eq!(found["matches"], 2);
    assert_eq!(
        found["coverage"],
        json!({ "itemsInScope": ALFA_ITEMS + BETA_ITEMS, "itemsWithPassages": 1 })
    );
    let records = records(&found);
    let current = records
        .iter()
        .find(|record| record["assetId"] == "ast_alfa_1")
        .expect("the current passage");
    assert_eq!(
        keys(current),
        sorted(&[
            "itemId",
            "title",
            "collectionId",
            "assetId",
            "pageNumber",
            "source",
            "startChar",
            "endChar",
            "textHash",
            "current",
            "text",
            "rank",
        ])
    );
    assert_eq!(current["itemId"], test_archive::item_id(ALFA, 1));
    assert_eq!(current["title"], "Documento 1");
    assert_eq!(current["collectionId"], ALFA);
    assert_eq!(current["source"], "extraction");
    assert_eq!(current["startChar"], 0);
    assert_eq!(current["endChar"], 33);
    assert_eq!(current["textHash"], sha256_hex(PAGE_ONE));
    assert_eq!(
        current["current"], true,
        "the page text it indexes is the current one"
    );
    let text: String = PAGE_ONE.chars().take(33).collect();
    assert_eq!(current["text"], text);

    // The same window read through documents_get_text is the same text.
    let page = harness
        .payload(
            &token,
            "documents_get_text",
            json!({ "itemId": test_archive::item_id(ALFA, 1), "assetId": "ast_alfa_1", "maxChars": 33 }),
        )
        .await;
    assert_eq!(page["pages"][0]["text"], current["text"]);
    assert_eq!(page["pages"][0]["textHash"], current["textHash"]);

    let stale = records
        .iter()
        .find(|record| record["assetId"] == "ast_alfa_2")
        .expect("the stale passage");
    assert_eq!(stale["current"], false, "its page text has changed since");
}

#[tokio::test]
async fn search_passages_is_scoped_and_refuses_bad_requests() {
    let harness = harness().await;
    let (id, token) = harness.pair("Claude Code");
    harness.seed_passages();
    let in_beta = harness
        .payload(
            &token,
            "search_passages",
            json!({ "query": "acción", "collectionId": BETA }),
        )
        .await;
    assert_eq!(in_beta["matches"], 0);
    assert_eq!(
        in_beta["coverage"],
        json!({ "itemsInScope": BETA_ITEMS, "itemsWithPassages": 0 })
    );
    let in_item = harness
        .payload(
            &token,
            "search_passages",
            json!({ "query": "acción", "itemId": test_archive::item_id(ALFA, 1), "limit": 1 }),
        )
        .await;
    assert_eq!(records(&in_item).len(), 1, "limit applies");
    assert_eq!(in_item["matches"], 2, "matches counts before the limit");
    assert_eq!(
        in_item["coverage"],
        json!({ "itemsInScope": 1, "itemsWithPassages": 1 })
    );

    for arguments in [
        json!({ "query": "   " }),
        json!({ "query": "acción", "limit": 0 }),
        json!({ "query": "acción", "limit": 201 }),
    ] {
        let refusal = harness
            .refusal(&token, "search_passages", arguments.clone())
            .await;
        assert_eq!(refusal["error"], ERROR_INVALID_ARGUMENT, "{arguments}");
    }
    let last = harness
        .audit_lines()
        .into_iter()
        .rev()
        .find(|line| line["tool"] == "search_passages" && line["status"] == "ok")
        .expect("audited");
    assert!(
        last.get("queryChars").is_some(),
        "only the query length is audited"
    );

    harness.access.set_scopes(&id, vec![]).unwrap();
    let refusal = harness
        .refusal(&token, "search_passages", json!({ "query": "acción" }))
        .await;
    assert_eq!(refusal["error"], ERROR_FORBIDDEN_SCOPE);
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
