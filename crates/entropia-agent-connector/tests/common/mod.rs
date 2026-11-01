//! A fake EntropIA endpoint: the smallest HTTP/1.1 server the relay can POST to.
//!
//! Every request is recorded, and one handler decides the reply, so a test can
//! assert on the wire (path, headers, body) and on the relay's stdout at once.
//!
//! It also answers the D12 instance challenge the way the real app does, with a
//! proof derived from the instance key it advertises — or with the wrong proof,
//! or none, when a test needs to see the connector refuse.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use entropia_agent_connector::proof::{self, CHALLENGE_HEADER, PROOF_HEADER};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The instance key a fake app advertises unless a test says otherwise.
pub const INSTANCE_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// How a fake app answers the instance challenge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proof {
    /// The real app's behavior: 401 plus `X-Entropia-Proof` for the challenge.
    Correct,
    /// 401 plus a proof computed from another key.
    Wrong,
    /// 401 without any proof header.
    Missing,
}

/// One request the fake endpoint received.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Header names lower-cased.
    pub headers: HashMap<String, String>,
    pub body: String,
    /// Whether this was the D12 instance challenge rather than an MCP message.
    pub challenge: bool,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// The JSON-RPC method of the request body.
    pub fn rpc_method(&self) -> String {
        serde_json::from_str::<serde_json::Value>(&self.body)
            .ok()
            .and_then(|body| body["method"].as_str().map(str::to_string))
            .unwrap_or_default()
    }
}

/// One scripted reply.
#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub content_type: Option<&'static str>,
    pub body: String,
    pub delay: Duration,
    /// Extra response headers, in order.
    pub headers: Vec<(&'static str, String)>,
}

impl Reply {
    pub fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: Some("application/json"),
            body: body.into(),
            delay: Duration::ZERO,
            headers: Vec::new(),
        }
    }

    pub fn sse(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: Some("text/event-stream"),
            body: body.into(),
            delay: Duration::ZERO,
            headers: Vec::new(),
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            content_type: Some("application/json"),
            body: String::new(),
            delay: Duration::ZERO,
            headers: Vec::new(),
        }
    }

    pub fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            body: String::new(),
            delay: Duration::ZERO,
            headers: Vec::new(),
        }
    }

    pub fn after(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

/// What the app answers to a challenge: the guard's `unauthorized` body and, if
/// the proof mode says so, the HMAC of the challenge under the instance key.
fn challenge_reply(mode: Proof, instance_key: &str, challenge: &str) -> Reply {
    let body = serde_json::json!({
        "error": "unauthorized",
        "message": "Falta la credencial del asistente o ya no sirve: volvé a emparejarlo en EntropIA.",
    })
    .to_string();
    let mut reply = Reply {
        status: 401,
        content_type: Some("application/json"),
        body,
        delay: Duration::ZERO,
        headers: Vec::new(),
    };
    match mode {
        Proof::Correct => {
            reply
                .headers
                .push((PROOF_HEADER, proof::proof_for(instance_key, challenge)));
        }
        Proof::Wrong => {
            let other = "ff".repeat(32);
            reply
                .headers
                .push((PROOF_HEADER, proof::proof_for(&other, challenge)));
        }
        Proof::Missing => {}
    }
    reply
}

/// A running fake endpoint on an ephemeral loopback port.
pub struct FakeApp {
    pub port: u16,
    instance_key: String,
    requests: Arc<Mutex<Vec<Request>>>,
    task: tokio::task::JoinHandle<()>,
}

impl FakeApp {
    pub async fn start<F>(handler: F) -> Self
    where
        F: Fn(&Request) -> Reply + Send + Sync + 'static,
    {
        Self::start_with(Proof::Correct, handler).await
    }

    pub async fn start_with<F>(mode: Proof, handler: F) -> Self
    where
        F: Fn(&Request) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let instance_key = INSTANCE_KEY.to_string();
        let handler = Arc::new(handler);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let key = instance_key.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let handler = handler.clone();
                let recorded = recorded.clone();
                let key = key.clone();
                tokio::spawn(async move {
                    let Some((mut stream, request)) = read_request(stream).await else {
                        return;
                    };
                    let reply = if request.challenge {
                        challenge_reply(
                            mode,
                            &key,
                            request.header(CHALLENGE_HEADER).unwrap_or_default(),
                        )
                    } else {
                        handler(&request)
                    };
                    recorded.lock().unwrap().push(request);
                    if !reply.delay.is_zero() {
                        tokio::time::sleep(reply.delay).await;
                    }
                    let _ = write_reply(&mut stream, &reply).await;
                });
            }
        });
        Self {
            port,
            instance_key,
            requests,
            task,
        }
    }

    /// The key this fake app advertises, and signs challenges with.
    pub fn instance_key(&self) -> &str {
        &self.instance_key
    }

    /// The MCP messages the app received, in order (the challenge is not one).
    pub fn requests(&self) -> Vec<Request> {
        self.all_requests()
            .into_iter()
            .filter(|request| !request.challenge)
            .collect()
    }

    /// The D12 challenges the app received.
    pub fn challenge_requests(&self) -> Vec<Request> {
        self.all_requests()
            .into_iter()
            .filter(|request| request.challenge)
            .collect()
    }

    /// Everything the app received, challenges included.
    pub fn all_requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeApp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A port nothing listens on (bound once, then released).
pub fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

async fn read_request(mut stream: TcpStream) -> Option<(TcpStream, Request)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = find(&buffer, b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut headers = HashMap::new();
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "content-length" {
            length = value.parse().unwrap_or(0);
        }
        headers.insert(name, value);
    }

    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    let challenge = headers.contains_key(CHALLENGE_HEADER);
    Some((
        stream,
        Request {
            method,
            path,
            headers,
            body: String::from_utf8_lossy(&body).to_string(),
            challenge,
        },
    ))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

async fn write_reply(stream: &mut TcpStream, reply: &Reply) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\nContent-Length: {}\r\n",
        reply.status,
        reason(reply.status),
        reply.body.len()
    );
    if let Some(content_type) = reply.content_type {
        head.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(reply.body.as_bytes()).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}
