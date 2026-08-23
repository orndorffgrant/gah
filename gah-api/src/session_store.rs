use sqlx::SqlitePool;
use gah_core::{ChatMessage, Session};

#[derive(Clone)]
pub struct SqliteSessionStore {
    pool: SqlitePool,
}

impl SqliteSessionStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_tables(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS agent_sessions (
                id TEXT PRIMARY KEY,
                config TEXT NOT NULL,
                messages TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create(&self, session: &Session) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO agent_sessions (id, config, messages, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&session.id)
        .bind(serde_json::to_string(&session.config).unwrap_or_default())
        .bind("[]")
        .bind(session.created_at.to_rfc3339())
        .bind(session.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<Session>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT id, config, messages, created_at, updated_at FROM agent_sessions
             ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|(id, config, messages, created_at, updated_at)| {
                Some(Session {
                    id,
                    config: serde_json::from_str(&config).ok()?,
                    messages: serde_json::from_str(&messages).ok()?,
                    created_at: parse_ts(&created_at)?,
                    updated_at: parse_ts(&updated_at)?,
                })
            })
            .collect())
    }

    pub async fn get(&self, id: &str) -> Result<Option<Session>, sqlx::Error> {
        let row = sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT id, config, messages, created_at, updated_at FROM agent_sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|(id, config, messages, created_at, updated_at)| {
            Some(Session {
                id,
                config: serde_json::from_str(&config).ok()?,
                messages: serde_json::from_str(&messages).unwrap_or_default(),
                created_at: parse_ts(&created_at)?,
                updated_at: parse_ts(&updated_at)?,
            })
        }))
    }

    pub async fn delete(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM agent_sessions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update_messages(
        &self,
        id: &str,
        messages: &[ChatMessage],
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE agent_sessions SET messages = ?, updated_at = ? WHERE id = ?",
        )
        .bind(serde_json::to_string(messages).unwrap_or_else(|_| "[]".into()))
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn parse_ts(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}
