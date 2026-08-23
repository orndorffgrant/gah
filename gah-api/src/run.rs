//! Server implementation for the API trait (behind the `run` feature).

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use dropshot::{
    ApiDescription, ClientErrorStatusCode, ConfigLogging, ConfigLoggingLevel, HttpError,
    HttpResponseCreated, HttpResponseOk, HttpResponseUpdatedNoContent, Path, RequestContext,
    ServerBuilder, TypedBody, WebsocketConnection,
};
use futures::{SinkExt, StreamExt};
use gah_core::{AgentEvent, Session};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio_tungstenite::tungstenite::{protocol::Role, Message as WsMessage};

use crate::{
    auth, ApiCtx, ApiState, CreateSessionRequest, CreateTokenRequest, CreateTokenResponse,
    GahApi, SendPromptRequest, SendPromptResponse, SessionListResponse, SessionPath,
    SessionResponse, SqliteSessionStore, SqliteTokenStore,
};

/// Placeholder type carrying the implementation. Never constructed.
pub(crate) enum ServerImpl {}

fn session_response(s: &Session) -> SessionResponse {
    SessionResponse {
        id: s.id.clone(),
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
        message_count: s.messages.len(),
        config: s.config.redacted(),
        messages: s.messages.clone(),
    }
}

fn not_found() -> HttpError {
    HttpError::for_client_error_with_status(None, ClientErrorStatusCode::NOT_FOUND)
}

fn internal(msg: String) -> HttpError {
    HttpError::for_internal_error(msg)
}

impl GahApi for ServerImpl {
    type Context = ApiCtx;

    async fn create_session(
        rqctx: RequestContext<Self::Context>,
        body: TypedBody<CreateSessionRequest>,
    ) -> Result<HttpResponseCreated<SessionResponse>, HttpError> {
        auth::check(&rqctx).await?;
        let config = body.into_inner().config;
        let session = Session::new(config);
        let response = session_response(&session);
        rqctx.context()
            .session_store
            .create(&session)
            .await
            .map_err(|e| internal(format!("failed to create session: {e}")))?;
        Ok(HttpResponseCreated(response))
    }

    async fn list_sessions(
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<SessionListResponse>, HttpError> {
        auth::check(&rqctx).await?;
        let sessions = rqctx
            .context()
            .session_store
            .list()
            .await
            .map_err(|e| internal(format!("failed to list sessions: {e}")))?;
        let list = sessions.iter().map(session_response).collect();
        Ok(HttpResponseOk(SessionListResponse { sessions: list }))
    }

    async fn get_session(
        rqctx: RequestContext<Self::Context>,
        path: Path<SessionPath>,
    ) -> Result<HttpResponseOk<SessionResponse>, HttpError> {
        auth::check(&rqctx).await?;
        let session = rqctx
            .context()
            .session_store
            .get(&path.into_inner().id)
            .await
            .map_err(|e| internal(format!("failed to get session: {e}")))?
            .ok_or_else(not_found)?;
        Ok(HttpResponseOk(session_response(&session)))
    }

    async fn delete_session(
        rqctx: RequestContext<Self::Context>,
        path: Path<SessionPath>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        auth::check(&rqctx).await?;
        let deleted = rqctx
            .context()
            .session_store
            .delete(&path.into_inner().id)
            .await
            .map_err(|e| internal(format!("failed to delete session: {e}")))?;
        if !deleted {
            return Err(not_found());
        }
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn send_prompt(
        rqctx: RequestContext<Self::Context>,
        path: Path<SessionPath>,
        body: TypedBody<SendPromptRequest>,
    ) -> Result<HttpResponseOk<SendPromptResponse>, HttpError> {
        auth::check(&rqctx).await?;
        let id = path.into_inner().id;
        let prompt = body.into_inner().prompt;
        let state = rqctx.context();

        let session = state
            .session_store
            .get(&id)
            .await
            .map_err(|e| internal(format!("failed to get session: {e}")))?
            .ok_or_else(not_found)?;

        let outcome = gah_agent::run_blocking(&session.config, &prompt, &session.messages)
            .await
            .map_err(|e| internal(format!("agent run failed: {e}")))?;

        let mut messages = session.messages.clone();
        messages.extend(outcome.messages);
        state
            .session_store
            .update_messages(&id, &messages)
            .await
            .map_err(|e| internal(format!("failed to persist session: {e}")))?;

        Ok(HttpResponseOk(SendPromptResponse {
            output: outcome.output,
            usage: outcome.usage,
            message_count: messages.len(),
        }))
    }

    async fn session_stream(
        rqctx: RequestContext<Self::Context>,
        path: Path<SessionPath>,
        upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        auth::check(&rqctx).await?;
        let id = path.into_inner().id;
        let state = rqctx.context();

        let session = state
            .session_store
            .get(&id)
            .await?
            .ok_or_else(|| {
                HttpError::for_client_error_with_status(
                    None,
                    ClientErrorStatusCode::NOT_FOUND,
                )
            })?;

        let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            upgraded.into_inner(),
            Role::Server,
            None,
        )
        .await;

        let prompt = match ws.next().await {
            Some(Ok(WsMessage::Text(text))) => text.to_string(),
            _ => return Ok(()),
        };

        let mut events =
            match gah_agent::run_stream(&session.config, &prompt, &session.messages).await {
                Ok(events) => events,
                Err(e) => {
                    let frame = serde_json::json!({
                        "type": "error",
                        "message": format!("agent error: {e}"),
                    })
                    .to_string();
                    let _ = ws.send(WsMessage::Text(frame.into())).await;
                    return Ok(());
                }
            };

        while let Some(event) = events.next().await {
            let finished = matches!(
                event,
                AgentEvent::Done { .. } | AgentEvent::Error { .. }
            );
            let new_messages = match &event {
                AgentEvent::Done { messages, .. } => Some(messages.clone()),
                _ => None,
            };

            let frame = serde_json::to_string(&event).unwrap_or_else(|_| {
                r#"{"type":"error","message":"event serialization failed"}"#.to_string()
            });
            if ws.send(WsMessage::Text(frame.into())).await.is_err() {
                break;
            }

            if let Some(new_messages) = new_messages {
                let mut messages = session.messages.clone();
                messages.extend(new_messages);
                state.session_store.update_messages(&id, &messages).await?;
            }

            if finished {
                break;
            }
        }

        Ok(())
    }

    async fn create_token(
        rqctx: RequestContext<Self::Context>,
        body: TypedBody<CreateTokenRequest>,
    ) -> Result<HttpResponseCreated<CreateTokenResponse>, HttpError> {
        auth::check(&rqctx).await?;
        let body = body.into_inner();
        let token = rqctx
            .context()
            .token_store
            .create_token(&body.label)
            .await
            .map_err(|e| internal(format!("failed to create token: {e}")))?;
        Ok(HttpResponseCreated(token))
    }
}

/// Accept either a bare path or a full sqlite:// URL.
pub fn sqlite_url(db: &str) -> String {
    if db.starts_with("sqlite:") {
        db.to_string()
    } else {
        format!("sqlite://{db}")
    }
}

/// Connect options that create the database file when missing.
pub fn sqlite_options(db: &str) -> Result<SqliteConnectOptions, sqlx::Error> {
    Ok(SqliteConnectOptions::from_str(&sqlite_url(db))?.create_if_missing(true))
}

/// Build the API state against a database pool.
pub async fn build_state(
    db_url: &str,
) -> Result<ApiCtx, Box<dyn std::error::Error + Send + Sync>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(sqlite_options(db_url)?)
        .await?;

    let token_store = SqliteTokenStore::new(pool.clone());
    token_store.create_table().await?;

    let session_store = SqliteSessionStore::new(pool);
    session_store.create_tables().await?;

    Ok(Arc::new(ApiState {
        token_store,
        session_store,
    }))
}

/// Start the API server on the given address; returns the running server.
pub async fn start(
    bind_addr: SocketAddr,
    state: ApiCtx,
) -> Result<dropshot::HttpServer<ApiCtx>, Box<dyn std::error::Error + Send + Sync>> {
    let log = ConfigLogging::StderrTerminal {
        level: ConfigLoggingLevel::Error,
    }
    .to_logger("gah-api")?;

    let api: ApiDescription<ApiCtx> =
        crate::gah_api_mod::api_description::<ServerImpl>().unwrap();
    let server = ServerBuilder::new(api, state, log)
        .config(dropshot::ConfigDropshot {
            bind_address: bind_addr,
            default_request_body_max_bytes: 1024 * 1024,
            ..Default::default()
        })
        .start()
        .map_err(|e| format!("failed to start server: {e}"))?;

    tracing::info!("api listening on {bind_addr}");
    Ok(server)
}

pub async fn run(
    bind_addr: SocketAddr,
    db_url: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = build_state(db_url).await?;
    let server = start(bind_addr, state).await?;
    server.await.map_err(|e| e.into())
}
