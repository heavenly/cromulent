use crate::protocol::events::ServerEvent;
use crate::protocol::responses::CommandResponse;
use crate::transport::writer::OutputItem;
use tokio::sync::mpsc;

/// Helper function to emit events via the output queue
pub fn emit_event(tx: &mpsc::UnboundedSender<OutputItem>, event: ServerEvent) {
    let value = serde_json::to_value(event).expect("Event serialization failed");
    let _ = tx.send(OutputItem::Event(value));
}

/// Helper function to send command responses via the output queue
pub fn respond(tx: &mpsc::UnboundedSender<OutputItem>, response: CommandResponse) {
    let _ = tx.send(OutputItem::Response(response));
}
