use axum::{http, routing::get, Router};
use sqlx::SqlitePool;
use std::sync::Arc;

pub mod run;
mod session;
mod templates;
mod routes;

pub use session::{AuthSession, SessionStore, SESSION_COOKIE};

pub struct UiState {
    pub db: SqlitePool,
    pub session_store: SessionStore,
    pub api: gah_api_client::Client,
}

pub type UiCtx = Arc<UiState>;

pub fn router(state: UiCtx) -> Router {
    Router::new()
        .route("/", get(routes::home::index))
        .route("/login", get(routes::auth::login_page).post(routes::auth::login))
        .route("/logout", get(routes::auth::logout).post(routes::auth::logout))
        .route("/sessions", get(routes::sessions::list))
        .route("/sessions/new", get(routes::sessions::new_page).post(routes::sessions::create))
        .route("/sessions/{id}", get(routes::sessions::chat_page))
        .route("/sessions/{id}/ws", get(routes::sessions::ws))
        .route("/sessions/{id}/delete", axum::routing::post(routes::sessions::delete))
        .route("/settings", get(routes::auth::settings_page).post(routes::auth::change_password))
        .route("/admin", get(routes::auth::admin_page))
        .route("/admin/users", axum::routing::post(routes::auth::create_user))
        .route("/admin/users/{id}/delete", axum::routing::post(routes::auth::delete_user))
        .route("/assets/htmx.js", get(serve_htmx))
        .route("/assets/styles.css", get(serve_styles))
        .route("/assets/chat.js", get(serve_chat_js))
        .fallback(routes::not_found)
        .with_state(state)
}

async fn serve_htmx() -> impl axum::response::IntoResponse {
    (
        [(http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("../assets/htmx.js"),
    )
}

async fn serve_styles() -> impl axum::response::IntoResponse {
    (
        [(http::header::CONTENT_TYPE, "text/css")],
        include_str!("../assets/styles.css"),
    )
}

async fn serve_chat_js() -> impl axum::response::IntoResponse {
    (
        [(http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("../assets/chat.js"),
    )
}
