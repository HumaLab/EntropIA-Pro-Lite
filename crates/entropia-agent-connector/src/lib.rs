//! `entropia-agent-connector`: the relay between an assistant's stdio and
//! EntropIA's local MCP endpoint.
//!
//! The binary ([`main`](../main/index.html) is thin) owns the command line and
//! the OS keyring; the crate keeps the testable parts here: [`cli`] resolves the
//! client id and the data directory, [`discovery`] reads the file the app
//! advertises its endpoint with, [`keyring_source`] reads the client token under
//! the same `(service, account)` the app wrote it to, [`proof`] verifies that the
//! process holding the advertised port is the app that wrote the file (D12), and
//! [`relay`] is the stdio↔loopback relay itself, generic over the streams and
//! over the credential/endpoint source so a test never touches the real keyring.

pub mod cli;
pub mod discovery;
pub mod error;
pub mod keyring_source;
pub mod proof;
pub mod relay;
pub mod session_bus;

pub use error::ConnectorError;
