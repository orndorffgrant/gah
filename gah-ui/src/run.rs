use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use crate::session::SessionStore;
use crate::UiCtx;

/// Accept either a bare path or a full sqlite:// URL.
fn sqlite_url(db: &str) -> String {
    if db.starts_with("sqlite:") {
        db.to_string()
    } else {
        format!("sqlite://{db}")
    }
}

fn sqlite_options(db: &str) -> Result<SqliteConnectOptions, sqlx::Error> {
    Ok(SqliteConnectOptions::from_str(&sqlite_url(db))?.create_if_missing(true))
}

pub async fn run(
    bind_addr: SocketAddr,
    db_url: &str,
    _session_secret: &str,
    api_url: &str,
    api_token: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(sqlite_options(db_url)?)
        .await?;

    let store = SessionStore::new(pool.clone());
    store.create_tables().await?;

    bootstrap_admin(&store).await?;

    let cleanup_store = store.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let _ = cleanup_store.cleanup_expired().await;
        }
    });

    let api = build_api_client(api_url, api_token)?;

    let state: UiCtx = Arc::new(crate::UiState {
        db: pool,
        session_store: store,
        api,
    });

    let app = crate::router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!("ui listening on {bind_addr} (api at {api_url})");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn build_api_client(
    api_url: &str,
    token: &str,
) -> Result<gah_api_client::Client, Box<dyn std::error::Error + Send + Sync>> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {token}").parse()?,
    );
    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;
    Ok(gah_api_client::Client::new_with_client(api_url, http))
}

async fn bootstrap_admin(store: &SessionStore) -> Result<(), sqlx::Error> {
    let count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE role = 'admin'")
            .fetch_one(&store.db)
            .await?;

    if count == 0 {
        use rand::Rng;
        let password: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(14)
            .map(char::from)
            .collect();

        store.create_user("admin", &password, "admin").await?;

        eprintln!("=== Bootstrapped admin user ===");
        eprintln!("Username: admin");
        eprintln!("Password: {password}");
        eprintln!("===============================");
    }

    Ok(())
}
