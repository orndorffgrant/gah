use axum::{
    extract::{FromRequestParts, OptionalFromRequestParts},
    http::request::Parts,
    response::Redirect,
};
use sqlx::SqlitePool;

use crate::UiCtx;

#[derive(Clone)]
pub struct SessionStore {
    pub db: SqlitePool,
}

#[derive(Clone, Debug)]
pub struct AuthSession {
    pub session_id: String,
    pub user_id: i64,
    pub username: String,
    pub role: String,
}

impl SessionStore {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    pub async fn create_tables(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                salt TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'creator',
                created_at TEXT NOT NULL
            )",
        )
        .execute(&self.db)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS auth_sessions (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                username TEXT NOT NULL,
                role TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                FOREIGN KEY (user_id) REFERENCES users(id)
            )",
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn create_session(
        &self,
        user_id: i64,
        username: &str,
        role: &str,
    ) -> Result<String, sqlx::Error> {
        use rand::Rng;
        let id: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(64)
            .map(char::from)
            .collect();
        let now = chrono::Utc::now();
        let exp = now + chrono::Duration::hours(24);
        sqlx::query(
            "INSERT INTO auth_sessions (id, user_id, username, role, created_at, expires_at)
             VALUES (?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(username)
        .bind(role)
        .bind(now.to_rfc3339())
        .bind(exp.to_rfc3339())
        .execute(&self.db)
        .await?;
        Ok(id)
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<AuthSession>, sqlx::Error> {
        let row = sqlx::query_as::<_, (i64, String, String, String)>(
            "SELECT user_id, username, role, expires_at FROM auth_sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        if let Some((uid, uname, role, exp)) = row {
            let exp: chrono::DateTime<chrono::Utc> = chrono::DateTime::parse_from_rfc3339(&exp)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or(chrono::Utc::now());
            if chrono::Utc::now() < exp {
                return Ok(Some(AuthSession {
                    session_id: id.to_string(),
                    user_id: uid,
                    username: uname,
                    role,
                }));
            }
            sqlx::query("DELETE FROM auth_sessions WHERE id = ?")
                .bind(id)
                .execute(&self.db)
                .await?;
        }
        Ok(None)
    }

    pub async fn delete_session(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM auth_sessions WHERE id = ?")
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn cleanup_expired(&self) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM auth_sessions WHERE expires_at < ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn verify_login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<(i64, String)>, sqlx::Error> {
        let row = sqlx::query_as::<_, (i64, String, String, String)>(
            "SELECT id, password_hash, salt, role FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.db)
        .await?;

        Ok(row.and_then(|(uid, hash, salt, role)| {
            (hash_password(password, &salt) == hash).then_some((uid, role))
        }))
    }

    pub async fn change_password(
        &self,
        user_id: i64,
        current: &str,
        new: &str,
    ) -> Result<bool, sqlx::Error> {
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT password_hash, salt FROM users WHERE id = ?",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await?;

        if let Some((hash, salt)) = row {
            if hash_password(current, &salt) != hash {
                return Ok(false);
            }
            let new_salt = generate_salt();
            sqlx::query("UPDATE users SET password_hash = ?, salt = ? WHERE id = ?")
                .bind(hash_password(new, &new_salt))
                .bind(&new_salt)
                .bind(user_id)
                .execute(&self.db)
                .await?;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn list_users(&self) -> Result<Vec<(i64, String, String, String)>, sqlx::Error> {
        sqlx::query_as::<_, (i64, String, String, String)>(
            "SELECT id, username, role, created_at FROM users ORDER BY username",
        )
        .fetch_all(&self.db)
        .await
    }

    pub async fn create_user(
        &self,
        username: &str,
        password: &str,
        role: &str,
    ) -> Result<(), sqlx::Error> {
        let salt = generate_salt();
        sqlx::query(
            "INSERT INTO users (username, password_hash, salt, role, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(username)
        .bind(hash_password(password, &salt))
        .bind(&salt)
        .bind(role)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_user(&self, user_id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(user_id)
            .execute(&self.db)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

pub const SESSION_COOKIE: &str = "gah.session.id";

async fn auth_session_from_parts(parts: &Parts, state: &UiCtx) -> Option<AuthSession> {
    let id = cookie_value(parts, SESSION_COOKIE)?;
    state.session_store.get_session(&id).await.ok().flatten()
}

/// Extract the authenticated user from the session cookie, or redirect to
/// the login page.
impl FromRequestParts<UiCtx> for AuthSession {
    type Rejection = Redirect;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &UiCtx,
    ) -> Result<Self, Self::Rejection> {
        auth_session_from_parts(parts, state)
            .await
            .ok_or(Redirect::to("/login"))
    }
}

/// `Option<AuthSession>` in a handler: `None` when not logged in.
impl OptionalFromRequestParts<UiCtx> for AuthSession {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &UiCtx,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(auth_session_from_parts(parts, state).await)
    }
}

fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    for value in parts.headers.get_all(axum::http::header::COOKIE) {
        let value = value.to_str().ok()?;
        for pair in value.split(';') {
            let pair = pair.trim();
            if let Some((k, v)) = pair.split_once('=') {
                if k == name {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

pub fn hash_password(pw: &str, salt: &str) -> String {
    use sha2::{Digest, Sha512};
    format!(
        "{:x}",
        Sha512::new().chain_update(format!("{pw}-{salt}")).finalize()
    )
}

pub fn generate_salt() -> String {
    use rand::Rng;
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}


