//! `entropia-agent-connector`: the stdio↔loopback bridge an assistant launches.
//!
//! It identifies itself with a paired client id, reads that client's token from
//! the OS keyring (never from agent configuration, S4), finds the running app
//! through its discovery file (D7) and relays JSON-RPC between the agent's
//! stdio and the app's local MCP endpoint. Stdout carries protocol messages only
//! (S5); everything the person needs to act on goes to stderr (S7).

use std::path::PathBuf;
use std::process::ExitCode;

use entropia_agent_connector::cli::{self, Invocation};
use entropia_agent_connector::discovery::DISCOVERY_FILE;
use entropia_agent_connector::keyring_source::KeyringSource;
use entropia_agent_connector::relay::{self, RelayConfig};

#[tokio::main]
async fn main() -> ExitCode {
    let invocation = match cli::parse_args(std::env::args()) {
        Ok(invocation) => invocation,
        Err(error) => {
            eprintln!("entropia-agent-connector: {}", error.0);
            return ExitCode::from(2);
        }
    };
    match invocation {
        Invocation::Version => {
            println!("entropia-agent-connector {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Invocation::Help => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Invocation::Run(options) => run(options).await,
    }
}

async fn run(options: cli::Options) -> ExitCode {
    let override_dir = std::env::var_os(cli::DATA_DIR_VAR).map(PathBuf::from);
    let platform_data = match &override_dir {
        // Ignored: the override replaces the whole shared directory.
        Some(_) => PathBuf::new(),
        None => match dirs::data_dir() {
            Some(dir) => dir,
            None => {
                eprintln!(
                    "entropia-agent-connector: no pude resolver el directorio de datos del \
                     sistema; indicá {}=<ruta>",
                    cli::DATA_DIR_VAR
                );
                return ExitCode::FAILURE;
            }
        },
    };
    let data_dir = cli::resolve_data_dir(
        override_dir.as_deref(),
        options.profile.as_deref(),
        &platform_data,
    );

    let source: std::sync::Arc<dyn relay::ConnectorSource> =
        std::sync::Arc::new(KeyringSource::new(
            data_dir.join(DISCOVERY_FILE),
            &data_dir,
            options.profile.as_deref(),
        ));
    let config = RelayConfig::new(options.client_id);
    match relay::run(tokio::io::stdin(), tokio::io::stdout(), config, source).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("entropia-agent-connector: {error}");
            ExitCode::FAILURE
        }
    }
}
