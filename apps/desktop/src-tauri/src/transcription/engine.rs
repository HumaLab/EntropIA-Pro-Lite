/// Whisper transcription engine — subprocess-based via faster-whisper (Python).
///
/// Calls the `transcribe.py` script as a child process instead of linking
/// whisper.cpp directly. This avoids C++ foreign exceptions that crash Rust
/// on Windows (F16C/AVX2 issues in ggml).
///
/// The Python process is isolated — if it crashes, we catch it as a
/// `Result::Err` instead of a hard abort that kills the entire app.
#[cfg(feature = "local-ml")]
use std::io::Read;
#[cfg(feature = "local-ml")]
use std::path::PathBuf;
#[cfg(feature = "local-ml")]
use std::process::{Command, Output};
#[cfg(feature = "local-ml")]
use std::time::{Duration, Instant};

#[cfg(all(windows, feature = "local-ml"))]
use std::os::windows::process::CommandExt;

#[cfg(all(windows, feature = "local-ml"))]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Wall-clock cap for the transcription subprocess. Generous enough for a first-run
/// model download plus transcription on a slow CPU/network, but bounded so a stalled
/// download (flaky network, captive portal) can't wedge the worker thread forever.
#[cfg(feature = "local-ml")]
const TRANSCRIPTION_TIMEOUT_SECS: u64 = 1800;

#[cfg(feature = "local-ml")]
fn apply_windows_no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        _cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Read a child stdout/stderr pipe to EOF on its own thread, so polling try_wait()
/// never deadlocks against a full pipe buffer.
#[cfg(feature = "local-ml")]
fn drain_child_pipe(mut pipe: impl Read + Send + 'static) -> Vec<u8> {
    let mut buffer = Vec::new();
    let _ = pipe.read_to_end(&mut buffer);
    buffer
}

#[cfg(feature = "local-ml")]
fn join_child_reader(handle: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

/// Configuration for the transcription engine.
#[cfg(feature = "local-ml")]
#[derive(Clone)]
pub struct WhisperConfig {
    /// Path to the Python interpreter or transcribe.py executable.
    pub python_path: PathBuf,
    /// Path to the transcribe.py script.
    pub script_path: PathBuf,
    /// Whisper model size: "tiny", "base", "small", "medium", "large-v3".
    pub model_size: String,
    /// Language code (e.g. "es", "en"). Use "auto" for auto-detection.
    pub language: String,
    /// Compute type: "int8" (fast, universal), "float16", "float32".
    pub compute_type: String,
    /// Directory to cache models (None = use faster-whisper default).
    pub model_dir: Option<PathBuf>,
}

/// Transcription result matching the Python script's JSON output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptionResult {
    /// Full transcription text (all segments joined).
    pub text: String,
    /// Detected or configured language.
    pub language: String,
    /// Timestamped segments.
    pub segments: Vec<Segment>,
    /// Duration of the audio in milliseconds.
    pub duration_ms: u64,
}

/// A single timestamped segment from faster-whisper.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// The transcription engine — spawns Python as a child process.
///
/// Unlike the whisper-rs engine, this is NOT created at startup.
/// Each transcription call spawns a fresh Python process, which:
/// - Loads the model (cached by faster-whisper after first download)
/// - Transcribes the audio file
/// - Outputs JSON to stdout
/// - Exits (freeing all memory)
#[cfg(feature = "local-ml")]
pub struct WhisperEngine {
    config: WhisperConfig,
}

#[cfg(feature = "local-ml")]
impl WhisperEngine {
    /// Validate configuration. The model is loaded per-call by the Python process.
    ///
    /// NOTE: Python interpreter was already validated by `which_python_for_module()`
    /// which ran `import faster_whisper; print('ok')` successfully. No redundant
    /// verification needed — the discovery module already proved it works.
    pub fn init(config: WhisperConfig) -> Result<Self, String> {
        // Verify the script exists
        if !config.script_path.exists() {
            return Err(format!(
                "Transcription script not found: {}",
                config.script_path.display()
            ));
        }

        eprintln!(
            "[transcription] Engine configured: python={}, script={}, model={}, compute_type={}",
            config.python_path.display(),
            config.script_path.display(),
            config.model_size,
            config.compute_type,
        );

        Ok(Self { config })
    }

    /// Transcribe an audio file by spawning the Python script.
    ///
    /// The script receives the audio path and outputs JSON segments to stdout,
    /// wrapped in sentinel markers for reliable extraction.
    /// Progress and errors go to stderr — we capture both.
    pub fn transcribe(
        &self,
        audio_path: &str,
        duration_ms: u64,
    ) -> Result<TranscriptionResult, String> {
        eprintln!("[transcription] Spawning Python transcription for: {audio_path}");

        let mut cmd = Command::new(&self.config.python_path);
        apply_windows_no_window(&mut cmd);
        cmd.arg(&self.config.script_path)
            .arg(audio_path)
            .arg("--model")
            .arg(&self.config.model_size)
            .arg("--language")
            .arg(&self.config.language)
            .arg("--compute-type")
            .arg(&self.config.compute_type);

        if let Some(ref model_dir) = self.config.model_dir {
            cmd.arg("--model-dir").arg(model_dir);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Spawn and wait with a wall-clock timeout. The pipes are drained on
        // background threads so a chatty child (e.g. faster-whisper download
        // progress on stderr) can't fill the ~64 KB pipe buffer and deadlock.
        // Without this, a stalled model download on a slow/flaky network or
        // captive portal would block cmd.output() forever and wedge the
        // transcription worker thread, queuing every later job behind it.
        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to spawn Python process (python={}): {e}",
                self.config.python_path.display()
            )
        })?;

        let stdout_reader = child
            .stdout
            .take()
            .map(|stdout| std::thread::spawn(move || drain_child_pipe(stdout)));
        let stderr_reader = child
            .stderr
            .take()
            .map(|stderr| std::thread::spawn(move || drain_child_pipe(stderr)));

        let timeout = Duration::from_secs(TRANSCRIPTION_TIMEOUT_SECS);
        let started_at = Instant::now();
        let output = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let _ = child.wait();
                    break Output {
                        status,
                        stdout: join_child_reader(stdout_reader),
                        stderr: join_child_reader(stderr_reader),
                    };
                }
                Ok(None) if started_at.elapsed() >= timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "Transcription timed out after {}s (python={}). The model download or \
                         transcription did not finish — check your network connection and try again.",
                        timeout.as_secs(),
                        self.config.python_path.display()
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "Failed while waiting for transcription process: {e}"
                    ));
                }
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            let exit_code = output.status.code().unwrap_or(-1);
            return Err(format!(
                "Transcription script failed (exit code {exit_code}).\n\
                 Python: {}\n\
                 Script: {}\n\
                 Stderr: {}\n\
                 Stdout: {}",
                self.config.python_path.display(),
                self.config.script_path.display(),
                stderr.trim(),
                stdout.trim(),
            ));
        }

        // Extract JSON between sentinel markers for safe parsing
        let json_str = extract_sentinel_json(&stdout);

        let segments: Vec<Segment> = serde_json::from_str(json_str).map_err(|e| {
            format!(
                "Failed to parse transcription JSON: {e}\n\
                 Extracted: {}\n\
                 Full stdout ({} bytes): {}\n\
                 Stderr: {}",
                if json_str.len() > 200 {
                    &json_str[..200]
                } else {
                    json_str
                },
                stdout.len(),
                if stdout.len() > 500 {
                    &stdout[..500.min(stdout.len())]
                } else {
                    &stdout
                },
                stderr.trim(),
            )
        })?;

        // Extract language from stderr logs
        let language = stderr
            .lines()
            .find(|l| l.contains("language="))
            .and_then(|l| l.split("language=").nth(1))
            .and_then(|s| s.split(',').next())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| self.config.language.clone());

        let full_text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        eprintln!(
            "[transcription] Python transcription complete: {} segments, {} chars, lang={}",
            segments.len(),
            full_text.len(),
            language
        );

        Ok(TranscriptionResult {
            text: full_text,
            language,
            segments,
            duration_ms,
        })
    }
}

/// Extract JSON content between `===TRANSCRIPTION_JSON_BEGIN===` and
/// `===TRANSCRIPTION_JSON_END===` sentinels. Falls back to the full output
/// if sentinels are not found (backwards compatibility).
#[cfg(feature = "local-ml")]
fn extract_sentinel_json(output: &str) -> &str {
    const BEGIN: &str = "===TRANSCRIPTION_JSON_BEGIN===";
    const END: &str = "===TRANSCRIPTION_JSON_END===";

    if let Some(start_idx) = output.find(BEGIN) {
        let content_start = start_idx + BEGIN.len();
        if let Some(end_idx) = output[content_start..].find(END) {
            let json_content = &output[content_start..content_start + end_idx];
            return json_content.trim();
        }
    }

    // Fallback: return the full trimmed output (backwards compat)
    output.trim()
}

#[cfg(all(test, feature = "local-ml"))]
mod tests {
    use super::*;

    #[test]
    fn test_init_rejects_missing_script() {
        let config = WhisperConfig {
            python_path: std::path::PathBuf::from("python"),
            script_path: std::path::PathBuf::from("nonexistent.py"),
            model_size: "base".to_string(),
            language: "es".to_string(),
            compute_type: "int8".to_string(),
            model_dir: None,
        };
        let result = WhisperEngine::init(config);
        assert!(result.is_err(), "Should fail with missing script");
    }
}
