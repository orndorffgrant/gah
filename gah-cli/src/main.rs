use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;

mod systemd_service;
mod update;

#[derive(Parser)]
#[command(name = "gah", about = "Grant's Agent Harness")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args)]
pub struct StartApiArgs {
    /// Address for the service to listen on (defaults to 127.0.0.1:46674)
    #[arg(long)]
    pub bind: Option<SocketAddr>,
    /// Path to the SQLite database (defaults to gah.db)
    #[arg(long)]
    pub db: Option<String>,
}

#[derive(Args)]
pub struct StartWebuiArgs {
    /// Address for the service to listen on (defaults to 127.0.0.1:3000)
    #[arg(long)]
    pub bind: Option<SocketAddr>,
    /// Path to the SQLite database (defaults to gah-ui.db)
    #[arg(long)]
    pub db: Option<String>,
    /// URL of the API server (defaults to http://127.0.0.1:46674)
    #[arg(long)]
    pub api_url: Option<String>,
    #[arg(long, env = "GAH_SESSION_SECRET")]
    pub session_secret: Option<String>,
    #[arg(long, env = "GAH_API_TOKEN")]
    pub api_token: Option<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the API server as a systemd service
    StartApi(StartApiArgs),
    /// Stop the API systemd service
    StopApi,
    /// Restart the API systemd service
    RestartApi,
    /// Run the API server in the foreground
    RunApi {
        #[arg(long, default_value = "127.0.0.1:46674")]
        bind: SocketAddr,
        #[arg(long, default_value = "gah.db")]
        db: String,
    },
    /// Start the web UI as a systemd service
    StartWebui(StartWebuiArgs),
    /// Stop the web UI systemd service
    StopWebui,
    /// Restart the web UI systemd service
    RestartWebui,
    /// Run the web UI in the foreground
    RunWebui {
        #[arg(long, default_value = "127.0.0.1:3000")]
        bind: SocketAddr,
        #[arg(long, default_value = "gah-ui.db")]
        db: String,
        #[arg(long, env = "GAH_SESSION_SECRET")]
        session_secret: String,
        #[arg(long, default_value = "http://127.0.0.1:46674")]
        api_url: String,
        #[arg(long, env = "GAH_API_TOKEN")]
        api_token: String,
    },
    /// Create an API bearer token
    CreateToken {
        #[arg(long, default_value = "gah.db")]
        db: String,
        #[arg(long)]
        label: String,
    },
    /// Export OpenAPI spec
    Openapi,
    /// Print version
    Version,
    /// Check for a newer release and apply it
    Update {
        /// Only report whether an update is available; don't apply it
        #[arg(long)]
        check: bool,
        /// GitHub repo to fetch releases from (owner/name)
        #[arg(long)]
        repo: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::StartApi(args) => {
            let mut service_args = vec!["run-api".to_string()];
            if let Some(bind) = args.bind {
                service_args.push("--bind".to_string());
                service_args.push(bind.to_string());
            }
            if let Some(db) = args.db {
                service_args.push("--db".to_string());
                service_args.push(db);
            }
            systemd_service::start_service("gah-api", "GAH API Service", &service_args, &[])?;
            println!("API service started (gah-api)");
        }
        Command::StopApi => systemd_service::stop_service("gah-api")?,
        Command::RestartApi => {
            systemd_service::restart_service("gah-api")?;
            println!("API service restarted");
        }
        Command::RunApi { bind, db } => {
            gah_api::run::run(bind, &db).await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Command::StartWebui(args) => {
            let session_secret = args.session_secret.clone().unwrap_or_default();
            let api_token = args.api_token.clone().unwrap_or_default();
            if session_secret.is_empty() {
                anyhow::bail!("GAH_SESSION_SECRET is required");
            }
            if api_token.is_empty() {
                anyhow::bail!("GAH_API_TOKEN is required (create one with `gah create-token`)");
            }
            let mut service_args = vec!["run-webui".to_string()];
            if let Some(bind) = args.bind {
                service_args.push("--bind".to_string());
                service_args.push(bind.to_string());
            }
            if let Some(db) = args.db {
                service_args.push("--db".to_string());
                service_args.push(db);
            }
            if let Some(api_url) = args.api_url {
                service_args.push("--api-url".to_string());
                service_args.push(api_url);
            }
            let env_vars = vec![
                format!("GAH_SESSION_SECRET={session_secret}"),
                format!("GAH_API_TOKEN={api_token}"),
            ];
            systemd_service::start_service(
                "gah-webui",
                "GAH Web UI Service",
                &service_args,
                &env_vars,
            )?;
            println!("Web UI service started (gah-webui)");
        }
        Command::StopWebui => systemd_service::stop_service("gah-webui")?,
        Command::RestartWebui => {
            systemd_service::restart_service("gah-webui")?;
            println!("Web UI service restarted");
        }
        Command::RunWebui {
            bind,
            db,
            session_secret,
            api_url,
            api_token,
        } => {
            if session_secret.is_empty() {
                anyhow::bail!("GAH_SESSION_SECRET is required");
            }
            if api_token.is_empty() {
                anyhow::bail!("GAH_API_TOKEN is required (create one with `gah create-token`)");
            }
            gah_ui::run::run(bind, &db, &session_secret, &api_url, &api_token)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Command::CreateToken { db, label } => {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(gah_api::run::sqlite_options(&db)?)
                .await?;
            let store = gah_api::SqliteTokenStore::new(pool);
            store.create_table().await?;
            let token = store.create_token(&label).await?;
            println!("Token: {}", token.token);
            println!("Label: {}", token.label);
            println!("Created: {}", token.created_at);
        }
        Command::Openapi => {
            let spec: serde_json::Value = serde_json::from_str(&gah_api::openapi())?;
            println!("{}", serde_json::to_string_pretty(&spec)?);
        }
        Command::Version => {
            println!("gah {}", env!("CARGO_PKG_VERSION"));
        }
        Command::Update { check, repo } => {
            update::run(check, repo.as_deref()).await?;
        }
    }

    Ok(())
}