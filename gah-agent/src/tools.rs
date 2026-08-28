//! Tools the agent can call during a run.
//!
//! - [`BashTool`]: runs a shell command. Output is always captured to tmp log
//!   files; the model receives the file paths (plus a bounded preview) rather
//!   than the full stream. With `blocking: false` the command is detached and
//!   the model can poll the log files / exit file asynchronously.
//! - [`WebSearchTool`] / [`WebFetchTool`]: Exa-backed web search and fetch
//!   (the same provider zerostack uses), keyed off `$EXA_API_KEY`.

use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rig::tool::{Tool, ToolContext};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::timeout;

/// Where bash command output is parked for the agent to read back.
fn log_dir() -> PathBuf {
    std::env::temp_dir().join("gah-bash")
}

fn job_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{n}")
}

/// Tail the last `max_bytes` bytes of a file as a String (lossy utf-8).
fn tail(path: &PathBuf, max_bytes: usize) -> String {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    if buf.len() > max_bytes {
        String::from_utf8_lossy(&buf[buf.len() - max_bytes..]).into_owned()
    } else {
        String::from_utf8_lossy(&buf).into_owned()
    }
}

/// ── BashTool ──────────────────────────────────────────────────────
pub struct BashTool;

#[derive(Debug, Clone, Deserialize)]
pub struct BashArgs {
    /// The bash command to execute.
    pub command: String,
    /// Kill the command after this many seconds (default 60). Ignored for
    /// background (`blocking: false`) jobs, which run until they finish.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Wait for the command to finish and report its exit code (default
    /// `true`). When `false` the command is detached immediately and the
    /// model is given the log file paths plus an exit file to poll.
    #[serde(default)]
    pub blocking: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BashOutput {
    /// Unique id for this invocation.
    pub job_id: String,
    /// Whether the call waited for completion.
    pub blocking: bool,
    /// Path to the file capturing the command's stdout.
    pub stdout_log: String,
    /// Path to the file capturing the command's stderr.
    pub stderr_log: String,
    /// For background jobs: path to a file that will hold the exit code once
    /// the command finishes. `None` for blocking jobs (see `exit_code`).
    pub exit_log: Option<String>,
    /// Exit code. `None` if the job is still running or failed to spawn.
    pub exit_code: Option<i32>,
    /// True if a blocking job was killed after `timeout_secs`.
    pub timed_out: bool,
    /// Bounded tail (~4 KB) of combined stdout/stderr so the model has an
    /// immediate signal without reading the full log files.
    pub preview: String,
}

#[derive(Debug, Error)]
pub enum BashError {
    #[error("bash command timed out after {0}s and was killed")]
    Timeout(u64),
}

const DEFAULT_TIMEOUT_SECS: u64 = 60;
const PREVIEW_BYTES: usize = 4096;

impl Tool for BashTool {
    const NAME: &'static str = "bash";
    type Args = BashArgs;
    type Output = BashOutput;
    type Error = BashError;

    fn description(&self) -> String {
        "Run a bash command on the host. Output is written to tmp log files; \
         you receive the file paths (plus a small preview), not the full \
         output — read the files (e.g. `cat`/`tail`) when you need detail. \
         By default the call blocks until the command finishes. Set \
         `blocking: false` to detach a long-running command: you get the log \
         paths and an `exit_log` file immediately; poll them with follow-up \
         calls (e.g. `tail -n 50 <stdout_log>`, `cat <exit_log>`) to watch \
         progress and completion.".into()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Kill the command after this many seconds (default 60). Ignored for background jobs."
                },
                "blocking": {
                    "type": "boolean",
                    "description": "Wait for completion (default true). False detaches the command into the background."
                }
            },
            "required": ["command"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let blocking = args.blocking.unwrap_or(true);
        let _ = fs::create_dir_all(log_dir());
        let id = job_id();
        let stdout_path = log_dir().join(format!("{id}.out.log"));
        let stderr_path = log_dir().join(format!("{id}.err.log"));
        let exit_path = log_dir().join(format!("{id}.exit"));

        let stdout_file = match File::create(&stdout_path) {
            Ok(f) => f,
            Err(e) => {
                return Ok(spawn_failed_output(&id, blocking, &stdout_path, &stderr_path, e));
            }
        };
        let stderr_file = match File::create(&stderr_path) {
            Ok(f) => f,
            Err(e) => {
                return Ok(spawn_failed_output(&id, blocking, &stdout_path, &stderr_path, e));
            }
        };

        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c")
            .arg(&args.command)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            // Keep the child alive after this handle drops so background jobs
            // survive; blocking jobs manage their own lifecycle below.
            .kill_on_drop(false);

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ok(spawn_failed_output(&id, blocking, &stdout_path, &stderr_path, e));
            }
        };

        if !blocking {
            // Detach: a watcher task waits for the child and records the exit
            // code once it finishes. We return immediately.
            let exit_log = exit_path.to_string_lossy().into_owned();
            tokio::spawn(async move {
                let mut child = child;
                let code = child.wait().await.ok().and_then(|s| s.code());
                let _ = fs::write(
                    &exit_path,
                    code.map(|c| c.to_string()).unwrap_or_else(|| "none".into()),
                );
            });
            return Ok(BashOutput {
                job_id: id,
                blocking: false,
                stdout_log: stdout_path.to_string_lossy().into(),
                stderr_log: stderr_path.to_string_lossy().into(),
                exit_log: Some(exit_log),
                exit_code: None,
                timed_out: false,
                preview: String::new(),
            });
        }

        // Blocking: wait up to timeout_secs, then report.
        let secs = args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let mut child = child;
        let wait = child.wait();
        match timeout(Duration::from_secs(secs), wait).await {
            Ok(Ok(status)) => Ok(BashOutput {
                job_id: id,
                blocking: true,
                stdout_log: stdout_path.to_string_lossy().into(),
                stderr_log: stderr_path.to_string_lossy().into(),
                exit_log: None,
                exit_code: status.code(),
                timed_out: false,
                preview: combined_tail(&stdout_path, &stderr_path),
            }),
            Ok(Err(e)) => {
                let _ = fs::write(&exit_path, format!("error: {e}"));
                Ok(BashOutput {
                    job_id: id,
                    blocking: true,
                    stdout_log: stdout_path.to_string_lossy().into(),
                    stderr_log: stderr_path.to_string_lossy().into(),
                    exit_log: None,
                    exit_code: None,
                    timed_out: false,
                    preview: format!("failed to wait on command: {e}"),
                })
            }
            Err(_) => {
                let _ = child.kill().await;
                Err(BashError::Timeout(secs))
            }
        }
    }
}

fn combined_tail(stdout_path: &PathBuf, stderr_path: &PathBuf) -> String {
    let out = tail(stdout_path, PREVIEW_BYTES / 2);
    let err = tail(stderr_path, PREVIEW_BYTES / 2);
    if out.is_empty() && err.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    if !out.is_empty() {
        s.push_str(&out);
    }
    if !err.is_empty() {
        if !s.is_empty() {
            s.push_str("\n--- stderr ---\n");
        }
        s.push_str(&err);
    }
    s
}

fn spawn_failed_output(
    id: &str,
    blocking: bool,
    stdout_path: &PathBuf,
    stderr_path: &PathBuf,
    e: std::io::Error,
) -> BashOutput {
    BashOutput {
        job_id: id.to_string(),
        blocking,
        stdout_log: stdout_path.to_string_lossy().into(),
        stderr_log: stderr_path.to_string_lossy().into(),
        exit_log: None,
        exit_code: None,
        timed_out: false,
        preview: format!("failed to set up bash: {e}"),
    }
}

/// ── Exa web tools ──────────────────────────────────────────────────
const EXA_BASE: &str = "https://api.exa.ai";

fn exa_key() -> Option<String> {
    std::env::var("EXA_API_KEY").ok().filter(|k| !k.is_empty())
}

#[derive(Debug, Error)]
pub enum WebError {
    #[error("EXA_API_KEY is not set; ask the operator to set it")]
    NoKey,
    #[error("exa request failed: {0}")]
    Request(String),
    #[error("exa returned an error: {0}")]
    Api(String),
}

fn other_reqwest<E: std::fmt::Display>(e: E) -> WebError {
    WebError::Request(e.to_string())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebResult {
    pub title: Option<String>,
    pub url: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebSearchOutput {
    pub query: String,
    pub results: Vec<WebResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebFetchOutput {
    pub results: Vec<WebResult>,
}

/// Search the web via Exa.
pub struct WebSearchTool;

#[derive(Debug, Clone, Deserialize)]
pub struct WebSearchArgs {
    /// A natural-language description of the ideal pages, not just keywords.
    pub query: String,
    /// Number of results to return (default 5).
    #[serde(default)]
    pub num_results: Option<u32>,
}

impl Tool for WebSearchTool {
    const NAME: &'static str = "web_search";
    type Args = WebSearchArgs;
    type Output = WebSearchOutput;
    type Error = WebError;

    fn description(&self) -> String {
        "Search the web with Exa. Describe the ideal page rather than \
         keywords. Returns titles, URLs, and short text snippets for each \
         result.".into()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Natural-language description of the ideal page" },
                "num_results": { "type": "integer", "description": "Number of results (default 5)" }
            },
            "required": ["query"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let key = exa_key().ok_or(WebError::NoKey)?;
        let num = args.num_results.unwrap_or(5);
        let body = serde_json::json!({
            "query": args.query,
            "numResults": num,
            "type": "auto",
            "contents": { "text": { "maxCharacters": 1000 } }
        });
        let resp = reqwest::Client::new()
            .post(format!("{EXA_BASE}/search"))
            .header("x-api-key", key)
            .json(&body)
            .send()
            .await
            .map_err(other_reqwest)?;
        let status = resp.status();
        let text = resp.text().await.map_err(other_reqwest)?;
        if !status.is_success() {
            return Err(WebError::Api(format!("HTTP {status}: {text}")));
        }
        let parsed: ExaResponse = serde_json::from_str(&text)
            .map_err(|e| WebError::Api(format!("bad response: {e}")))?;
        Ok(WebSearchOutput {
            query: args.query,
            results: parsed.results,
        })
    }
}

/// Fetch full content for one or more URLs via Exa.
pub struct WebFetchTool;

#[derive(Debug, Clone, Deserialize)]
pub struct WebFetchArgs {
    /// URLs to read. Batch several in one call.
    pub urls: Vec<String>,
    /// Maximum characters to extract per page (default 3000).
    #[serde(default)]
    pub max_characters: Option<usize>,
}

impl Tool for WebFetchTool {
    const NAME: &'static str = "web_fetch";
    type Args = WebFetchArgs;
    type Output = WebFetchOutput;
    type Error = WebError;

    fn description(&self) -> String {
        "Read a webpage's full content as clean text via Exa. Use after \
         web_search when snippets are insufficient, or to read any known \
         URL. Batch multiple URLs in one call.".into()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "urls": { "type": "array", "items": { "type": "string" }, "description": "URLs to read" },
                "max_characters": { "type": "integer", "description": "Max characters per page (default 3000)" }
            },
            "required": ["urls"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let key = exa_key().ok_or(WebError::NoKey)?;
        let max = args.max_characters.unwrap_or(3000);
        let body = serde_json::json!({
            "ids": args.urls,
            "contents": { "text": { "maxCharacters": max } }
        });
        let resp = reqwest::Client::new()
            .post(format!("{EXA_BASE}/contents"))
            .header("x-api-key", key)
            .json(&body)
            .send()
            .await
            .map_err(other_reqwest)?;
        let status = resp.status();
        let text = resp.text().await.map_err(other_reqwest)?;
        if !status.is_success() {
            return Err(WebError::Api(format!("HTTP {status}: {text}")));
        }
        let parsed: ExaResponse = serde_json::from_str(&text)
            .map_err(|e| WebError::Api(format!("bad response: {e}")))?;
        Ok(WebFetchOutput { results: parsed.results })
    }
}

#[derive(Debug, Deserialize)]
struct ExaResponse {
    #[serde(default)]
    results: Vec<WebResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_args_blocking_defaults_to_none() {
        let a: BashArgs = serde_json::from_str(r#"{"command":"echo hi"}"#).unwrap();
        assert_eq!(a.blocking, None);
        assert_eq!(a.timeout_secs, None);
    }

    #[test]
    fn bash_args_blocking_false_parses() {
        let a: BashArgs =
            serde_json::from_str(r#"{"command":"sleep 10","blocking":false}"#).unwrap();
        assert_eq!(a.blocking, Some(false));
    }

    #[tokio::test]
    async fn bash_blocking_captures_exit_and_preview() {
        let out = BashTool
            .call(&mut ToolContext::default(), BashArgs {
                command: "echo hello-out; echo hello-err >&2".into(),
                timeout_secs: Some(5),
                blocking: Some(true),
            })
            .await
            .unwrap();
        assert!(out.blocking);
        assert_eq!(out.exit_code, Some(0));
        assert!(out.preview.contains("hello-out"));
        assert!(out.preview.contains("hello-err"));
        assert!(out.stdout_log.ends_with(".out.log"));
        assert_eq!(out.exit_log, None);
    }

    #[tokio::test]
    async fn bash_background_returns_immediately_with_exit_log() {
        let out = BashTool
            .call(&mut ToolContext::default(), BashArgs {
                command: "sleep 2".into(),
                timeout_secs: None,
                blocking: Some(false),
            })
            .await
            .unwrap();
        assert!(!out.blocking);
        assert_eq!(out.exit_code, None);
        assert!(out.exit_log.is_some());
        // The watcher writes the exit file once the child finishes.
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let exit = fs::read_to_string(out.exit_log.unwrap()).unwrap();
        assert_eq!(exit.trim(), "0");
    }

    #[tokio::test]
    async fn bash_blocking_timeout_kills_and_errors() {
        let res = BashTool
            .call(&mut ToolContext::default(), BashArgs {
                command: "sleep 10".into(),
                timeout_secs: Some(1),
                blocking: Some(true),
            })
            .await;
        assert!(matches!(res, Err(BashError::Timeout(1))));
    }

    #[test]
    fn web_search_args_parse() {
        let a: WebSearchArgs =
            serde_json::from_str(r#"{"query":"rust async","num_results":3}"#).unwrap();
        assert_eq!(a.query, "rust async");
        assert_eq!(a.num_results, Some(3));
    }

    #[test]
    fn web_fetch_args_parse() {
        let a: WebFetchArgs =
            serde_json::from_str(r#"{"urls":["https://example.com"]}"#).unwrap();
        assert_eq!(a.urls, vec!["https://example.com".to_string()]);
        assert_eq!(a.max_characters, None);
    }
}