#![cfg(windows)]
// Reuse the relay's wire-level harness and run its HTTP preservation cases too.
include!("relay.rs");
use tokio::net::windows::named_pipe::ServerOptions;

struct FakePipe {
    name: String,
    requests: Arc<Mutex<Vec<common::Request>>>,
    task: tokio::task::JoinHandle<()>,
}

impl FakePipe {
    fn start(mode: Proof, response: Option<Reply>) -> Self {
        let name = format!(r"\\.\pipe\entropia-agent-{}", proof::challenge().unwrap());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let path = name.clone();
        let task = tokio::spawn(async move {
            loop {
                server.connect().await.unwrap();
                let connected = server;
                server = ServerOptions::new().create(&path).unwrap();
                let recorded = recorded.clone();
                let response = response.clone();
                tokio::spawn(async move {
                    let Some((mut stream, request)) = common::read_request(connected).await else {
                        return;
                    };
                    let reply = if request.challenge {
                        common::challenge_reply(
                            mode,
                            common::INSTANCE_KEY,
                            request.header(proof::CHALLENGE_HEADER).unwrap(),
                        )
                    } else if let Some(reply) = response {
                        reply
                    } else {
                        let body: Value = serde_json::from_str(&request.body).unwrap();
                        if body.get("id").is_none() {
                            Reply::empty(202)
                        } else {
                            Reply::json(json!({"jsonrpc":"2.0", "id":body["id"], "result":{"tools":[], "protocolVersion":"2025-06-18"}}).to_string())
                        }
                    };
                    recorded.lock().unwrap().push(request);
                    common::write_reply(&mut stream, &reply).await.unwrap();
                });
            }
        });
        Self {
            name,
            requests,
            task,
        }
    }
}
impl Drop for FakePipe {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn advertise_pipe(dir: &Path, port: u16, pipe: &str) {
    write_discovery_value(
        dir,
        json!({"protocol":"entropia-agent/1", "port":port, "instanceKey":common::INSTANCE_KEY, "pipe":pipe}),
    );
}

#[tokio::test]
async fn pipe_bypasses_shield_chunk_corruption_and_preserves_lines_and_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let tcp = FakeApp::start(|_| {
        let mut reply = Reply::json(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        reply.headers.push(("transfer-encoding", "chunked".into()));
        reply
    })
    .await;
    // Demonstrate the raw unchunked body really reproduces the failure.
    write_discovery(dir.path(), "entropia-agent/1", tcp.port);
    let broken = relay_once(discovery_file(dir.path()), Some(TOKEN), &[initialize(1)]).await;
    assert_eq!(error_code(&parse(&broken.lines[0])), "app_absent");
    assert!(broken.warnings[0].contains("se cortó la respuesta"));
    let before = tcp.all_requests().len();
    let pipe = FakePipe::start(Proof::Correct, None);
    advertise_pipe(dir.path(), tcp.port, &pipe.name);
    let outcome = relay_once(
        discovery_file(dir.path()),
        Some(TOKEN),
        &[initialize(1), tools_list(2), initialized_notification()],
    )
    .await;
    assert_eq!(outcome.lines.len(), 2);
    let mut ids = outcome
        .lines
        .iter()
        .map(|line| {
            let value = parse(line);
            assert!(value.get("error").is_none(), "{value}");
            assert_eq!(value["result"]["tools"], json!([]));
            value["id"].as_u64().unwrap()
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(ids, [1, 2]);
    assert!(outcome.warnings.is_empty());
    assert_eq!(
        tcp.all_requests().len(),
        before,
        "no TCP request after pipe advertised"
    );
    let requests = pipe.requests.lock().unwrap();
    assert_eq!(requests.len(), 6);
    for request in requests.iter() {
        assert_eq!(request.path, "/mcp");
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.header("host"),
            Some(format!("127.0.0.1:{}", tcp.port).as_str())
        );
        if request.challenge {
            assert_eq!(request.header("authorization"), None);
        } else {
            assert_eq!(
                request.header("authorization"),
                Some(format!("Bearer {TOKEN}").as_str())
            );
            assert_eq!(
                request.header("mcp-method"),
                Some(request.rpc_method().as_str())
            );
            assert_eq!(
                request.header("accept"),
                Some("application/json, text/event-stream")
            );
        }
    }
}

#[tokio::test]
async fn pipe_preserves_sse_and_explicit_protocol_and_routing_headers() {
    let dir = tempfile::tempdir().unwrap();
    let tcp = FakeApp::start(|_| initialize_reply(1)).await;
    let pipe = FakePipe::start(
        Proof::Correct,
        Some(Reply::sse(
            "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n",
        )),
    );
    advertise_pipe(dir.path(), tcp.port, &pipe.name);
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"entropia_status", "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}).to_string();
    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[request]).await;
    assert_eq!(outcome.lines, [r#"{"id":1,"jsonrpc":"2.0","result":{}}"#]);
    assert!(outcome.warnings.is_empty());
    assert!(tcp.all_requests().is_empty());
    let requests = pipe.requests.lock().unwrap();
    let post = requests.iter().find(|request| !request.challenge).unwrap();
    assert_eq!(post.header("mcp-protocol-version"), Some("2026-07-28"));
    assert_eq!(post.header("mcp-name"), Some("entropia_status"));
    assert_eq!(post.header("content-type"), Some("application/json"));
}

#[tokio::test]
async fn busy_pipe_wait_is_bounded_and_does_not_fall_back() {
    use tokio::net::windows::named_pipe::ClientOptions;
    let dir = tempfile::tempdir().unwrap();
    let tcp = FakeApp::start(|_| initialize_reply(1)).await;
    let name = format!(r"\\.\pipe\entropia-agent-{}", proof::challenge().unwrap());
    let server = ServerOptions::new().max_instances(1).create(&name).unwrap();
    let _occupant = ClientOptions::new().open(&name).unwrap();
    server.connect().await.unwrap();
    advertise_pipe(dir.path(), tcp.port, &name);
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        relay_once(discovery_file(dir.path()), Some(TOKEN), &[initialize(1)]),
    )
    .await
    .expect("busy wait must end");
    assert!(started.elapsed() >= std::time::Duration::from_millis(200));
    assert_eq!(outcome.lines.len(), 1);
    assert_eq!(error_code(&parse(&outcome.lines[0])), "app_absent");
    assert_eq!(outcome.warnings.len(), 1);
    assert!(tcp.all_requests().is_empty());
}

#[tokio::test]
async fn missing_pipe_falls_back_to_http() {
    let dir = tempfile::tempdir().unwrap();
    let tcp = FakeApp::start(|_| initialize_reply(1)).await;
    advertise_pipe(
        dir.path(),
        tcp.port,
        &format!(r"\\.\pipe\entropia-agent-{}", proof::challenge().unwrap()),
    );
    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[initialize(1)]).await;
    assert_eq!(
        parse(&outcome.lines[0])["result"]["protocolVersion"],
        "2025-06-18"
    );
    assert!(outcome.warnings.is_empty());
    assert_eq!(tcp.requests().len(), 1);
}

#[tokio::test]
async fn pipe_proof_failure_never_sends_token_or_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let tcp = FakeApp::start(|_| initialize_reply(1)).await;
    let pipe = FakePipe::start(Proof::Wrong, None);
    advertise_pipe(dir.path(), tcp.port, &pipe.name);
    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[initialize(1)]).await;
    assert_eq!(outcome.lines.len(), 1);
    assert_eq!(error_code(&parse(&outcome.lines[0])), "app_absent");
    assert_eq!(outcome.warnings.len(), 1);
    assert!(outcome.warnings[0].contains("verificar"));
    assert!(!outcome.warnings[0].contains(TOKEN));
    assert!(tcp.all_requests().is_empty());
    let requests = pipe.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].header("authorization"), None);
}

#[tokio::test]
async fn pipe_errors_do_not_fall_back() {
    for (reply, code) in [
        (Reply::status(401), "unauthorized"),
        (Reply::status(403), "forbidden"),
        (
            {
                let mut reply = Reply::json("private-body");
                reply.headers.push(("transfer-encoding", "chunked".into()));
                reply
            },
            "app_absent",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let tcp = FakeApp::start(|_| initialize_reply(1)).await;
        let pipe = FakePipe::start(Proof::Correct, Some(reply));
        advertise_pipe(dir.path(), tcp.port, &pipe.name);
        let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[initialize(1)]).await;
        assert_eq!(outcome.lines.len(), 1);
        assert_eq!(error_code(&parse(&outcome.lines[0])), code);
        assert_eq!(outcome.warnings.len(), 1);
        assert!(!outcome.warnings[0].contains(TOKEN));
        assert!(!outcome.warnings[0].contains("private-body"));
        assert!(tcp.all_requests().is_empty());
    }
}
