//! Windows HTTP/1 transport. Only a missing pipe permits HTTP fallback, and
//! only before the challenge: a selected pipe never changes transport mid-flow.
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper_util::rt::TokioIo;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

use crate::error::ConnectorError;

pub struct Pipe {
    name: String,
    first: Option<NamedPipeClient>,
}

async fn open(name: &str) -> std::io::Result<NamedPipeClient> {
    // ERROR_PIPE_BUSY alone is transient. Bound the total wait to 200 ms.
    for attempt in 0..=10 {
        match ClientOptions::new().open(name) {
            Err(error) if error.raw_os_error() == Some(231) && attempt < 10 => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            result => return result,
        }
    }
    unreachable!()
}

impl Pipe {
    pub async fn select(name: &str) -> std::io::Result<Option<Self>> {
        match open(name).await {
            Ok(stream) => Ok(Some(Self {
                name: name.to_string(),
                first: Some(stream),
            })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn send(
        &mut self,
        request: reqwest::Request,
    ) -> Result<hyper::Response<Incoming>, ConnectorError> {
        let url = request.url().to_string();
        let connection_error = |error: String| {
            ConnectorError::app_absent(format!("no pude conectar con {url}: {error}"))
        };
        let stream = match self.first.take() {
            Some(stream) => stream,
            None => open(&self.name)
                .await
                .map_err(|error| connection_error(error.to_string()))?,
        };
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|error| connection_error(error.to_string()))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let body = request
            .body()
            .and_then(reqwest::Body::as_bytes)
            .unwrap_or_default();
        let mut outgoing = hyper::Request::builder()
            .method(request.method().clone())
            .uri(request.url().path())
            .body(Full::new(Bytes::copy_from_slice(body)))
            .map_err(|error| connection_error(error.to_string()))?;
        *outgoing.headers_mut() = request.headers().clone();
        outgoing.headers_mut().insert(
            hyper::header::HOST,
            format!("127.0.0.1:{}", request.url().port().unwrap_or(80))
                .parse()
                .map_err(|error: hyper::header::InvalidHeaderValue| {
                    connection_error(error.to_string())
                })?,
        );
        sender
            .send_request(outgoing)
            .await
            .map_err(|error| connection_error(error.to_string()))
    }
}

pub async fn chunk(body: &mut Incoming) -> Result<Option<Bytes>, hyper::Error> {
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data() {
            return Ok(Some(data));
        }
    }
    Ok(None)
}
