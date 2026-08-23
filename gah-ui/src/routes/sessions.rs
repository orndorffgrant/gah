use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use futures::{SinkExt, StreamExt};
use gah_api_client::types::{AgentConfig, CreateSessionRequest, ProviderKind};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::{protocol::Role, Message as ApiMessage};
use tokio_tungstenite::WebSocketStream;

use crate::templates::{ChatPage, NewSessionPage, SessionListPage};
use crate::{AuthSession, UiCtx};

pub async fn list(session: AuthSession, State(ctx): State<UiCtx>) -> Response {
    let (sessions, error) = match ctx.api.list_sessions().await {
        Ok(r) => (r.into_inner().sessions, None),
        Err(e) => (Vec::new(), Some(format!("Failed to list sessions: {e}"))),
    };
    SessionListPage {
        html_title: "Sessions".into(),
        username: Some(session.username),
        role: Some(session.role),
        sessions,
        error,
    }
    .into_response()
}

pub async fn new_page(session: AuthSession) -> Response {
    NewSessionPage {
        html_title: "New Session".into(),
        username: Some(session.username),
        role: Some(session.role),
        error: None,
        provider: "openrouter".into(),
        model: String::new(),
        api_base_url: String::new(),
        system_prompt: String::new(),
    }
    .into_response()
}

#[derive(Deserialize)]
pub struct NewSessionForm {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    pub api_base_url: Option<String>,
    pub system_prompt: Option<String>,
}

pub async fn create(
    session: AuthSession,
    State(ctx): State<UiCtx>,
    Form(form): Form<NewSessionForm>,
) -> Response {
    let provider = match form.provider.parse::<ProviderKind>() {
        Ok(p) => p,
        Err(_) => {
            return new_session_failed(&session, &form, "Unknown provider");
        }
    };

    if form.model.trim().is_empty() {
        return new_session_failed(&session, &form, "Model is required");
    }
    if form.api_key.trim().is_empty()
        && !matches!(form.provider.as_str(), "ollama" | "openrouter")
    {
        return new_session_failed(&session, &form, "API key is required");
    }

    let base_url = form
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let system_prompt = form
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let request = CreateSessionRequest {
        config: AgentConfig {
            provider,
            model: form.model.trim().to_string(),
            api_key: form.api_key.clone(),
            api_base_url: base_url,
            system_prompt,
        },
    };

    match ctx.api.create_session(&request).await {
        Ok(r) => Redirect::to(&format!("/sessions/{}", r.into_inner().id)).into_response(),
        Err(e) => new_session_failed(&session, &form, &format!("Failed to create session: {e}")),
    }
}

fn new_session_failed(session: &AuthSession, form: &NewSessionForm, msg: &str) -> Response {
    NewSessionPage {
        html_title: "New Session".into(),
        username: Some(session.username.clone()),
        role: Some(session.role.clone()),
        error: Some(msg.to_string()),
        provider: form.provider.clone(),
        model: form.model.clone(),
        api_base_url: form.api_base_url.clone().unwrap_or_default(),
        system_prompt: form.system_prompt.clone().unwrap_or_default(),
    }
    .into_response()
}

pub async fn chat_page(
    _session: AuthSession,
    State(ctx): State<UiCtx>,
    Path(id): Path<String>,
) -> Response {
    match ctx.api.get_session(&id).await {
        Ok(r) => {
            let s = r.into_inner();
            ChatPage {
                html_title: s.config.model.clone(),
                session_id: s.id,
                messages: s.messages,
                error: None,
            }
            .into_response()
        }
        Err(_) => Redirect::to("/sessions").into_response(),
    }
}

/// Websocket chat endpoint. The browser opens one connection per prompt: it
/// sends a single text frame, then receives JSON `AgentEvent` frames streamed
/// from the API until a terminal (`done`/`error`) frame, then the socket
/// closes. Per-prompt connections keep mobile reconnects cheap.
pub async fn ws(
    _session: AuthSession,
    State(ctx): State<UiCtx>,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |browser| chat_bridge(ctx, id, browser))
}

async fn chat_bridge(ctx: UiCtx, id: String, mut browser: WebSocket) {
    // One prompt per connection: the first text frame is the prompt.
    let prompt = match browser.recv().await {
        Some(Ok(Message::Text(t))) => t.to_string(),
        _ => return,
    };
    if prompt.trim().is_empty() {
        return;
    }

    // Dial the API's streaming endpoint with our bearer token.
    let upgraded = match ctx.api.session_stream(&id).await {
        Ok(r) => r.into_inner(),
        Err(e) => {
            send_frame(&mut browser, &error_frame(&format!("failed to start stream: {e}"))).await;
            let _ = browser.send(Message::Close(None)).await;
            return;
        }
    };
    let mut api = WebSocketStream::from_raw_socket(upgraded, Role::Client, None).await;

    if api.send(ApiMessage::Text(prompt.into())).await.is_err() {
        return;
    }

    // Pump API frames to the browser until the stream terminates; abort if
    // the browser goes away mid-stream.
    loop {
        tokio::select! {
            frame = api.next() => {
                let text = match frame {
                    Some(Ok(ApiMessage::Text(t))) => t.to_string(),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        send_frame(&mut browser, &error_frame(&format!("stream error: {e}"))).await;
                        break;
                    }
                    None => {
                        send_frame(&mut browser, &error_frame("stream ended unexpectedly")).await;
                        break;
                    }
                };
                let terminal = is_terminal(&text);
                send_frame(&mut browser, &text).await;
                if terminal {
                    break;
                }
            }
            closed = browser.recv() => {
                match closed {
                    None | Some(Ok(Message::Close(_))) => break,
                    _ => {}
                }
            }
        }
    }
    let _ = browser.send(Message::Close(None)).await;
}

async fn send_frame(socket: &mut WebSocket, text: &str) {
    let _ = socket.send(Message::Text(text.to_owned().into())).await;
}

/// A frame is terminal when it is the `done` or `error` event.
fn is_terminal(frame: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(frame)
        .ok()
        .and_then(|v| {
            v.get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == "done" || t == "error")
        })
        .unwrap_or(false)
}

fn error_frame(message: &str) -> String {
    serde_json::json!({ "type": "error", "message": message }).to_string()
}

/// htmx endpoint: deletes the session and removes the table row.
pub async fn delete(
    session: AuthSession,
    State(ctx): State<UiCtx>,
    Path(id): Path<String>,
) -> Response {
    let _ = session;
    match ctx.api.delete_session(&id).await {
        Ok(_) => axum::http::StatusCode::OK.into_response(),
        Err(_) => axum::http::StatusCode::BAD_GATEWAY.into_response(),
    }
}
