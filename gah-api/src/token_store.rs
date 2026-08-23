use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::hash_token;
use crate::CreateTokenResponse;

#[derive(Clone)]
pub struct SqliteTokenStore {
    pool: SqlitePool,
}

impl SqliteTokenStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_table(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS api_tokens (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                token_hash TEXT NOT NULL UNIQUE,
                label TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_token(
        &self,
        label: &str,
    ) -> Result<CreateTokenResponse, sqlx::Error> {
        let token = Uuid::new_v4().to_string();
        let hash = hash_token(&token);
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO api_tokens (token_hash, label, created_at) VALUES (?, ?, ?)",
        )
        .bind(&hash)
        .bind(label)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(CreateTokenResponse {
            token,
            label: label.to_string(),
            created_at: now,
        })
    }

    pub async fn validate(&self, token: &str) -> Result<bool, sqlx::Error> {
        let hash = hash_token(token);
        let row = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM api_tokens WHERE token_hash = ?",
        )
        .bind(&hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(row > 0)
    }
}