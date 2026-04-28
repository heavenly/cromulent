use chrono::Utc;

/// Get current timestamp in ISO 8601 format
pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}
