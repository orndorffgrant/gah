use gah_agent::tools::{BashArgs, BashError, BashTool};
use rig::tool::{Tool, ToolContext};

async fn call(args: BashArgs) -> Result<<BashTool as Tool>::Output, <BashTool as Tool>::Error> {
    BashTool.call(&mut ToolContext::default(), args).await
}

#[tokio::test]
async fn echo_returns_stdout_and_zero_exit() {
    let out = call(BashArgs {
        command: "echo hello".into(),
        timeout_secs: None,
    })
    .await
    .unwrap();
    assert_eq!(out.exit_code, Some(0));
    assert_eq!(out.stdout.trim(), "hello");
    assert!(out.stderr.is_empty());
}

#[tokio::test]
async fn failing_command_reports_exit_code_and_stderr() {
    let out = call(BashArgs {
        command: "echo oops >&2; exit 3".into(),
        timeout_secs: None,
    })
    .await
    .unwrap();
    assert_eq!(out.exit_code, Some(3));
    assert!(out.stdout.is_empty());
    assert_eq!(out.stderr.trim(), "oops");
}

#[tokio::test]
async fn long_command_is_killed_at_timeout() {
    let err = call(BashArgs {
        command: "sleep 30".into(),
        timeout_secs: Some(1),
    })
    .await
    .unwrap_err();
    assert!(matches!(err, BashError::Timeout(1)));
}

#[tokio::test]
async fn missing_binary_is_reported_not_fatal() {
    let out = call(BashArgs {
        command: "definitely-not-a-real-binary-xyz".into(),
        timeout_secs: None,
    })
    .await
    .unwrap();
    assert_eq!(out.exit_code, Some(127));
    assert!(out.stderr.contains("not found"));
}
