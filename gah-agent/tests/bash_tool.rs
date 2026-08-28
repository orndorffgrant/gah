use gah_agent::tools::{BashArgs, BashError, BashTool};
use rig::tool::{Tool, ToolContext};

async fn call(args: BashArgs) -> Result<<BashTool as Tool>::Output, <BashTool as Tool>::Error> {
    BashTool.call(&mut ToolContext::default(), args).await
}

#[tokio::test]
async fn echo_returns_exit_and_preview() {
    let out = call(BashArgs {
        command: "echo hello".into(),
        timeout_secs: None,
        blocking: None,
    })
    .await
    .unwrap();
    assert_eq!(out.exit_code, Some(0));
    assert_eq!(out.exit_log, None);
    assert!(out.preview.contains("hello"));
    let stderr = std::fs::read_to_string(&out.stderr_log).unwrap();
    assert!(stderr.is_empty());
}

#[tokio::test]
async fn failing_command_reports_exit_code_and_stderr_preview() {
    let out = call(BashArgs {
        command: "echo oops >&2; exit 3".into(),
        timeout_secs: None,
        blocking: None,
    })
    .await
    .unwrap();
    assert_eq!(out.exit_code, Some(3));
    assert!(out.preview.contains("oops"));
    let stdout = std::fs::read_to_string(&out.stdout_log).unwrap();
    assert!(stdout.is_empty());
}

#[tokio::test]
async fn long_command_is_killed_at_timeout() {
    let err = call(BashArgs {
        command: "sleep 30".into(),
        timeout_secs: Some(1),
        blocking: None,
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
        blocking: None,
    })
    .await
    .unwrap();
    assert_eq!(out.exit_code, Some(127));
    assert!(out.preview.contains("not found"));
}

#[tokio::test]
async fn background_returns_immediately_and_writes_exit_file() {
    let out = call(BashArgs {
        command: "sleep 1".into(),
        timeout_secs: None,
        blocking: Some(false),
    })
    .await
    .unwrap();
    assert!(!out.blocking);
    assert_eq!(out.exit_code, None);
    assert!(out.exit_log.is_some());
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let exit = std::fs::read_to_string(out.exit_log.unwrap()).unwrap();
    assert_eq!(exit.trim(), "0");
}
