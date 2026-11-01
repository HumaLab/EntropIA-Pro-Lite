//! The command line: a client id, an optional dev profile, and the data
//! directory those two resolve to.

use std::path::{Path, PathBuf};

/// The directory name every EntropIA variant shares (`path_utils.rs`).
pub const SHARED_DIR_NAME: &str = "com.entropia.shared";

/// Directory, under the shared one, that holds the dev profiles.
pub const PROFILES_DIR: &str = "dev-profiles";

/// Environment variable that replaces the data directory (tests, stages 6).
pub const DATA_DIR_VAR: &str = "ENTROPIA_DATA_DIR";

const PROFILE_MAX_CHARS: usize = 32;
const CLIENT_MAX_CHARS: usize = 64;

/// What the process was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    Version,
    Help,
    Run(Options),
}

/// A run: which paired client, and which dev profile's data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub client_id: String,
    pub profile: Option<String>,
}

/// A refusal that is the caller's, not the environment's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for CliError {}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

pub const USAGE: &str = "\
Uso:
  entropia-agent-connector --client <id> [--profile <nombre>]
  entropia-agent-connector --version

Habla JSON-RPC por stdin/stdout y lo retransmite al endpoint local de EntropIA.
El id del cliente se empareja en EntropIA (Ajustes → Asistentes); el token se
lee del llavero del sistema, nunca de la configuración del agente.

  --client <id>        id del cliente emparejado en EntropIA
  --profile <nombre>   perfil de desarrollo: 1 a 32 caracteres de A-Z, a-z, 0-9,
                       '-' y '_'
  --version, -V        versión del conector
  --help, -h           esta ayuda

La variable ENTROPIA_DATA_DIR reemplaza el directorio de datos de la aplicación.
Con --profile, los datos viven en <datos>/dev-profiles/<nombre>.";

/// Parse the process arguments, program name included.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Invocation, CliError> {
    let mut arguments = args.into_iter();
    let _program = arguments.next();
    let arguments: Vec<String> = arguments.collect();

    let mut client_id: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].clone();
        let (name, inline) = match argument.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(value.to_string())),
            None => (argument.clone(), None),
        };
        match name.as_str() {
            "--version" | "-V" => return Ok(Invocation::Version),
            "--help" | "-h" => return Ok(Invocation::Help),
            "--client" | "--profile" => {
                let value = match inline {
                    Some(value) => value,
                    None => {
                        index += 1;
                        arguments.get(index).cloned().ok_or_else(|| {
                            CliError(format!("falta el valor de {name}\n\n{USAGE}"))
                        })?
                    }
                };
                if name == "--client" {
                    if client_id.is_some() {
                        return Err(CliError(format!(
                            "--client se indicó más de una vez\n\n{USAGE}"
                        )));
                    }
                    client_id = Some(validate_client_id(&value)?);
                } else {
                    if profile.is_some() {
                        return Err(CliError(format!(
                            "--profile se indicó más de una vez\n\n{USAGE}"
                        )));
                    }
                    profile = Some(parse_profile_name(&value)?);
                }
            }
            other => {
                return Err(CliError(format!("opción desconocida: {other}\n\n{USAGE}")));
            }
        }
        index += 1;
    }

    let client_id = client_id.ok_or_else(|| {
        CliError(format!(
            "falta --client <id>: el asistente se identifica con el id del cliente emparejado en \
             EntropIA\n\n{USAGE}"
        ))
    })?;
    Ok(Invocation::Run(Options { client_id, profile }))
}

/// A profile name: the rule `dev_profile.rs` applies, so both sides name the
/// same directory.
pub fn parse_profile_name(raw: &str) -> Result<String, String> {
    let plain = !raw.is_empty()
        && raw.chars().count() <= PROFILE_MAX_CHARS
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if plain {
        Ok(raw.to_string())
    } else {
        Err(format!(
            "el perfil debe tener de 1 a {PROFILE_MAX_CHARS} caracteres de A-Z, a-z, 0-9, '-' y '_' \
             (recibido {raw:?})"
        ))
    }
}

/// A client id: the `cl_…` name EntropIA minted, never a token or a path.
fn validate_client_id(raw: &str) -> Result<String, CliError> {
    let plain = !raw.is_empty()
        && raw.chars().count() <= CLIENT_MAX_CHARS
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if plain {
        Ok(raw.to_string())
    } else {
        Err(CliError(format!(
            "el id del cliente debe tener de 1 a {CLIENT_MAX_CHARS} caracteres de A-Z, a-z, 0-9, \
             '-' y '_' (recibido {raw:?})"
        )))
    }
}

/// Where the app keeps its archive, and where the connector must look.
///
/// The platform directory holds every EntropIA variant's shared directory; the
/// override replaces that whole path. A profile nests one level deeper, under
/// the same `dev-profiles` directory the app uses.
pub fn resolve_data_dir(
    override_dir: Option<&Path>,
    profile: Option<&str>,
    platform_data: &Path,
) -> PathBuf {
    let shared = match override_dir {
        Some(dir) => dir.to_path_buf(),
        None => platform_data.join(SHARED_DIR_NAME),
    };
    match profile {
        Some(name) => shared.join(PROFILES_DIR).join(name),
        None => shared,
    }
}
