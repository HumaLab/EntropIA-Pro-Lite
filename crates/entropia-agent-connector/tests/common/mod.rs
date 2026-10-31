//! A fake EntropIA endpoint: the smallest HTTP/1.1 server the relay can POST to.
//!
//! Every request is recorded, and one handler decides the reply, so a test can
//! assert on the wire (path, headers, body) and on the relay's stdout at once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// One request the fake endpoint received.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Header names lower-cased.
    pub headers: HashMap<String, String>,
    pub body: String,
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
}

impl Reply {
    pub fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: Some("application/json"),
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    pub fn sse(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: Some("text/event-stream"),
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            content_type: Some("application/json"),
            body: String::new(),
            delay: Duration::ZERO,
        }
    }

    pub fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            body: String::new(),
            delay: Duration::ZERO,
        }
    }

    pub fn after(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

/// A running fake endpoint on an ephemeral loopback port.
pub struct FakeApp {
    pub port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    task: tokio::task::JoinHandle<()>,
}

impl FakeApp {
    pub async fn start<F>(handler: F) -> Self
    where
        F: Fn(&Request) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handler = Arc::new(handler);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let handler = handler.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let Some((mut stream, request)) = read_request(stream).await else {
                        return;
                    };
                    recorded.lock().unwrap().push(request.clone());
                    let reply = handler(&request);
                    if !reply.delay.is_zero() {
                        tokio::time::sleep(reply.delay).await;
                    }
                    let _ = write_reply(&mut stream, &reply).await;
                });
            }
        });
        Self {
            port,
            requests,
            task,
        }
    }

    pub fn requests(&self) -> Vec<Request> {
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

    Some((
        stream,
        Request {
            method,
            path,
            headers,
            body: String::from_utf8_lossy(&body).to_string(),
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
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(reply.body.as_bytes()).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}
