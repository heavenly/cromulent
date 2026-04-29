use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tokio::sync::Mutex;
static LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
pub fn file_lock(path: &Path) -> Arc<Mutex<()>> {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let map = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap();
    guard
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}
