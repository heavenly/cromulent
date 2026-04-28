use crate::app::runtime::AppRuntime;
use crate::protocol::commands::ClientCommand;

/// Route a parsed `ClientCommand` to the appropriate `AppRuntime` handler.
///
/// This function is the single dispatch point called from the main event loop.
/// It validates the command shape (at the variant level) and delegates to
/// the runtime's typed handler method.
///
/// All responses and events are emitted via the runtime's output channel.
pub async fn route_command(runtime: &AppRuntime, command: ClientCommand) {
    match command {
        ClientCommand::Prompt { id, message } => {
            runtime.handle_prompt(id, message).await;
        }
        ClientCommand::Abort { id } => {
            runtime.handle_abort(id).await;
        }
        ClientCommand::UserResponse {
            id,
            ask_id,
            response,
        } => {
            runtime.handle_user_response(id, ask_id, response).await;
        }
        ClientCommand::SetModel {
            id,
            provider,
            model_id,
        } => {
            runtime.handle_set_model(id, provider, model_id).await;
        }
        ClientCommand::SetThinking { id, level } => {
            runtime.handle_set_thinking(id, level).await;
        }
        ClientCommand::CycleModel { id } => {
            runtime.handle_cycle_model(id).await;
        }
        ClientCommand::ListSessions { id } => {
            runtime.handle_list_sessions(id).await;
        }
        ClientCommand::LoadSession { id, session_id } => {
            runtime.handle_load_session(id, session_id).await;
        }
        ClientCommand::NewSession { id } => {
            runtime.handle_new_session(id).await;
        }
        ClientCommand::ForkSession { id, entry_id } => {
            runtime.handle_fork_session(id, entry_id).await;
        }
        ClientCommand::GetState { id } => {
            runtime.handle_get_state(id).await;
        }
        ClientCommand::GetMessages { id } => {
            runtime.handle_get_messages(id).await;
        }
        ClientCommand::ExportSession { id, output_path } => {
            runtime.handle_export_session(id, output_path).await;
        }
        ClientCommand::Bash { id, command } => {
            runtime.handle_bash(id, command).await;
        }
        ClientCommand::Shutdown { id } => {
            runtime.handle_shutdown(id).await;
        }
        ClientCommand::Invalid => {
            runtime.handle_invalid_command().await;
        }
    }
}
