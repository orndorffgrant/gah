use axum::response::{IntoResponse, Redirect, Response};

use crate::AuthSession;

pub async fn index(session: Option<AuthSession>) -> Response {
    match session {
        Some(s) if s.role == "admin" => Redirect::to("/admin").into_response(),
        Some(_) => Redirect::to("/sessions").into_response(),
        None => Redirect::to("/login").into_response(),
    }
}
