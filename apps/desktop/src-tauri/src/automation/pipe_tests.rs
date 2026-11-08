use super::*;

fn advertised_pipe(h: &Harness) -> String {
    let raw: Value =
        serde_json::from_slice(&std::fs::read(h.dir.path().join(DISCOVERY_FILE)).unwrap()).unwrap();
    raw["pipe"]
        .as_str()
        .expect("Windows advertises a listening pipe")
        .to_owned()
}

async fn request(
    h: &Harness,
    path: &str,
    headers: &[(&str, String)],
    body: Value,
) -> (StatusCode, HeaderMap, Bytes) {
    let pipe = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(advertised_pipe(h))
        .unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(pipe))
        .await
        .unwrap();
    let driver = tokio::spawn(async move { connection.await.unwrap() });
    let mut req = Request::post(path)
        .header("host", format!("127.0.0.1:{}", h.port))
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json");
    for (name, value) in headers {
        req.headers_mut().unwrap().insert(
            http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    let response = sender
        .send_request(req.body(Full::new(Bytes::from(body.to_string()))).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    drop(sender);
    driver.await.unwrap();
    (parts.status, parts.headers, bytes)
}

#[tokio::test]
async fn pipe_handshake_and_read_tool() {
    let h = harness().await;
    let (id, token) = h.pair("Pipe client");
    let auth = [("authorization", format!("Bearer {token}"))];
    let (status, _, body) = request(&h, MCP_PATH, &auth, initialize_body()).await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["result"]["serverInfo"]["name"], "entropia");
    let (status, _, body) = request(
        &h,
        MCP_PATH,
        &auth,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body.is_empty());
    let (status, _, body) = request(
        &h,
        MCP_PATH,
        &auth,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "entropia_status"));
    let (status, _, body) = request(&h, MCP_PATH, &auth, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"entropia_status","arguments":{}}})).await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["result"]["isError"], false);
    assert_eq!(body["result"]["structuredContent"]["client"]["id"], id);
    h.access.revoke_client(&id).unwrap();
    assert_eq!(
        request(&h, MCP_PATH, &auth, initialize_body()).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn pipe_preserves_proof_and_guards() {
    let h = harness().await;
    let challenge = "ab".repeat(32);
    let (status, headers, _) = request(
        &h,
        MCP_PATH,
        &[(proof::CHALLENGE_HEADER, challenge.clone())],
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        headers[proof::PROOF_HEADER],
        proof::proof_for(&h.discovery().instance_key, &challenge)
    );
    for (path, headers, expected, error) in [
        (
            MCP_PATH,
            vec![("authorization", "Bearer bad".to_string())],
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        ("/wrong", vec![], StatusCode::NOT_FOUND, "not_found"),
        (
            MCP_PATH,
            vec![("origin", "https://evil.example".to_string())],
            StatusCode::FORBIDDEN,
            "forbidden_origin",
        ),
        (
            MCP_PATH,
            vec![("host", "evil.example".to_string())],
            StatusCode::FORBIDDEN,
            "forbidden_host",
        ),
    ] {
        let (status, _, body) = request(&h, path, &headers, initialize_body()).await;
        assert_eq!(status, expected);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"],
            error
        );
    }
}

#[tokio::test]
async fn pipe_discovery_lifecycle() {
    let h = harness().await;
    let first = advertised_pipe(&h);
    let suffix = first.strip_prefix(r"\\.\pipe\entropia-agent-").unwrap();
    assert_eq!(suffix.len(), 64);
    assert!(suffix
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    h.endpoint.stop();
    assert!(!h.dir.path().join(DISCOVERY_FILE).exists());
    tokio::task::yield_now().await;
    assert!(tokio::net::windows::named_pipe::ClientOptions::new()
        .open(&first)
        .is_err());
    h.endpoint.sync(&h.access).await.unwrap();
    assert_ne!(advertised_pipe(&h), first);
}
