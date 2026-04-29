use std::path::Path;

pub enum LoadedFile {
    Directory,
    Image(&'static str),
    Binary(String),
    Text(String),
}

pub async fn load_file(path: &Path) -> std::io::Result<LoadedFile> {
    let meta = tokio::fs::metadata(path).await?;
    if meta.is_dir() {
        return Ok(LoadedFile::Directory);
    }
    if !meta.is_file() {
        return Ok(LoadedFile::Binary("unsupported file type".into()));
    }
    let bytes = tokio::fs::read(path).await?;
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Ok(LoadedFile::Image("image/png"));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Ok(LoadedFile::Image("image/jpeg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok(LoadedFile::Image("image/gif"));
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok(LoadedFile::Image("image/webp"));
    }
    if bytes.contains(&0) {
        return Ok(LoadedFile::Binary("null bytes detected".into()));
    }
    match String::from_utf8(bytes) {
        Ok(s) => Ok(LoadedFile::Text(
            strip_bom(&s).replace("\r\n", "\n").replace('\r', "\n"),
        )),
        Err(_) => Ok(LoadedFile::Binary("invalid UTF-8".into())),
    }
}
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}
