//! One-click registration. Only the entropia server is changed; no token is read.

use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupError {
    pub code: &'static str,
    pub assistant: String,
    pub detail: String,
    pub exit_code: Option<i32>,
}
impl SetupError {
    pub fn new(code: &'static str, assistant: &str, detail: impl ToString) -> Self {
        Self {
            code,
            assistant: label(assistant).into(),
            detail: detail.to_string(),
            exit_code: None,
        }
    }
}
fn label(id: &str) -> &str {
    match id {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "pi" => "Pi",
        _ => id,
    }
}

/// Paths are injected so registration tests never inspect the owner's config.
pub struct ConfigPaths {
    pub opencode: PathBuf,
    pub pi: PathBuf,
}
impl ConfigPaths {
    pub fn from_home(home: &Path, pi_override: Option<std::ffi::OsString>) -> Self {
        let pi = pi_override
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .map(|p| {
                p.to_str()
                    .and_then(|s| s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")))
                    .map(|s| home.join(s))
                    .unwrap_or(p)
            })
            .unwrap_or_else(|| home.join(".pi/agent"));
        Self {
            opencode: home.join(".config/opencode"),
            pi,
        }
    }
}
#[derive(Debug)]
pub struct RunOutput {
    pub code: Option<i32>,
    pub stderr: String,
}
pub trait Runner {
    fn run(
        &self,
        assistant: &str,
        args: &[String],
    ) -> impl std::future::Future<Output = Result<RunOutput, SetupError>> + Send;
}

/// Deliberately no shell, including Windows batch execution. A known Node shim
/// is data: its boilerplate is checked but never evaluated.
pub struct ProcessRunner {
    pub search: Vec<PathBuf>,
}
impl ProcessRunner {
    pub fn from_environment() -> Self {
        Self {
            search: std::env::var_os("PATH")
                .map(|p| {
                    std::env::split_paths(&p)
                        .filter(|p| p.is_absolute())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
    fn find(&self, name: &str, scripts: bool) -> Option<PathBuf> {
        let suffixes: &[&str] = if cfg!(windows) {
            if scripts {
                &[".exe", ".com", ".cmd", ".bat", ".ps1"]
            } else {
                &[".exe", ".com"]
            }
        } else {
            &[""]
        };
        self.search
            .iter()
            .flat_map(|dir| {
                suffixes
                    .iter()
                    .map(move |suffix| dir.join(format!("{name}{suffix}")))
            })
            .find(|p| p.is_file())
    }
    fn invocation(&self, assistant: &str) -> Result<(PathBuf, Vec<String>), SetupError> {
        let path = self
            .find(assistant, true)
            .ok_or_else(|| SetupError::new("not_found", assistant, ""))?;
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["cmd", "bat", "ps1"].contains(&ext.as_str()) {
            return Ok((path, vec![]));
        }
        let unsupported = || SetupError::new("unsafe_wrapper", assistant, "");
        if ext != "cmd" {
            return Err(unsupported());
        }
        let content = std::fs::read_to_string(&path).map_err(|_| unsupported())?;
        let relative = parse_node_shim(&content).ok_or_else(unsupported)?;
        let dir = path.parent().ok_or_else(unsupported)?;
        let entry = dir
            .join(relative)
            .canonicalize()
            .map_err(|_| unsupported())?;
        if !entry.is_file() {
            return Err(unsupported());
        }
        let local_node = dir.join("node.exe");
        let node = if local_node.is_file() {
            local_node
        } else {
            self.find("node", false)
                .ok_or_else(|| SetupError::new("not_found", assistant, "node"))?
        };
        Ok((node, vec![entry.to_string_lossy().into_owned()]))
    }
}
impl Runner for ProcessRunner {
    async fn run(&self, assistant: &str, args: &[String]) -> Result<RunOutput, SetupError> {
        let (program, prefix) = self.invocation(assistant)?;
        let mut command = tokio::process::Command::new(program);
        command
            .args(prefix)
            .args(args)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
            .await
            .map_err(|_| SetupError::new("timeout", assistant, ""))?
            .map_err(|e| SetupError::new("io", assistant, e))?;
        Ok(RunOutput {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
        })
    }
}

/// Accept the npm/pnpm Node-launch forms only. Unknown statements, extra
/// arguments, environment expansions in the entry and inconsistent branches
/// fail closed. NODE_PATH boilerplate is ignored: Node resolves the package's
/// own dependencies relative to the entry, not through a wrapper's environment.
fn parse_node_shim(text: &str) -> Option<String> {
    let mut entry: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('@');
        if line.is_empty() {
            continue;
        }
        let launch = line
            .strip_prefix("endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & ")
            .unwrap_or(line);
        let rest = ["\"%_prog%\" ", "\"%~dp0\\node.exe\" ", "node "]
            .iter()
            .find_map(|prefix| launch.strip_prefix(prefix));
        if let Some(rest) = rest {
            let quoted = rest
                .strip_suffix(" %*")?
                .strip_prefix('"')?
                .strip_suffix('"')?;
            let relative = quoted
                .strip_prefix("%dp0%\\")
                .or_else(|| quoted.strip_prefix("%~dp0\\"))?;
            if !relative.ends_with(".js")
                && !relative.ends_with(".cjs")
                && !relative.ends_with(".mjs")
            {
                return None;
            }
            if relative
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || "_@./\\-".contains(c)))
            {
                return None;
            }
            let relative = relative.replace('\\', "/");
            if !(relative.starts_with("node_modules/") || relative.starts_with("../")) {
                return None;
            }
            if entry.as_ref().is_some_and(|old| old != &relative) {
                return None;
            }
            entry = Some(relative);
            continue;
        }
        if [
            "ECHO off",
            "GOTO start",
            ":find_dp0",
            "SET dp0=%~dp0",
            "EXIT /b",
            ":start",
            "SETLOCAL",
            "CALL :find_dp0",
            "IF EXIST \"%dp0%\\node.exe\" (",
            "IF EXIST \"%~dp0\\node.exe\" (",
            "SET \"_prog=%dp0%\\node.exe\"",
            "SET \"_prog=node\"",
            "SET PATHEXT=%PATHEXT:;.JS;=;%",
            ") ELSE (",
            ")",
            "IF NOT DEFINED NODE_PATH (",
        ]
        .contains(&line)
        {
            continue;
        }
        if line.starts_with("SET \"NODE_PATH=")
            && line.ends_with('"')
            && !line.contains(['&', '|', '<', '>'])
        {
            continue;
        }
        return None;
    }
    entry
}

pub async fn install(
    assistant: &str,
    client_id: &str,
    connector: &str,
    paths: &ConfigPaths,
    runner: &impl Runner,
) -> Result<(), SetupError> {
    let run = |parts: Vec<&str>| {
        let args: Vec<String> = parts.into_iter().map(String::from).collect();
        async move { runner.run(assistant, &args).await }
    };
    match assistant {
        "codex" => check_output(
            assistant,
            run(vec![
                "mcp", "add", "entropia", "--", connector, "--client", client_id,
            ])
            .await?,
            false,
        ),
        "claude" => {
            for scope in ["user", "local"] {
                check_output(
                    assistant,
                    run(vec!["mcp", "remove", "entropia", "-s", scope]).await?,
                    true,
                )?;
            }
            check_output(
                assistant,
                run(vec![
                    "mcp", "add", "-s", "user", "entropia", "--", connector, "--client", client_id,
                ])
                .await?,
                false,
            )
        }
        "opencode" => merge(
            assistant,
            &paths.opencode,
            "opencode.json",
            "mcp",
            json!({"type":"local","command":[connector,"--client",client_id]}),
        ),
        "pi" => merge(
            assistant,
            &paths.pi,
            "mcp.json",
            "mcpServers",
            json!({"command":connector,"args":["--client",client_id]}),
        ),
        _ => Err(SetupError::new("not_found", assistant, "")),
    }
}
fn check_output(assistant: &str, output: RunOutput, removing: bool) -> Result<(), SetupError> {
    let stderr = output.stderr.to_ascii_lowercase();
    if output.code == Some(0)
        || (removing
            && output.code == Some(1)
            && (stderr.contains("no mcp server found with name: entropia")
                || stderr.contains("mcp server entropia not found")))
    {
        return Ok(());
    }
    Err(SetupError {
        code: "command_failed",
        assistant: label(assistant).into(),
        detail: output.stderr,
        exit_code: output.code,
    })
}
fn merge(
    assistant: &str,
    dir: &Path,
    file: &str,
    key: &str,
    entry: Value,
) -> Result<(), SetupError> {
    if !dir.is_dir() {
        return Err(SetupError::new("not_found", assistant, ""));
    }
    let path = dir.join(file);
    let invalid = || SetupError::new("invalid_config", assistant, path.display());
    // Do not replace a symlink or follow it outside this assistant's config.
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid());
    }
    let mut value: Value = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(SetupError::new("io", assistant, e)),
    };
    let root = value.as_object_mut().ok_or_else(invalid)?;
    let servers = root
        .entry(key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(invalid)?;
    servers.insert("entropia".into(), entry);
    let bytes =
        serde_json::to_vec_pretty(&value).map_err(|e| SetupError::new("io", assistant, e))?;
    super::access::write_atomic(&path, &bytes).map_err(|e| SetupError::new("io", assistant, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        replies: Mutex<std::collections::VecDeque<Result<RunOutput, SetupError>>>,
    }
    impl Runner for Fake {
        async fn run(&self, assistant: &str, args: &[String]) -> Result<RunOutput, SetupError> {
            self.calls
                .lock()
                .unwrap()
                .push((assistant.into(), args.to_vec()));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(RunOutput {
                    code: Some(0),
                    stderr: String::new(),
                }))
        }
    }
    fn paths(dir: &std::path::Path) -> ConfigPaths {
        ConfigPaths {
            opencode: dir.join("opencode"),
            pi: dir.join("pi"),
        }
    }
    #[tokio::test]
    async fn codex_exact_arguments_and_failure() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Fake::default();
        install(
            "codex",
            "cl_test",
            "/connector with spaces",
            &paths(dir.path()),
            &runner,
        )
        .await
        .unwrap();
        assert_eq!(
            *runner.calls.lock().unwrap(),
            vec![(
                "codex".into(),
                vec![
                    "mcp",
                    "add",
                    "entropia",
                    "--",
                    "/connector with spaces",
                    "--client",
                    "cl_test"
                ]
                .into_iter()
                .map(String::from)
                .collect()
            )]
        );
        runner.replies.lock().unwrap().push_back(Ok(RunOutput {
            code: Some(7),
            stderr: "permission denied".into(),
        }));
        let error = install(
            "codex",
            "cl_test",
            "/connector",
            &paths(dir.path()),
            &runner,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "command_failed");
        assert_eq!(error.exit_code, Some(7));
        assert_eq!(error.detail, "permission denied");
        assert!(!format!("{:?}", runner.calls).contains("token"));
    }
    #[tokio::test]
    async fn claude_removes_both_scopes_before_adding_and_only_tolerates_absence() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Fake::default();
        for _ in 0..2 {
            runner.replies.lock().unwrap().push_back(Ok(RunOutput {
                code: Some(1),
                stderr: "No MCP server found with name: entropia".into(),
            }));
        }
        install(
            "claude",
            "cl_test",
            "/connector",
            &paths(dir.path()),
            &runner,
        )
        .await
        .unwrap();
        let calls = runner.calls.lock().unwrap().clone();
        assert_eq!(
            calls.iter().map(|c| c.1.join(" ")).collect::<Vec<_>>(),
            vec![
                "mcp remove entropia -s user",
                "mcp remove entropia -s local",
                "mcp add -s user entropia -- /connector --client cl_test"
            ]
        );
        runner.replies.lock().unwrap().push_back(Ok(RunOutput {
            code: Some(2),
            stderr: "access denied".into(),
        }));
        assert_eq!(
            install(
                "claude",
                "cl_test",
                "/connector",
                &paths(dir.path()),
                &runner
            )
            .await
            .unwrap_err()
            .detail,
            "access denied"
        );
        assert_eq!(runner.calls.lock().unwrap().len(), 4);
    }
    #[tokio::test]
    async fn json_merge_create_replace_and_refuse_without_changes() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let runner = Fake::default();
        for (assistant, folder, file, key, entry) in [
            (
                "opencode",
                &paths.opencode,
                "opencode.json",
                "mcp",
                json!({"type":"local","command":["/connector","--client","cl_test"]}),
            ),
            (
                "pi",
                &paths.pi,
                "mcp.json",
                "mcpServers",
                json!({"command":"/connector","args":["--client","cl_test"]}),
            ),
        ] {
            assert_eq!(
                install(assistant, "cl_test", "/connector", &paths, &runner)
                    .await
                    .unwrap_err()
                    .code,
                "not_found"
            );
            assert!(!folder.exists());
            std::fs::create_dir(folder).unwrap();
            install(assistant, "cl_test", "/connector", &paths, &runner)
                .await
                .unwrap();
            let path = folder.join(file);
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(value, json!({key: {"entropia": entry.clone()}}));
            std::fs::write(&path, serde_json::to_vec(&json!({"theme":"dark",key:{"other":{"command":"keep"},"entropia":{"old":true}}})).unwrap()).unwrap();
            install(assistant, "cl_test", "/connector", &paths, &runner)
                .await
                .unwrap();
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(
                value,
                json!({"theme":"dark",key:{"other":{"command":"keep"},"entropia":entry}})
            );
            for invalid in ["not JSON", "[]", "{\"mcp\":false,\"mcpServers\":false}"] {
                std::fs::write(&path, invalid).unwrap();
                assert_eq!(
                    install(assistant, "cl_test", "/connector", &paths, &runner)
                        .await
                        .unwrap_err()
                        .code,
                    "invalid_config"
                );
                assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
                assert_eq!(std::fs::read_dir(folder).unwrap().count(), 1);
            }
        }
        assert!(runner.calls.lock().unwrap().is_empty());
    }
    #[test]
    fn npm_and_pnpm_shims_are_parsed_not_executed() {
        let npm = "@ECHO off\nGOTO start\n:find_dp0\nSET dp0=%~dp0\nEXIT /b\n:start\nSETLOCAL\nCALL :find_dp0\nIF EXIST \"%dp0%\\node.exe\" (\n SET \"_prog=%dp0%\\node.exe\"\n) ELSE (\n SET \"_prog=node\"\n SET PATHEXT=%PATHEXT:;.JS;=;%\n)\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\" \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*\n";
        assert_eq!(
            parse_node_shim(npm),
            Some("node_modules/@openai/codex/bin/codex.js".into())
        );
        let pnpm = "@SETLOCAL\n@IF EXIST \"%~dp0\\node.exe\" (\n  \"%~dp0\\node.exe\" \"%~dp0\\..\\@anthropic-ai\\claude-code\\cli.js\" %*\n) ELSE (\n  @SET PATHEXT=%PATHEXT:;.JS;=;%\n  node \"%~dp0\\..\\@anthropic-ai\\claude-code\\cli.js\" %*\n)\n";
        assert_eq!(
            parse_node_shim(pnpm),
            Some("../@anthropic-ai/claude-code/cli.js".into())
        );
        assert_eq!(parse_node_shim(&format!("{npm}\npowershell evil")), None);
        assert_eq!(parse_node_shim("@echo off\nnode evil.js %*"), None);
        assert_eq!(
            parse_node_shim(&npm.replace("codex.js", "codex.js&evil")),
            None
        );
    }
    #[test]
    fn config_paths_use_the_documented_home_and_pi_override() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let defaults = ConfigPaths::from_home(home, None);
        assert_eq!(defaults.opencode, home.join(".config/opencode"));
        assert_eq!(defaults.pi, home.join(".pi/agent"));
        let custom = home.join("custom-agent");
        assert_eq!(
            ConfigPaths::from_home(home, Some(custom.clone().into_os_string())).pi,
            custom
        );
        assert_eq!(
            ConfigPaths::from_home(home, Some("~/custom-agent".into())).pi,
            home.join("custom-agent")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_resolution_uses_node_and_never_a_script_interpreter() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessRunner {
            search: vec![dir.path().to_path_buf()],
        };
        let package = dir.path().join("node_modules/codex/bin");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("codex.js"), "// fixture").unwrap();
        std::fs::write(dir.path().join("node.exe"), "fixture").unwrap();
        std::fs::write(
            dir.path().join("codex.cmd"),
            "@SETLOCAL\nnode \"%~dp0\\node_modules\\codex\\bin\\codex.js\" %*\n",
        )
        .unwrap();
        let (program, args) = runner.invocation("codex").unwrap();
        assert_eq!(program, dir.path().join("node.exe"));
        assert_eq!(
            args,
            vec![package
                .join("codex.js")
                .canonicalize()
                .unwrap()
                .to_string_lossy()]
        );
        std::fs::write(dir.path().join("codex.cmd"), "@powershell evil").unwrap();
        assert_eq!(
            runner.invocation("codex").unwrap_err().code,
            "unsafe_wrapper"
        );
        std::fs::write(dir.path().join("claude.ps1"), "evil").unwrap();
        assert_eq!(
            runner.invocation("claude").unwrap_err().code,
            "unsafe_wrapper"
        );
        std::fs::write(dir.path().join("codex.exe"), "fixture").unwrap();
        assert_eq!(
            runner.invocation("codex").unwrap(),
            (dir.path().join("codex.exe"), vec![])
        );
    }

    #[tokio::test]
    async fn missing_cli_is_reported_without_running_anything() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessRunner {
            search: vec![dir.path().to_path_buf()],
        };
        assert_eq!(
            runner.run("codex", &["mcp".into()]).await.unwrap_err().code,
            "not_found"
        );
    }
}
