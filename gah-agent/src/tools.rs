//! Tools the agent can call during a run.

use rig::tool::{Tool, ToolContext};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::{timeout, Duration};

/// The default number of seconds a bash command may run before it is killed.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Executes arbitrary bash commands on the host running gah-api.
///
/// Commands run with the API server's privileges: there is no sandboxing,
/// filtering, or confirmation step. Only expose sessions on hosts where the
/// model is trusted to run shell commands.
pub struct BashTool;

#[derive(Debug, Clone, Deserialize)]
pub struct BashArgs {
    /// The bash command to execute.
    pub command: String,
    /// Kill the command after this many seconds (default 60).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BashOutput {
    /// The command's exit code, or None if it could not be run.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Error)]
pub enum BashError {
    #[error("bash command timed out after {0}s and was killed")]
    Timeout(u64),
}

impl Tool for BashTool {
    const NAME: &'static str = "bash";
    type Args = BashArgs;
    type Output = BashOutput;
    type Error = BashError;

    fn description(&self) -> String {
        "Run a bash command on the host and return its exit code, stdout, and stderr.".into()
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
                    "description": "Kill the command after this many seconds (default 60)"
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
        let secs = args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let spawned = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(&args.command)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();

        let child = match spawned {
            Ok(child) => child,
            // Report spawn failures as output rather than a tool error: the
            // model cannot fix a missing bash binary, and the run should keep
            // going so it can tell the user.
            Err(e) => {
                return Ok(BashOutput {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("failed to spawn bash: {e}"),
                })
            }
        };

        match timeout(Duration::from_secs(secs), child.wait_with_output()).await {
            Ok(Ok(output)) => Ok(BashOutput {
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
            Ok(Err(e)) => Ok(BashOutput {
                exit_code: None,
                stdout: String::new(),
                stderr: format!("failed to run command: {e}"),
            }),
            Err(_) => Err(BashError::Timeout(secs)),
        }
    }
}
