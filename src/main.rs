use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod app;
mod agent;
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
    #[arg(long, default_value_t = 40)]
    max_turns: u32,

    /// Directory for session persistence. Defaults to XDG data dir.
    #[arg(long)]
    sessions_dir: Option<PathBuf>,

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
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // --setup-codex: run auth setup and exit
    if cli.setup_codex {
        tracing::info!("Running codex auth setup...");
        // TODO: implement actual codex auth setup
        eprintln!("Codex auth setup is not yet implemented.");
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

    // Parse thinking level from CLI string, defaulting to Medium
    let thinking_level = match cli.thinking.as_deref() {
        Some("low") => protocol::types::ThinkingLevel::Low,
        Some("high") => protocol::types::ThinkingLevel::High,
        _ => protocol::types::ThinkingLevel::Medium,
    };

    let default_model = protocol::types::ModelInfo {
        provider: cli.provider.clone().unwrap_or_else(|| "openai".to_string()),
        id: cli.model.clone().unwrap_or_else(|| "gpt-4o".to_string()),
        display_name: String::new(),
        context_window: 128_000,
        supports_reasoning: matches!(thinking_level, protocol::types::ThinkingLevel::High),
        supports_tools: true,
    };

    let config = app::state::AppConfig {
        max_turns: cli.max_turns,
        sessions_dir: sessions_dir.clone(),
        default_model: default_model.clone(),
        default_thinking: thinking_level.clone(),
    };

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

    let state = app::state::AppState::new(
        loaded_session,
        config,
        model,
        thinking,
        cwd.clone(),
    );
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
