//! gah-api: the HTTP API interface for Grant's Agent Harness.
//!
//! This module defines the API surface (endpoints and types) via a Dropshot
//! trait, plus [`openapi()`] for generating the OpenAPI spec. The server
//! implementation lives in [`run`] behind the `run` feature, so client crates
//! can generate from the spec without compiling the server stack
//! (mirrors boot/boot-api).

use dropshot::{
    HttpError, HttpResponseCreated, HttpResponseOk, HttpResponseUpdatedNoContent, Path,
    RequestContext, TypedBody, WebsocketChannelResult, WebsocketConnection,
};
use gah_core::{AgentConfig, ChatMessage, UsageInfo};
use serde::{Deserialize, Serialize};

#[dropshot::api_description]
pub trait GahApi {
    /// The context type used within endpoints.
    type Context;

    /// Create a new agent session.
    #[endpoint {
        method = POST,
        path = "/v1/sessions",
    }]
    async fn create_session(
        rqctx: RequestContext<Self::Context>,
        body: TypedBody<CreateSessionRequest>,
    ) -> Result<HttpResponseCreated<SessionResponse>, HttpError>;

    /// List all sessions.
    #[endpoint {
        method = GET,
        path = "/v1/sessions",
    }]
    async fn list_sessions(
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<SessionListResponse>, HttpError>;

    /// Get a session by id.
    #[endpoint {
        method = GET,
        path = "/v1/sessions/{id}",
    }]
    async fn get_session(
        rqctx: RequestContext<Self::Context>,
        path: Path<SessionPath>,
    ) -> Result<HttpResponseOk<SessionResponse>, HttpError>;

    /// Delete a session by id.
    #[endpoint {
        method = DELETE,
        path = "/v1/sessions/{id}",
    }]
    async fn delete_session(
        rqctx: RequestContext<Self::Context>,
        path: Path<SessionPath>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    /// Send a prompt to a session and wait for the agent's reply.
    #[endpoint {
        method = POST,
        path = "/v1/sessions/{id}/prompt",
    }]
    async fn send_prompt(
        rqctx: RequestContext<Self::Context>,
        path: Path<SessionPath>,
        body: TypedBody<SendPromptRequest>,
    ) -> Result<HttpResponseOk<SendPromptResponse>, HttpError>;

    /// Stream a prompt over a websocket. Send one text frame with the prompt;
    /// receive a stream of JSON-encoded AgentEvent frames, ending with a
    /// done (or error) event.
    #[channel {
        protocol = WEBSOCKETS,
        path = "/v1/sessions/{id}/stream",
    }]
    async fn session_stream(
        rqctx: RequestContext<Self::Context>,
        path: Path<SessionPath>,
        upgraded: WebsocketConnection,
    ) -> WebsocketChannelResult;

    /// Create a new API bearer token.
    #[endpoint {
        method = POST,
        path = "/v1/tokens",
    }]
    async fn create_token(
        rqctx: RequestContext<Self::Context>,
        body: TypedBody<CreateTokenRequest>,
    ) -> Result<HttpResponseCreated<CreateTokenResponse>, HttpError>;
}

/// Generate the OpenAPI spec for this API from the interface alone, without
/// compiling (or even having) an implementation.
pub fn openapi() -> String {
    let description = gah_api_mod::stub_api_description().unwrap();
    let spec = description.openapi("gah-api", semver::Version::new(0, 1, 0));
    let json = spec.json().unwrap();
    serde_json::to_string(&json).unwrap()
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateSessionRequest {
    pub config: AgentConfig,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionResponse {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub config: AgentConfig,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionResponse>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SendPromptRequest {
    pub prompt: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SendPromptResponse {
    pub output: String,
    pub usage: UsageInfo,
    pub message_count: usize,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateTokenRequest {
    pub label: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateTokenResponse {
    pub token: String,
    pub label: String,
    pub created_at: String,
}

#[derive(Deserialize, Debug, schemars::JsonSchema)]
pub struct SessionPath {
    pub id: String,
}

#[cfg(feature = "run")]
mod auth;
#[cfg(feature = "run")]
pub mod run;
#[cfg(feature = "run")]
mod session_store;
#[cfg(feature = "run")]
mod token_store;

#[cfg(feature = "run")]
pub use run::run;
#[cfg(feature = "run")]
pub use session_store::SqliteSessionStore;
#[cfg(feature = "run")]
pub use token_store::SqliteTokenStore;

#[cfg(feature = "run")]
pub type ApiCtx = std::sync::Arc<ApiState>;

#[cfg(feature = "run")]
pub struct ApiState {
    pub token_store: SqliteTokenStore,
    pub session_store: SqliteSessionStore,
}
