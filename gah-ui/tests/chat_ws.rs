//! End-to-end test of the chat websocket bridge: a browser websocket hits the
//! UI, which dials the API's streaming endpoint and forwards AgentEvent frames.

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

async fn spawn_api() -> (String, String, String) {
    let state = gah_api::run::build_state("sqlite::memory:").await.unwrap();
    let token = state.token_store.create_token("test").await.unwrap().token;

    // A session pointed at an unreachable provider: streaming it yields an
    // error event quickly, which is all the bridge needs to prove forwarding.
    let session = gah_core::Session::new(gah_core::AgentConfig {
        provider: gah_core::ProviderKind::Ollama,
        model: "test-model".into(),
        api_key: String::new(),
        api_base_url: Some("http://127.0.0.1:1".into()),
        system_prompt: None,
    });
    let sid = session.id.clone();
    state.session_store.create(&session).await.unwrap();

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    let server = gah_api::run::start(addr, state).await.unwrap();
    let url = format!("http://{}", server.local_addr());
    tokio::spawn(server);
    (url, token, sid)
}

async fn spawn_ui(api_url: &str, token: &str) -> (String, String) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let store = gah_ui::SessionStore::new(pool.clone());
    store.create_tables().await.unwrap();
    store.create_user("u", "p", "creator").await.unwrap();
    let ui_sid = store.create_session(1, "u", "creator").await.unwrap();

    let state: gah_ui::UiCtx = std::sync::Arc::new(gah_ui::UiState {
        db: pool,
        session_store: store,
        api: gah_ui::run::build_api_client(api_url, token).unwrap(),
    });
    let app = gah_ui::router(state);
    let listener = tokio::net::TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, ui_sid)
}

#[tokio::test]
async fn chat_ws_streams_agent_events() {
    let (api_url, token, session_id) = spawn_api().await;
    let (ui_url, ui_sid) = spawn_ui(&api_url, &token).await;
    let ui_host = ui_url.trim_start_matches("http://");

    // Missing session cookie: the upgrade must be refused.
    let anon = format!("ws://{ui_host}/sessions/{session_id}/ws")
        .into_client_request()
        .unwrap();
    assert!(tokio_tungstenite::connect_async(anon).await.is_err());

    // Authenticated: send a prompt, expect forwarded frames until a terminal
    // event, then the server closes the socket.
    let mut req = format!("ws://{ui_host}/sessions/{session_id}/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "cookie",
        format!("{}={ui_sid}", gah_ui::SESSION_COOKIE)
            .parse()
            .unwrap(),
    );
    let (ws, _resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    let (mut write, mut read) = ws.split();
    write.send(Message::Text("hello".into())).await.unwrap();

    let mut saw_terminal = false;
    while let Some(msg) = tokio::time::timeout(std::time::Duration::from_secs(30), read.next())
        .await
        .expect("timed out waiting for frames")
    {
        match msg.unwrap() {
            Message::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
                match v["type"].as_str() {
                    Some("done") | Some("error") => {
                        saw_terminal = true;
                        break;
                    }
                    _ => {}
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    assert!(saw_terminal, "expected a terminal event frame");
}
