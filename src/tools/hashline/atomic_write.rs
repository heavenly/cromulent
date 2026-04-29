use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let target = tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf());
    let meta = tokio::fs::metadata(&target).await.ok();
    #[cfg(unix)]
    let nlink = {
        use std::os::unix::fs::MetadataExt;
        meta.as_ref().map(|m| m.nlink()).unwrap_or(1)
    };
    #[cfg(not(unix))]
    let nlink = 1u64;
    if nlink > 1 {
        return tokio::fs::write(&target, content).await;
    }
    let dir = target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    tokio::fs::create_dir_all(&dir).await?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tmp = dir.join(format!(".tmp-cromulent-{nonce}-{}", std::process::id()));
    tokio::fs::write(&tmp, content).await?;
    if let Some(m) = meta {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(
                &tmp,
                std::fs::Permissions::from_mode(m.permissions().mode()),
            )
            .await?;
        }
    }
    tokio::fs::rename(tmp, target).await
}
