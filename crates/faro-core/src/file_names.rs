//! Names for attachment files and MIME types (D75, D77): pure string work,
//! shared by the terminal (`paths`, which re-exports these) and the web.

use std::path::Path;

use crate::domain::MessageRef;

/// File name under `attachments/` for the `index`-th attachment on `message` (D75):
/// `<sent_at>-<sender8>-<index>-<safe name>`.
pub fn attachment_file_name(
    message: MessageRef,
    index: usize,
    file_name: Option<&str>,
    content_type: &str,
) -> String {
    let sent_at = message.sent_at.as_millis();
    let mut sender = message.sender.uuid().simple().to_string();
    sender.truncate(8);
    let safe = sanitize_file_name(file_name, content_type);
    format!("{sent_at}-{sender}-{index}-{safe}")
}

/// Reduce an untrusted attachment file name to safe ASCII characters, falling
/// back to `attachment.<ext>` from `content_type` when nothing usable remains.
pub fn sanitize_file_name(file_name: Option<&str>, content_type: &str) -> String {
    let cleaned = file_name
        .map(|raw| raw.rsplit(['/', '\\']).next().unwrap_or(raw))
        .map(|base| {
            base.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .map(|s| clamp_name(s.trim_start_matches(['.', '_'])))
        .filter(|s| !s.is_empty());
    cleaned.unwrap_or_else(|| format!("attachment.{}", extension_for(content_type)))
}

/// Longest attachment file name we keep.
const MAX_NAME_LEN: usize = 64;

/// Longest extension we keep when a name has to be shortened.
const MAX_EXT_LEN: usize = 10;

/// Shorten `name` to [`MAX_NAME_LEN`] characters, keeping a short extension so
/// a long `report.pdf` doesn't end up as `report.p`. Names of at most
/// [`MAX_NAME_LEN`] characters are returned as they are. `name` comes from
/// [`sanitize_file_name`], so it is ASCII: its bytes are its characters.
fn clamp_name(name: &str) -> String {
    if name.len() <= MAX_NAME_LEN {
        return name.to_owned();
    }
    let ext = match name.rfind('.') {
        Some(dot) if dot > 0 => name.get(dot + 1..).unwrap_or_default(),
        _ => "",
    };
    if ext.is_empty() || ext.len() > MAX_EXT_LEN {
        return name.chars().take(MAX_NAME_LEN).collect();
    }
    let mut short = name
        .chars()
        .take(MAX_NAME_LEN - ext.len() - 1)
        .collect::<String>();
    short.push('.');
    short.push_str(ext);
    short
}

/// File extension (without leading dot) for a MIME type.
pub fn extension_for(content_type: &str) -> &'static str {
    match content_type.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "audio/aac" | "audio/mp4" => "m4a",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/csv" => "csv",
        _ => "bin",
    }
}

/// MIME type guessed from a file path's extension (D77).
pub fn content_type_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("m4a" | "aac") => "audio/aac",
        Some("ogg" | "oga" | "opus") => "audio/ogg",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log" | "rs" | "toml" | "json") => "text/plain",
        Some("csv") => "text/csv",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}
