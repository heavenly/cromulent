use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing_subscriber::EnvFilter;

mod agent;
mod app;
mod auth;
mod process;
mod protocol;
mod providers;
mod session;
mod tools;
mod transport;
mod util;

/// Headless coding agent daemon — JSONL protocol over stdin/stdout.
#[derive(Parser)]
#[command(name = "cromulent", about = "Headless coding agent daemon", version)]
struct Cli {
    /// Provider to use (overrides config default)
    #[arg(long)]
    provider: Option<String>,

    /// Model ID to use (overrides config default)
    #[arg(long)]
    model: Option<String>,

    /// Thinking level (low, medium, high)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(["low", "medium", "high"]))]
    thinking: Option<String>,

    /// Session ID to load on startup. If omitted a new session is created.
    #[arg(long)]
    session: Option<String>,

    /// Working directory. Defaults to current directory.
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Maximum turns per agent run.
    #[arg(long)]
    max_turns: Option<u32>,

    /// Directory for session persistence. Defaults to XDG data dir.
    #[arg(long)]
    sessions_dir: Option<PathBuf>,

    /// Config file path. Defaults to ~/.cromulent/config.json.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Run codex auth setup and exit.
    #[arg(long)]
    setup_codex: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize tracing to stderr
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // --setup-codex: seed a local credential cache from env vars and exit.
    if cli.setup_codex {
        if let Err(e) = setup_codex().await {
            eprintln!("Codex auth setup failed: {e}");
            std::process::exit(1);
        }
        return;
    }

    // ------------------------------------------------------------------
    // Resolve directories and config
    // ------------------------------------------------------------------

    let cwd = cli
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let sessions_dir = cli
        .sessions_dir
        .clone()
        .unwrap_or_else(util::fs::default_sessions_dir);

    let config_file = match &cli.config {
        Some(path) => auth::config::load_config(path).await,
        None => auth::config::load_default_config().await,
    }
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "Failed to load config, using defaults");
        auth::config::AppConfigFile::default()
    });

    let cli_thinking = match cli.thinking.as_deref() {
        Some("low") => Some(protocol::types::ThinkingLevel::Low),
        Some("medium") => Some(protocol::types::ThinkingLevel::Medium),
        Some("high") => Some(protocol::types::ThinkingLevel::High),
        _ => None,
    };

    let mut config = config_file.merge_with_cli(
        cli.provider.as_deref(),
        cli.model.as_deref(),
        cli_thinking,
        cli.max_turns,
    );
    config.sessions_dir = sessions_dir.clone();

    let default_model = config.default_model.clone();
    let thinking_level = config.default_thinking.clone();

    // ------------------------------------------------------------------
    // Session store & session startup
    // ------------------------------------------------------------------

    let session_store = session::store::SessionStore::new(sessions_dir);
    let _ = session_store.ensure_dir().await;

    let loaded_session = if let Some(session_id) = &cli.session {
        // Load existing session; fall back to new session on failure
        match session_store.load_session(session_id).await {
            Ok(loaded) => {
                tracing::info!(session_id = %session_id, "Loaded session");
                loaded
            }
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "Failed to load session, creating new");
                create_default_session(&session_store, &default_model, &thinking_level, &cwd).await
            }
        }
    } else {
        tracing::info!("No session specified, creating new session");
        create_default_session(&session_store, &default_model, &thinking_level, &cwd).await
    };

    // Use model/thinking from session header (reflects persisted state)
    let model = loaded_session.header.model.clone();
    let thinking = loaded_session.header.thinking_level.clone();

    // ------------------------------------------------------------------
    // Build shared app state
    // ------------------------------------------------------------------

    let state = app::state::AppState::new(loaded_session, config, model, thinking, cwd.clone());
    let shared_state: app::state::SharedAppState = Arc::new(Mutex::new(state));

    // ------------------------------------------------------------------
    // Transport channels
    // ------------------------------------------------------------------

    let (output_tx, output_rx) = mpsc::unbounded_channel();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();

    // Start transport writer (stdout)
    let mut writer = transport::writer::TransportWriter::new(output_rx);
    let writer_handle = tokio::spawn(async move {
        writer.run().await;
    });

    // Start transport reader (stdin)
    let reader = transport::reader::TransportReader::new(cmd_tx.clone());
    let _reader_handle = reader.start();

    // ------------------------------------------------------------------
    // Build runtime
    // ------------------------------------------------------------------

    let runtime = app::runtime::AppRuntime::new(
        shared_state,
        output_tx.clone(),
        session_store,
        tools::registry::ToolRegistry::default(),
        process::bash_runner::BashRunner::new(cwd),
        tools::ask_user::AskManagerHandle::new(),
    );

    // ------------------------------------------------------------------
    // Command loop
    // ------------------------------------------------------------------

    while let Some(command) = cmd_rx.recv().await {
        let is_shutdown = matches!(command, protocol::commands::ClientCommand::Shutdown { .. });
        app::router::route_command(&runtime, command).await;
        if is_shutdown {
            break;
        }
    }

    // ------------------------------------------------------------------
    // Graceful shutdown
    // ------------------------------------------------------------------

    tracing::info!("Shutting down...");

    // Cancel any active run (handle_shutdown already does this, but be sure)
    {
        let state = runtime.state.lock().await;
        if let Some(cancel) = state.cancel_token() {
            cancel.cancel();
        }
    }

    // Drop the command sender so the reader loop exits on next failed send
    drop(cmd_tx);

    // Drop the runtime so output_tx is closed; writer will drain naturally
    drop(runtime);

    // Wait for the writer to finish flushing remaining events/responses
    let _ = writer_handle.await;

    tracing::info!("Shutdown complete.");
}

/// Seed the Codex credential cache from environment variables.
async fn setup_codex() -> std::io::Result<()> {
    let path = auth::codex::default_credentials_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let access_token = match std::env::var("CODEX_ACCESS_TOKEN") {
        Ok(token) if !token.is_empty() => token,
        _ => {
            eprintln!(
                "Created Codex auth directory at {}",
                path.parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .display()
            );
            eprintln!("Set CODEX_ACCESS_TOKEN and rerun `cromulent --setup-codex` to write cached credentials.");
            eprintln!("Optional: CODEX_REFRESH_TOKEN, CODEX_EXPIRES_AT, CODEX_SCOPE");
            return Ok(());
        }
    };

    let expires_at = std::env::var("CODEX_EXPIRES_AT")
        .unwrap_or_else(|_| (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339());

    let credentials = auth::codex::CodexCredentials {
        access_token,
        refresh_token: std::env::var("CODEX_REFRESH_TOKEN").unwrap_or_default(),
        expires_at,
        token_type: Some("Bearer".to_string()),
        scope: std::env::var("CODEX_SCOPE").ok(),
    };
    let cache = auth::codex::CredentialCache::new("codex", credentials);
    auth::codex::save_cached_credentials(&path, &cache).await?;
    eprintln!("Saved Codex credentials to {}", path.display());
    Ok(())
}

/// Create a new default session and persist it to disk.
async fn create_default_session(
    session_store: &session::store::SessionStore,
    model: &protocol::types::ModelInfo,
    thinking: &protocol::types::ThinkingLevel,
    cwd: &PathBuf,
) -> session::store::LoadedSessionState {
    let default_session_id = util::ids::generate_session_id();
    let default_header = session::store::SessionHeader::new(
        default_session_id,
        cwd.to_string_lossy().to_string(),
        model.clone(),
        thinking.clone(),
    );
    if let Err(e) = session_store.create_session(&default_header).await {
        tracing::warn!(error = %e, "Failed to persist new session");
    }
    session::store::LoadedSessionState {
        header: default_header,
        messages: Vec::new(),
    }
}
