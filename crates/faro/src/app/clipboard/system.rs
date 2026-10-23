//! Reading an image from the computer's own clipboard (D150), for terminals
//! that can't share theirs, when faro runs on that computer. Only when the
//! user presses `Ctrl+V`, and only image types: text on the clipboard is
//! never read. macOS: `osascript`; elsewhere: `wl-paste` (Wayland) or
//! `xclip` (X11).

use std::process::Stdio;
use std::time::Duration;

use faro_core::clipboard::ReadError;
use tokio::io::AsyncReadExt as _;

use super::super::event::ClipboardError;
use super::{MAX_IMAGE, limit_for};

/// How long a clipboard tool gets (the app that owns the clipboard serves
/// the data, and may be slow or stuck).
const COMMAND_WAIT: Duration = Duration::from_secs(10);

/// An image read from the clipboard, as it came.
#[derive(Debug)]
pub struct Image {
    pub mime: &'static str,
    pub data: Vec<u8>,
}

/// Read the best image on the clipboard.
pub async fn read() -> Result<Image, ClipboardError> {
    #[cfg(target_os = "macos")]
    {
        macos::read().await
    }
    #[cfg(not(target_os = "macos"))]
    {
        desktop::read().await
    }
}

/// Why a command gave nothing.
#[derive(Debug, PartialEq, Eq)]
enum RunError {
    /// Not installed.
    Missing,
    /// It failed, timed out or said there's nothing.
    Failed,
    /// Its output went over the limit (it was stopped).
    TooLarge,
}

/// Run `program`, taking at most `limit` bytes of its output.
async fn output(program: &str, args: &[&str], limit: usize) -> Result<Vec<u8>, RunError> {
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RunError::Missing
            } else {
                RunError::Failed
            }
        })?;
    let stdout = child.stdout.take().ok_or(RunError::Failed)?;
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let read = async {
        let mut data = Vec::new();
        stdout.take(cap).read_to_end(&mut data).await.map(|_| data)
    };
    // On every early return the child is dropped, which kills it.
    let Ok(Ok(data)) = tokio::time::timeout(COMMAND_WAIT, read).await else {
        return Err(RunError::Failed);
    };
    if data.len() > limit {
        return Err(RunError::TooLarge);
    }
    match tokio::time::timeout(COMMAND_WAIT, child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(data),
        Ok(Ok(_) | Err(_)) | Err(_) => Err(RunError::Failed),
    }
}

fn too_large() -> ClipboardError {
    ClipboardError::Read(ReadError::TooLarge { limit: MAX_IMAGE })
}

/// The best type faro takes among `types` (D150: TIFF last).
fn best(types: &[String]) -> Option<&'static str> {
    faro_core::clipboard::best_image_type(types)
}

/// macOS: `clipboard info` lists the types, then `the clipboard as «class
/// PNGf»` (or JPEG, GIF, TIFF) prints the data as `«data PNGf89504E47…»`.
#[cfg(any(target_os = "macos", test))]
mod macos {
    use super::{ClipboardError, Image, RunError, best, limit_for, output, too_large};

    /// `clipboard info`'s names for the types faro takes (`AppleScript` shows
    /// some by name, others as raw class codes).
    const NAMES: [(&str, &str); 7] = [
        ("«class PNGf»", "image/png"),
        ("JPEG picture", "image/jpeg"),
        ("«class JPEG»", "image/jpeg"),
        ("GIF picture", "image/gif"),
        ("«class GIFf»", "image/gif"),
        ("TIFF picture", "image/tiff"),
        ("«class TIFF»", "image/tiff"),
    ];

    /// The image types in `clipboard info`'s output (`«class PNGf», 12345,
    /// TIFF picture, 67890, …`: names and sizes, comma-separated).
    pub fn types(info: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for name in info.split(',').map(str::trim) {
            if let Some((_, mime)) = NAMES.iter().find(|(known, _)| *known == name)
                && !found.iter().any(|f| f == mime)
            {
                found.push((*mime).to_owned());
            }
        }
        found
    }

    /// The `AppleScript` class to ask for.
    pub fn class(mime: &str) -> Option<&'static str> {
        match mime {
            "image/png" => Some("PNGf"),
            "image/jpeg" => Some("JPEG"),
            "image/gif" => Some("GIFf"),
            "image/tiff" => Some("TIFF"),
            _ => None,
        }
    }

    /// The bytes in `«data PNGf89504E47…»`.
    pub fn data(text: &str) -> Option<Vec<u8>> {
        let rest = text.trim().strip_prefix("«data ")?;
        // A four-character class code (may end with a space), then hex.
        let hex = rest.get(4..)?.strip_suffix('»')?;
        let digits = hex.as_bytes();
        if digits.len() % 2 != 0 {
            return None;
        }
        digits
            .chunks(2)
            .map(|pair| {
                let text = std::str::from_utf8(pair).ok()?;
                u8::from_str_radix(text, 16).ok()
            })
            .collect()
    }

    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "macOS only; tests elsewhere cover the parsers")
    )]
    pub async fn read() -> Result<Image, ClipboardError> {
        let info = output("osascript", &["-e", "clipboard info"], 64 * 1024)
            .await
            .map_err(|error| match error {
                RunError::Missing => ClipboardError::Failed("osascript not found".to_owned()),
                RunError::Failed | RunError::TooLarge => ClipboardError::NoImage,
            })?;
        let types = types(&String::from_utf8_lossy(&info));
        let mime = best(&types).ok_or(ClipboardError::NoImage)?;
        let class = class(mime).ok_or(ClipboardError::NoImage)?;
        let script = format!("the clipboard as «class {class}»");
        // Two hex digits per byte, plus the `«data XXXX` wrapping.
        let limit = limit_for(mime).saturating_mul(2).saturating_add(64);
        let text =
            output("osascript", &["-e", &script], limit)
                .await
                .map_err(|error| match error {
                    RunError::TooLarge => too_large(),
                    RunError::Missing | RunError::Failed => ClipboardError::NoImage,
                })?;
        let data = data(&String::from_utf8_lossy(&text))
            .ok_or_else(|| ClipboardError::Failed("osascript's answer wasn't an image".into()))?;
        Ok(Image { mime, data })
    }
}

/// Linux and the BSDs: `wl-paste` under Wayland, `xclip` under X11 (also
/// under Wayland through `XWayland`, when wl-clipboard isn't installed).
#[cfg(not(target_os = "macos"))]
mod desktop {
    use super::{ClipboardError, Image, RunError, best, limit_for, output, too_large};

    #[derive(Clone, Copy)]
    enum Tool {
        WlPaste,
        Xclip,
    }

    impl Tool {
        const fn program(self) -> &'static str {
            match self {
                Self::WlPaste => "wl-paste",
                Self::Xclip => "xclip",
            }
        }

        fn list(self) -> Vec<&'static str> {
            match self {
                Self::WlPaste => vec!["--list-types"],
                Self::Xclip => vec!["-selection", "clipboard", "-target", "TARGETS", "-out"],
            }
        }

        fn read(self, mime: &str) -> Vec<&str> {
            match self {
                Self::WlPaste => vec!["--no-newline", "--type", mime],
                Self::Xclip => vec!["-selection", "clipboard", "-target", mime, "-out"],
            }
        }
    }

    pub async fn read() -> Result<Image, ClipboardError> {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let x11 = std::env::var_os("DISPLAY").is_some();
        if !wayland && !x11 {
            return Err(ClipboardError::NoDesktop);
        }
        let tools = [wayland.then_some(Tool::WlPaste), x11.then_some(Tool::Xclip)];
        for tool in tools.into_iter().flatten() {
            match with(tool).await {
                Err(Missing) => {}
                Ok(image) => return Ok(image),
                Err(Found(error)) => return Err(error),
            }
        }
        Err(ClipboardError::NeedsTool)
    }

    /// Not installed, or what the installed tool gave.
    enum Outcome {
        Missing,
        Found(ClipboardError),
    }
    use Outcome::{Found, Missing};

    async fn with(tool: Tool) -> Result<Image, Outcome> {
        let program = tool.program();
        let listed = match output(program, &tool.list(), 64 * 1024).await {
            Ok(listed) => listed,
            Err(RunError::Missing) => return Err(Missing),
            // An empty clipboard makes `wl-paste` fail.
            Err(RunError::Failed | RunError::TooLarge) => {
                return Err(Found(ClipboardError::NoImage));
            }
        };
        let types: Vec<String> = String::from_utf8_lossy(&listed)
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect();
        let mime = best(&types).ok_or(Found(ClipboardError::NoImage))?;
        let data = output(program, &tool.read(mime), limit_for(mime))
            .await
            .map_err(|error| {
                Found(match error {
                    RunError::TooLarge => too_large(),
                    RunError::Missing | RunError::Failed => {
                        ClipboardError::Failed(format!("{program} couldn't read the image"))
                    }
                })
            })?;
        Ok(Image { mime, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_image_types_in_clipboard_info() {
        let info = "«class PNGf», 285083, «class 8BPS», 1022, GIF picture, 3044, \
                    «class jp2 », 9000, JPEG picture, 22071, TIFF picture, 1200000, \
                    «class BMP », 1200054, «class TPIC», 9";
        assert_eq!(
            macos::types(info),
            vec!["image/png", "image/gif", "image/jpeg", "image/tiff"]
        );
        assert_eq!(best(&macos::types(info)), Some("image/png"));
        // Only TIFF (some macOS apps): taken, converted later.
        let tiff = "TIFF picture, 1200000, «class 8BPS», 1022";
        assert_eq!(best(&macos::types(tiff)), Some("image/tiff"));
        // Text only: no image.
        let text = "«class utf8», 12, string, 12, Unicode text, 26";
        assert_eq!(macos::types(text), Vec::<String>::new());
        assert_eq!(macos::class("image/tiff"), Some("TIFF"));
        assert_eq!(macos::class("image/webp"), None);
    }

    #[test]
    fn decodes_osascript_data() {
        assert_eq!(
            macos::data("«data PNGf89504E470D0A1A0A»\n"),
            Some(vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
        );
        assert_eq!(macos::data("«data jp2 FFD8»"), Some(vec![0xff, 0xd8]));
        assert_eq!(macos::data("«data PNGf89504»"), None, "odd digits");
        assert_eq!(macos::data("«data PNGfZZ»"), None);
        assert_eq!(macos::data("missing value"), None);
    }

    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn output_is_capped_and_missing_tools_are_told_apart() {
        assert_eq!(
            output("printf", &["hello"], 16).await.as_deref(),
            Ok(&b"hello"[..])
        );
        assert_eq!(
            output("printf", &["hello"], 4).await,
            Err(RunError::TooLarge)
        );
        assert_eq!(
            output("faro-no-such-tool", &[], 16).await,
            Err(RunError::Missing)
        );
        assert_eq!(output("false", &[], 16).await, Err(RunError::Failed));
    }
}
