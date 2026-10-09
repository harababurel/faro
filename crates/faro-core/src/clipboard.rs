//! kitty's clipboard protocol, OSC 5522, read side (D89, D90).
//!
//! Pure: builds requests and parses replies. The app writes the request (wrapped
//! for tmux passthrough when needed) and feeds what it reads from the tty (D90)
//! to a [`Reader`]. Spec: <https://sw.kovidgoyal.net/kitty/clipboard/>.
//!
//! A reply is `OSC 5522 ; type=read:status=OK ST`, then
//! `OSC 5522 ; type=read:status=DATA:mime=<b64> ; <b64 chunk> ST` packets (at most
//! 4096 bytes per chunk before base64, every chunk padded), then
//! `OSC 5522 ; type=read:status=DONE ST`. An error status (`EPERM`, `EBUSY`,
//! `ENOSYS`) comes instead of `OK`, and nothing follows it. A request the
//! terminal rejects gets no reply at all, so the caller needs a timeout.

use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Payload asking for the list of types on the clipboard. kitty answers it
/// without a permission prompt.
pub const LIST: &str = ".";

/// Image types faro can send, best first.
pub const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Longest packet kept while waiting for its end. A 4096-byte chunk is 5,464
/// bytes of base64; the rest is metadata.
const MAX_PACKET: usize = 16 * 1024;

/// Longest other escape sequence kept while waiting for its end (focus and
/// mouse reports are a few bytes).
const MAX_SEQUENCE: usize = 64;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const PREFIX: &[u8] = b"\x1b]5522;";

/// A password and a name kitty (≥ 0.42.2) can offer to remember, so later
/// reads on the same tty don't prompt. kitty ignores a password without a name.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    password: String,
    name: String,
}

impl Credentials {
    pub fn new(password: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            password: password.into(),
            name: name.into(),
        }
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("password", &"<redacted>")
            .field("name", &self.name)
            .finish()
    }
}

/// Ask for the list of types on the clipboard.
pub fn list_request(id: Option<&str>) -> String {
    read_request(&[LIST], id, None)
}

/// Ask for the data of `types` (sent one type after another).
///
/// `id` is echoed back in every reply packet, which tells this request's reply
/// from a late one; characters kitty would strip are left out here already.
pub fn read_request(types: &[&str], id: Option<&str>, credentials: Option<&Credentials>) -> String {
    let mut meta = String::from("type=read");
    if let Some(id) = id.map(sanitize_id).filter(|id| !id.is_empty()) {
        meta.push_str(":id=");
        meta.push_str(&id);
    }
    if let Some(credentials) = credentials {
        meta.push_str(":pw=");
        meta.push_str(&STANDARD.encode(&credentials.password));
        meta.push_str(":name=");
        meta.push_str(&STANDARD.encode(&credentials.name));
    }
    format!(
        "\x1b]5522;{meta};{}\x1b\\",
        STANDARD.encode(types.join(" "))
    )
}

/// Keep only the characters the spec allows in an id (`[a-zA-Z0-9-_+.]`).
fn sanitize_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || "-_+.".contains(*c))
        .collect()
}

/// The best image type faro can send among `types`.
pub fn best_image_type(types: &[String]) -> Option<&'static str> {
    IMAGE_TYPES
        .into_iter()
        .find(|wanted| types.iter().any(|t| t == wanted))
}

/// File extension for an image type from [`IMAGE_TYPES`].
pub fn image_extension(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// A focus report found in the reply's bytes (kitty's prompt takes and gives
/// back focus; tmux may even put the report inside a packet). The app turns it
/// back into a focus event (D42, D90).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Gained,
    Lost,
}

/// Why a read gave no data.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("clipboard access denied")]
    Denied,
    #[error("the clipboard is busy")]
    Busy,
    #[error("the clipboard isn't available")]
    Unavailable,
    #[error("the terminal answered {0}")]
    Status(String),
    #[error("the clipboard data is over {limit} bytes")]
    TooLarge { limit: usize },
    #[error("malformed reply: {0}")]
    Malformed(String),
    #[error("the reply stopped before the end")]
    Incomplete,
    #[error("no answer from the terminal")]
    NoReply,
}

/// The data for one type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub mime: String,
    pub data: Vec<u8>,
}

/// What a read returned, one entry per type, in the order they came.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contents {
    pub entries: Vec<Entry>,
}

impl Contents {
    /// The types listed in the reply to a [`list_request`].
    pub fn types(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.mime == LIST)
            .flat_map(|entry| {
                String::from_utf8_lossy(&entry.data)
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The data for `mime`, if it came.
    pub fn take(self, mime: &str) -> Option<Entry> {
        self.entries.into_iter().find(|entry| entry.mime == mime)
    }
}

/// Assembles one reply from bytes read off the tty, in pieces of any size.
///
/// Never panics, never stores more than `limit` decoded bytes, and never
/// buffers more than one packet. Keeps reading to `DONE` after the data turns
/// out too large or malformed, so the rest of the reply doesn't leak into the
/// key input. Bytes outside packets are dropped, except focus reports.
#[derive(Debug)]
pub struct Reader {
    buf: Vec<u8>,
    limit: usize,
    id: Option<String>,
    ok: bool,
    done: bool,
    status: Option<ReadError>,
    malformed: Option<String>,
    too_large: bool,
    entries: Vec<Entry>,
    stored: usize,
    packets: usize,
    ignored: usize,
    dropped: usize,
    focus: Vec<Focus>,
}

/// What the bytes at an `ESC` are.
enum Sequence {
    /// Not complete yet.
    Wait,
    /// Something else (or garbage): drop this many bytes.
    Skip(usize),
    Focus(Focus, usize),
    /// An OSC 5522 packet: its body ends at `end`, the whole thing is `len` bytes.
    Packet {
        end: usize,
        len: usize,
    },
    /// An OSC 5522 packet longer than [`MAX_PACKET`] with no end in sight.
    Overflow(usize),
}

impl Reader {
    /// `id`: the one given to [`read_request`]. Packets with another id are
    /// ignored; packets without one are taken (terminals that don't echo it).
    pub fn new(limit: usize, id: Option<&str>) -> Self {
        Self {
            buf: Vec::new(),
            limit,
            id: id.map(sanitize_id).filter(|id| !id.is_empty()),
            ok: false,
            done: false,
            status: None,
            malformed: None,
            too_large: false,
            entries: Vec::new(),
            stored: 0,
            packets: 0,
            ignored: 0,
            dropped: 0,
            focus: Vec::new(),
        }
    }

    /// Take more bytes. The reader stops at the end of its reply: bytes after
    /// it (another terminal's reply, under tmux with several clients) stay
    /// unread for [`take_rest`](Self::take_rest), so stop feeding once
    /// [`finished`](Self::finished).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        if self.finished() {
            return;
        }
        let buf = std::mem::take(&mut self.buf);
        let mut pos = 0;
        while let Some(rest) = buf.get(pos..) {
            let Some(esc) = rest.iter().position(|&b| b == ESC) else {
                self.dropped += rest.len();
                pos = buf.len();
                break;
            };
            self.dropped += esc;
            pos += esc;
            let rest = buf.get(pos..).unwrap_or_default();
            match sequence(rest) {
                Sequence::Wait => break,
                Sequence::Skip(n) => {
                    self.dropped += n;
                    pos += n;
                }
                Sequence::Focus(focus, n) => {
                    self.focus.push(focus);
                    pos += n;
                }
                Sequence::Packet { end, len } => {
                    let body = rest.get(PREFIX.len()..end).unwrap_or_default();
                    let body = self.strip_sequences(body);
                    self.packet(&body);
                    pos += len;
                    if self.finished() {
                        break;
                    }
                }
                Sequence::Overflow(n) => {
                    self.malformed
                        .get_or_insert_with(|| "a packet with no end".to_owned());
                    self.dropped += n;
                    pos += n;
                }
            }
        }
        self.buf = buf;
        self.buf.drain(..pos.min(self.buf.len()));
    }

    /// A packet of this reply has arrived (switch from waiting for the user to
    /// answer the prompt to waiting between packets).
    pub fn started(&self) -> bool {
        self.packets > 0
    }

    /// Nothing more will come: `DONE`, or an error status.
    pub fn finished(&self) -> bool {
        self.done || self.status.is_some()
    }

    /// Focus reports seen since the last call, in order.
    pub fn take_focus(&mut self) -> Vec<Focus> {
        std::mem::take(&mut self.focus)
    }

    /// Bytes outside packets that were dropped (keys typed meanwhile, mouse
    /// reports, other replies).
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Packets for another request (other id or type).
    pub fn ignored(&self) -> usize {
        self.ignored
    }

    /// The bytes after the end of this reply (see [`feed`](Self::feed)).
    pub fn take_rest(&mut self) -> Vec<u8> {
        if self.finished() {
            std::mem::take(&mut self.buf)
        } else {
            Vec::new()
        }
    }

    /// Bytes held while waiting for the rest of a sequence.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// The outcome. Call it once [`finished`](Self::finished), or when the
    /// caller's timeout runs out.
    pub fn finish(self) -> Result<Contents, ReadError> {
        if let Some(status) = self.status {
            return Err(status);
        }
        if !self.done {
            return Err(if self.started() {
                ReadError::Incomplete
            } else {
                ReadError::NoReply
            });
        }
        if self.too_large {
            return Err(ReadError::TooLarge { limit: self.limit });
        }
        if let Some(reason) = self.malformed {
            return Err(ReadError::Malformed(reason));
        }
        Ok(Contents {
            entries: self.entries,
        })
    }

    /// Packet bodies never contain `ESC` (base64 and `key=value` text), so a
    /// complete `ESC [ … final` inside one is another sequence that got mixed
    /// in: cut it out (keeping focus reports). Any other `ESC` stays, for the
    /// parser to reject.
    fn strip_sequences(&mut self, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(body.len());
        let mut pos = 0;
        while let Some(rest) = body.get(pos..).filter(|rest| !rest.is_empty()) {
            if rest.first() == Some(&ESC) && rest.get(1) == Some(&b'[') {
                match csi(rest) {
                    Sequence::Focus(focus, n) => {
                        self.focus.push(focus);
                        pos += n;
                        continue;
                    }
                    Sequence::Skip(n) if is_complete_csi(rest, n) => {
                        self.dropped += n;
                        pos += n;
                        continue;
                    }
                    Sequence::Wait
                    | Sequence::Skip(_)
                    | Sequence::Packet { .. }
                    | Sequence::Overflow(_) => {}
                }
            }
            out.extend(rest.first());
            pos += 1;
        }
        out
    }

    fn packet(&mut self, body: &[u8]) {
        let Ok(text) = std::str::from_utf8(body) else {
            self.packets += 1;
            self.malformed
                .get_or_insert_with(|| "a packet that isn't UTF-8".to_owned());
            return;
        };
        let (meta, payload) = text.split_once(';').unwrap_or((text, ""));
        let mut kind = None;
        let mut status = None;
        let mut mime = None;
        let mut id = None;
        for pair in meta.split(':') {
            match pair.split_once('=') {
                Some(("type", value)) => kind = Some(value),
                Some(("status", value)) => status = Some(value),
                Some(("mime", value)) => mime = Some(value),
                Some(("id", value)) => id = Some(value),
                _ => {}
            }
        }
        let other_request = match (&self.id, id) {
            (Some(ours), Some(theirs)) => ours != theirs,
            _ => false,
        };
        if kind != Some("read") || other_request {
            self.ignored += 1;
            return;
        }
        self.packets += 1;
        match status {
            Some("OK") => self.ok = true,
            Some("DONE") => self.done = true,
            Some("DATA") => self.data(mime, payload),
            Some("EPERM") => self.status = Some(ReadError::Denied),
            Some("EBUSY") => self.status = Some(ReadError::Busy),
            Some("ENOSYS") => self.status = Some(ReadError::Unavailable),
            Some(other) => self.status = Some(ReadError::Status(other.to_owned())),
            None => {
                self.malformed
                    .get_or_insert_with(|| format!("a packet without a status ({meta})"));
            }
        }
    }

    fn data(&mut self, mime: Option<&str>, payload: &str) {
        let mime = match mime.map(|m| STANDARD.decode(m)) {
            Some(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Some(Err(error)) => {
                self.malformed
                    .get_or_insert_with(|| format!("a bad type: {error}"));
                return;
            }
            None => {
                self.malformed
                    .get_or_insert_with(|| "data without a type".to_owned());
                return;
            }
        };
        let bytes = match STANDARD.decode(payload) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.malformed
                    .get_or_insert_with(|| format!("bad data for {mime}: {error}"));
                return;
            }
        };
        if self.too_large {
            return;
        }
        if self.stored.saturating_add(bytes.len()) > self.limit {
            // Free the memory now; keep reading to DONE.
            self.too_large = true;
            self.entries = Vec::new();
            return;
        }
        self.stored += bytes.len();
        match self.entries.last_mut() {
            Some(last) if last.mime == mime => last.data.extend(bytes),
            Some(_) | None => self.entries.push(Entry { mime, data: bytes }),
        }
    }
}

/// Classify the bytes starting at an `ESC`.
fn sequence(rest: &[u8]) -> Sequence {
    match rest.get(1) {
        None => Sequence::Wait,
        Some(b']') => osc(rest),
        Some(b'[') => csi(rest),
        // `ESC` + a key (Alt+x) or garbage.
        Some(_) => Sequence::Skip(2),
    }
}

fn osc(rest: &[u8]) -> Sequence {
    let ours = rest.starts_with(PREFIX);
    match terminator(rest) {
        Some((end, len)) if ours => Sequence::Packet {
            end,
            len: end + len,
        },
        // Another OSC reply: not ours, drop it whole.
        Some((end, len)) => Sequence::Skip(end + len),
        None if rest.len() > MAX_PACKET && ours => Sequence::Overflow(rest.len()),
        None if rest.len() > MAX_PACKET => Sequence::Skip(rest.len()),
        None => Sequence::Wait,
    }
}

/// Where the string terminator (`ESC \` or `BEL`) starts, and its length.
fn terminator(rest: &[u8]) -> Option<(usize, usize)> {
    let mut iter = rest.iter().enumerate().skip(2).peekable();
    while let Some((i, &b)) = iter.next() {
        match b {
            BEL => return Some((i, 1)),
            ESC if iter.peek().map(|&(_, &next)| next) == Some(b'\\') => return Some((i, 2)),
            _ => {}
        }
    }
    None
}

/// `ESC [ params intermediates final`. Focus reports are `ESC [ I` and `ESC [ O`.
fn csi(rest: &[u8]) -> Sequence {
    for (i, &b) in rest.iter().enumerate().skip(2) {
        match b {
            0x20..=0x3f if i < MAX_SEQUENCE => {}
            0x40..=0x7e => {
                return match rest.get(..=i) {
                    Some(b"\x1b[I") => Sequence::Focus(Focus::Gained, i + 1),
                    Some(b"\x1b[O") => Sequence::Focus(Focus::Lost, i + 1),
                    _ => Sequence::Skip(i + 1),
                };
            }
            // Not a CSI after all (or far too long): drop what came before.
            _ => return Sequence::Skip(i),
        }
    }
    Sequence::Wait
}

/// The `n` bytes `csi` skipped end with a final byte (a whole sequence, not
/// garbage it gave up on).
fn is_complete_csi(rest: &[u8], n: usize) -> bool {
    n >= 3 && rest.get(n - 1).is_some_and(|b| (0x40..=0x7e).contains(b))
}

#[cfg(test)]
mod tests;
