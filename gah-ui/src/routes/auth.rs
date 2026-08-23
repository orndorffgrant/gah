use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use crate::templates::{AdminPage, LoginPage, SettingsPage, UserView};
use crate::{AuthSession, UiCtx, SESSION_COOKIE};

#[derive(Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

pub async fn login_page() -> Response {
    LoginPage {
        html_title: "Login".into(),
        username: None,
        role: None,
        error: None,
    }
    .into_response()
}

pub async fn login(State(ctx): State<UiCtx>, Form(form): Form<LoginForm>) -> Response {
    if form.username.is_empty() || form.password.is_empty() {
        return login_failed("Username and password are required");
    }

    match ctx.session_store.verify_login(&form.username, &form.password).await {
        Ok(Some((uid, role))) => {
            let sid = ctx
                .session_store
                .create_session(uid, &form.username, &role)
                .await
                .unwrap_or_default();
            let target = if role == "admin" { "/admin" } else { "/sessions" };
            Response::builder()
                .status(302)
                .header(
                    "set-cookie",
                    format!("{SESSION_COOKIE}={sid}; Path=/; HttpOnly; SameSite=Lax"),
                )
                .header("location", target)
                .body(axum::body::Body::empty())
                .unwrap()
        }
        Ok(None) => login_failed("Invalid credentials"),
        Err(e) => login_failed(&format!("Login failed: {e}")),
    }
}

fn login_failed(msg: &str) -> Response {
    LoginPage {
        html_title: "Login".into(),
        username: None,
        role: None,
        error: Some(msg.to_string()),
    }
    .into_response()
}

pub async fn logout(State(ctx): State<UiCtx>, session: Option<AuthSession>) -> Response {
    if let Some(s) = session {
        let _ = ctx.session_store.delete_session(&s.session_id).await;
    }
    Response::builder()
        .status(302)
        .header("set-cookie", format!("{SESSION_COOKIE}=; Path=/; Max-Age=0"))
        .header("location", "/login")
        .body(axum::body::Body::empty())
        .unwrap()
}

pub async fn settings_page(session: AuthSession) -> Response {
    SettingsPage {
        html_title: "Settings".into(),
        username: Some(session.username),
        role: Some(session.role),
        saved: false,
        error: None,
    }
    .into_response()
}

#[derive(Deserialize)]
pub struct ChangePasswordForm {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(ctx): State<UiCtx>,
    session: AuthSession,
    Form(form): Form<ChangePasswordForm>,
) -> Response {
    let (saved, error) = match ctx
        .session_store
        .change_password(session.user_id, &form.current_password, &form.new_password)
        .await
    {
        Ok(true) => (true, None),
        Ok(false) => (false, Some("Current password is incorrect".into())),
        Err(e) => (false, Some(format!("Failed to change password: {e}"))),
    };
    SettingsPage {
        html_title: "Settings".into(),
        username: Some(session.username),
        role: Some(session.role),
        saved,
        error,
    }
    .into_response()
}

pub async fn admin_page(session: AuthSession, State(ctx): State<UiCtx>) -> Response {
    if session.role != "admin" {
        return Redirect::to("/sessions").into_response();
    }
    let users = ctx
        .session_store
        .list_users()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(id, username, role, created_at)| UserView {
            id,
            username,
            role,
            created_at,
        })
        .collect();
    AdminPage {
        html_title: "Admin".into(),
        username: Some(session.username),
        role: Some(session.role),
        users,
        error: None,
    }
    .into_response()
}

#[derive(Deserialize)]
pub struct CreateUserForm {
    pub username: String,
    pub password: String,
    pub role: String,
}

pub async fn create_user(
    State(ctx): State<UiCtx>,
    session: AuthSession,
    Form(form): Form<CreateUserForm>,
) -> Response {
    if session.role != "admin" {
        return Redirect::to("/sessions").into_response();
    }
    if !form.username.is_empty() && form.password.len() >= 8 {
        if let Err(e) = ctx
            .session_store
            .create_user(&form.username, &form.password, &form.role)
            .await
        {
            tracing::error!("failed to create user: {e}");
        }
    }
    Redirect::to("/admin").into_response()
}

pub async fn delete_user(
    State(ctx): State<UiCtx>,
    session: AuthSession,
    Path(id): Path<i64>,
) -> Response {
    if session.role != "admin" {
        return Redirect::to("/sessions").into_response();
    }
    if id != session.user_id {
        if let Err(e) = ctx.session_store.delete_user(id).await {
            tracing::error!("failed to delete user: {e}");
        }
    }
    Redirect::to("/admin").into_response()
}
