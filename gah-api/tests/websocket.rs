//! Websocket streaming endpoint test: connect, send a prompt, and receive
//! an error event (the session's provider points at an unreachable port).

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

async fn spawn_server() -> (String, String, dropshot::HttpServer<std::sync::Arc<gah_api::ApiState>>)
{
    let state = gah_api::run::build_state("sqlite::memory:")
        .await
        .unwrap_or_else(|e| panic!("build_state: {e}"));
    let token = state.token_store.create_token("test").await.unwrap().token;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    let server = gah_api::run::start(addr, state)
        .await
        .unwrap_or_else(|e| panic!("start: {e}"));
    (format!("http://{}", server.local_addr()), token, server)
}

#[tokio::test]
async fn websocket_streams_agent_events() {
    let (base, token, server) = spawn_server().await;
    let client = reqwest::Client::new();

    // Session pointed at a port nothing listens on, so the agent run fails.
    let resp = client
        .post(format!("{base}/v1/sessions"))
        .bearer_auth(&token)
        .json(&json!({
            "config": {
                "provider": "custom",
                "model": "test-model",
                "api_key": "k",
                "api_base_url": "http://127.0.0.1:1/"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let created: serde_json::Value = resp.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let ws_url = format!("{base}/v1/sessions/{id}/stream").replacen("http", "ws", 1);
    let mut request = ws_url.into_client_request().unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().unwrap(),
    );
    let (mut ws, _resp) = tokio_tungstenite::connect_async(request).await.unwrap();

    ws.send(Message::text("hello")).await.unwrap();

    // The run must fail (unreachable provider) and surface an error event.
    let mut got_error = false;
    for _ in 0..10 {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
            .await
            .expect("frame within timeout")
            .expect("stream open")
            .expect("frame ok");
        let text = frame.into_text().unwrap().to_string();
        let event: serde_json::Value = serde_json::from_str(&text).unwrap();
        match event["type"].as_str() {
            Some("error") => {
                assert!(!event["message"].as_str().unwrap().is_empty());
                got_error = true;
                break;
            }
            Some("text_delta") | Some("tool_call") | Some("tool_result") | Some("done") => {}
            other => panic!("unexpected event type: {other:?}"),
        }
    }
    assert!(got_error, "expected an error event from the stream");

    server.close().await.unwrap();
}
