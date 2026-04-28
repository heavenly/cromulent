use std::path::PathBuf;

/// Get the default cromulent home directory (`~/.cromulent`).
pub fn default_cromulent_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cromulent")
}

/// Get the default sessions directory (`~/.cromulent/sessions`).
pub fn default_sessions_dir() -> PathBuf {
    default_cromulent_dir().join("sessions")
}

/// Get the default config directory (`~/.cromulent`).
pub fn default_config_dir() -> PathBuf {
    default_cromulent_dir()
}

/// Get the default config file path (`~/.cromulent/config.json`).
pub fn default_config_path() -> PathBuf {
    default_config_dir().join("config.json")
}
