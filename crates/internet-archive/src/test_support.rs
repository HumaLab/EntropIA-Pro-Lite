//! Tiny hand-rolled HTTP server for tests: canned answers by exact path,
//! no external crates, no real network. Mirrors the `test_server` pattern
//! in `navegador/zotero_copy/port.rs`.
//!
//! The server records every request target (`path?query`) so tests can assert
//! on the exact query the client built; routing still matches the path part
//! only.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Canned answer served for one exact path.
pub struct Route {
    path: String,
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Local test server. Routing matches the path part of the request target;
/// the full target (`path?query`) of every request is recorded.
pub struct TestServer {
    base: String,
    targets: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    /// Build a 302 route to `location`.
    pub fn redirect(path: &str, location: &str) -> Route {
        Route {
            path: path.to_string(),
            status: 302,
            headers: vec![
                ("Location".to_string(), location.to_string()),
                ("Content-Length".to_string(), "0".to_string()),
            ],
            body: Vec::new(),
        }
    }

    /// Build a JSON route with the given status.
    pub fn json(path: &str, status: u16, body: &str) -> Route {
        let bytes = body.as_bytes().to_vec();
        Route {
            path: path.to_string(),
            status,
            headers: vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Content-Length".to_string(), bytes.len().to_string()),
            ],
            body: bytes,
        }
    }

    /// Build a route with explicit headers and body.
    pub fn raw(path: &str, status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Route {
        Route {
            path: path.to_string(),
            status,
            headers,
            body,
        }
    }

    /// Start the server on `127.0.0.1` with an ephemeral port.
    pub fn start(routes: Vec<Route>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let targets: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&targets);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let head_end = loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break None;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(at + 4);
                    }
                    if raw.len() > 64 * 1024 {
                        break None;
                    }
                };
                let Some(head_end) = head_end else { continue };
                let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
                let mut lines = head.lines();
                let request_line = lines.next().unwrap_or("").to_string();
                let mut parts = request_line.split_whitespace();
                let target = parts.nth(1).unwrap_or("/").to_string();
                recorded.lock().unwrap().push(target.clone());
                let path_only = target
                    .split(['?', '#'])
                    .next()
                    .unwrap_or(&target)
                    .to_string();
                let route = routes.iter().find(|r| r.path == path_only);
                let (status, headers, body) = match route {
                    Some(r) => (r.status, r.headers.clone(), r.body.clone()),
                    None => (
                        404,
                        vec![("Content-Length".to_string(), "0".to_string())],
                        Vec::new(),
                    ),
                };
                let mut reply = format!("HTTP/1.1 {status} X\r\nConnection: close\r\n");
                for (k, v) in &headers {
                    reply.push_str(&format!("{k}: {v}\r\n"));
                }
                reply.push_str("\r\n");
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        Self { base, targets }
    }

    /// Base URL of the server, e.g. `http://127.0.0.1:1234`.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Full request targets (`path?query`) in arrival order.
    pub fn targets(&self) -> Vec<String> {
        self.targets.lock().unwrap().clone()
    }

    /// How many requests arrived for `path` (query string ignored).
    pub fn count(&self, path: &str) -> usize {
        self.targets
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.split(['?', '#']).next().unwrap_or(t) == path)
            .count()
    }
}
