pub mod auth;
pub mod home;
pub mod sessions;

use askama::Template;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::templates::NotFoundPage;

pub async fn not_found() -> Response {
    let page = NotFoundPage {
        html_title: "Not Found".into(),
        username: None,
        role: None,
    };
    let body = page.render().unwrap_or_else(|_| "Not Found".into());
    (StatusCode::NOT_FOUND, axum::response::Html(body)).into_response()
}
