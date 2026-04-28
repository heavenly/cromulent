use std::path::PathBuf;

/// Get the default sessions directory
pub fn default_sessions_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cromulent")
        .join("sessions")
}

/// Get the default config directory
pub fn default_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cromulent")
}

/// Get the default config file path
pub fn default_config_path() -> PathBuf {
    default_config_dir().join("config.json")
}
