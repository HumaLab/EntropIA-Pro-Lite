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

fn main() -> ExitCode {
    // Before the runtime starts any thread: setting a variable is only sound
    // while nothing else can read the environment.
    point_keyring_at_session_bus();
    match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime.block_on(start()),
        Err(error) => {
            eprintln!("entropia-agent-connector: no pude iniciar el runtime: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Give the keyring the user's session bus when the agent launched this
/// process without one (see `session_bus`). Linux only: the other platforms'
/// stores do not use D-Bus.
#[cfg(target_os = "linux")]
fn point_keyring_at_session_bus() {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let Ok(me) = std::fs::metadata("/proc/self") else {
        return;
    };
    let existing = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let address = entropia_agent_connector::session_bus::default_address(
        existing.as_deref(),
        runtime_dir.as_deref(),
        me.uid(),
        |path| {
            std::fs::symlink_metadata(path)
                .ok()
                .filter(|meta| meta.file_type().is_socket())
                .map(|meta| meta.uid())
        },
    );
    if let Some(address) = address {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address);
    }
}

#[cfg(not(target_os = "linux"))]
fn point_keyring_at_session_bus() {}

async fn start() -> ExitCode {
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
        Invocation::Run(options) => run(options, false).await,
        Invocation::Check(options) => run(options, true).await,
    }
}

async fn run(options: cli::Options, check: bool) -> ExitCode {
    let override_dir = std::env::var_os(cli::DATA_DIR_VAR).map(PathBuf::from);
    let platform_data = match &override_dir {
        // Ignored: the override replaces the whole shared directory.
        Some(_) => PathBuf::new(),
        None => match cli::platform_data_dir(dirs::data_dir(), std::env::var_os("APPDATA")) {
            Some(dir) => dir,
            None => {
                if check {
                    println!("data_dir_unavailable: Indicá ENTROPIA_DATA_DIR=<ruta> para localizar los datos de EntropIA.");
                    return ExitCode::FAILURE;
                }
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
    if check {
        let (exit, line) = relay::check(config, source).await;
        println!("{line}");
        return ExitCode::from(exit);
    }
    match relay::run(tokio::io::stdin(), tokio::io::stdout(), config, source).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("entropia-agent-connector: {error}");
            ExitCode::FAILURE
        }
    }
}
