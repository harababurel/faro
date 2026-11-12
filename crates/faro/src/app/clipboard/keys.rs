//! Decoding user keystrokes and bracketed pastes captured on the tty outside
//! `OSC 5522` packets while a clipboard image read is in flight.
//!
//! Bare `Enter` (`\r` / `CSI 13 u`) is intentionally withheld so answering a
//! terminal permission prompt or typing ahead cannot prematurely send a
//! message before its image is staged (D91). `Esc` and `Ctrl+C` cancel the
//! clipboard read immediately.

use crate::app::term_event::{Event, KeyCode, KeyEvent, KeyModifiers};

const ESC: u8 = 0x1b;
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const MAX_PASTE: usize = 64 * 1024;

/// Decoded terminal events and whether the user cancelled (`Esc` or `Ctrl+C`).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    pub events: Vec<Event>,
    pub cancelled: bool,
}

/// Stateful UTF-8, escape-sequence, and bracketed-paste decoder across reads.
#[derive(Debug, Default)]
pub struct KeyDecoder {
    buf: Vec<u8>,
    in_paste: bool,
    paste_buf: Vec<u8>,
}

enum CsiOutcome {
    Key(KeyEvent),
    Cancel,
    PasteStart,
    Ignore,
}

impl KeyDecoder {
    pub fn feed(&mut self, bytes: &[u8]) -> Decoded {
        self.buf.extend_from_slice(bytes);
        let buf = std::mem::take(&mut self.buf);
        let mut out = Decoded::default();
        let mut pos = 0;

        while let Some(rest) = buf.get(pos..).filter(|r| !r.is_empty()) {
            if self.in_paste {
                if let Some(end) = find_subslice(rest, PASTE_END) {
                    self.push_paste(rest.get(..end).unwrap_or_default());
                    let pasted = String::from_utf8_lossy(&self.paste_buf).into_owned();
                    self.paste_buf.clear();
                    self.in_paste = false;
                    out.events.push(Event::Paste(pasted));
                    pos += end + PASTE_END.len();
                    continue;
                }
                let keep = trailing_prefix_len(rest, PASTE_END);
                let take = rest.len().saturating_sub(keep);
                self.push_paste(rest.get(..take).unwrap_or_default());
                pos += take;
                break;
            }

            let Some(&b) = rest.first() else {
                break;
            };
            match Self::step_byte(b, rest, &mut out) {
                Some(Step::Advance(n)) => pos += n,
                Some(Step::PasteStart(n)) => {
                    self.in_paste = true;
                    pos += n;
                }
                Some(Step::Cancel) => {
                    out.cancelled = true;
                    pos = buf.len();
                    break;
                }
                None => break,
            }
        }

        self.buf = buf;
        self.buf.drain(..pos.min(self.buf.len()));
        out
    }

    fn step_byte(b: u8, rest: &[u8], out: &mut Decoded) -> Option<Step> {
        match b {
            // Ctrl+C cancels at once.
            0x03 => Some(Step::Cancel),
            ESC => Self::step_esc(rest, out),
            0x7f | 0x08 => {
                out.events.push(Event::Key(KeyEvent::new(
                    KeyCode::Backspace,
                    KeyModifiers::NONE,
                )));
                Some(Step::Advance(1))
            }
            b'\t' => {
                out.events
                    .push(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
                Some(Step::Advance(1))
            }
            // Bare Enter (`\r`) and duplicate `Ctrl+V` (`0x16`) are withheld (D91).
            b'\r' | 0x16 => Some(Step::Advance(1)),
            0x01..=0x1a => {
                let ch = char::from(b - 1 + b'a');
                out.events.push(Event::Key(KeyEvent::new(
                    KeyCode::Char(ch),
                    KeyModifiers::CONTROL,
                )));
                Some(Step::Advance(1))
            }
            0x20..=0x7e => {
                let ch = char::from(b);
                let mods = if ch.is_ascii_uppercase() {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                };
                out.events
                    .push(Event::Key(KeyEvent::new(KeyCode::Char(ch), mods)));
                Some(Step::Advance(1))
            }
            0x80..=0xff => {
                let (ch, len) = decode_utf8_char(rest)?;
                if let Some(ch) = ch {
                    out.events.push(Event::Key(KeyEvent::new(
                        KeyCode::Char(ch),
                        KeyModifiers::NONE,
                    )));
                }
                Some(Step::Advance(len))
            }
            _ => Some(Step::Advance(1)),
        }
    }

    fn push_paste(&mut self, bytes: &[u8]) {
        let room = MAX_PASTE.saturating_sub(self.paste_buf.len());
        if let Some(slice) = bytes.get(..bytes.len().min(room)) {
            self.paste_buf.extend_from_slice(slice);
        }
    }

    fn step_esc(rest: &[u8], out: &mut Decoded) -> Option<Step> {
        match rest.get(1).copied() {
            // Standalone Esc (already idle-flushed) or Esc followed by Esc.
            None | Some(ESC) => Some(Step::Cancel),
            Some(b'[') => {
                let len = complete_csi_len(rest)?;
                let seq = rest.get(..len).unwrap_or_default();
                match decode_csi(seq) {
                    CsiOutcome::Cancel => Some(Step::Cancel),
                    CsiOutcome::PasteStart => Some(Step::PasteStart(len)),
                    CsiOutcome::Key(ev) => {
                        out.events.push(Event::Key(ev));
                        Some(Step::Advance(len))
                    }
                    CsiOutcome::Ignore => Some(Step::Advance(len)),
                }
            }
            Some(b'O') => {
                if let Some(final_byte @ (b'A'..=b'D' | b'F' | b'H')) = rest.get(2).copied() {
                    if let Some(code) = arrow_or_home(final_byte) {
                        out.events
                            .push(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
                    }
                    Some(Step::Advance(3))
                } else {
                    out.events.push(Event::Key(KeyEvent::new(
                        KeyCode::Char('O'),
                        KeyModifiers::ALT | KeyModifiers::SHIFT,
                    )));
                    Some(Step::Advance(2))
                }
            }
            Some(b'\r' | b'\n') => {
                out.events
                    .push(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)));
                Some(Step::Advance(2))
            }
            Some(0x7f | 0x08) => {
                out.events.push(Event::Key(KeyEvent::new(
                    KeyCode::Backspace,
                    KeyModifiers::ALT,
                )));
                Some(Step::Advance(2))
            }
            Some(ch @ 0x20..=0x7e) => {
                let c = char::from(ch);
                let mods = if c.is_ascii_uppercase() {
                    KeyModifiers::ALT | KeyModifiers::SHIFT
                } else {
                    KeyModifiers::ALT
                };
                out.events
                    .push(Event::Key(KeyEvent::new(KeyCode::Char(c), mods)));
                Some(Step::Advance(2))
            }
            Some(_) => Some(Step::Advance(2)),
        }
    }
}

enum Step {
    Advance(usize),
    PasteStart(usize),
    Cancel,
}

fn complete_csi_len(rest: &[u8]) -> Option<usize> {
    for (i, &b) in rest.iter().enumerate().skip(2) {
        match b {
            0x20..=0x3f if i < 64 => {}
            0x40..=0x7e => return Some(i + 1),
            _ => return Some(i.max(2)),
        }
    }
    None
}

fn decode_csi(seq: &[u8]) -> CsiOutcome {
    if seq == PASTE_START {
        return CsiOutcome::PasteStart;
    }
    if seq == PASTE_END {
        return CsiOutcome::Ignore;
    }
    let Some(&final_byte) = seq.last() else {
        return CsiOutcome::Ignore;
    };
    let params = seq
        .get(2..seq.len().saturating_sub(1))
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or_default();
    let (first, second) = parse_two_params(params);
    let mods = xterm_modifiers(second);

    match final_byte {
        b'A'..=b'D' | b'F' | b'H' => arrow_or_home(final_byte).map_or(CsiOutcome::Ignore, |code| {
            CsiOutcome::Key(KeyEvent::new(code, mods))
        }),
        b'Z' => CsiOutcome::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        b'~' => match first {
            1 | 7 => CsiOutcome::Key(KeyEvent::new(KeyCode::Home, mods)),
            3 => CsiOutcome::Key(KeyEvent::new(KeyCode::Delete, mods)),
            4 | 8 => CsiOutcome::Key(KeyEvent::new(KeyCode::End, mods)),
            5 => CsiOutcome::Key(KeyEvent::new(KeyCode::PageUp, mods)),
            6 => CsiOutcome::Key(KeyEvent::new(KeyCode::PageDown, mods)),
            _ => CsiOutcome::Ignore,
        },
        b'u' => decode_csi_u(first, mods),
        _ => CsiOutcome::Ignore,
    }
}

fn decode_csi_u(codepoint: u32, mods: KeyModifiers) -> CsiOutcome {
    match codepoint {
        27 if mods.is_empty() => CsiOutcome::Cancel,
        99 if mods == KeyModifiers::CONTROL => CsiOutcome::Cancel,
        // Bare Enter is withheld (D91); modified Enter inserts a newline.
        13 if mods.is_empty() => CsiOutcome::Ignore,
        13 => CsiOutcome::Key(KeyEvent::new(KeyCode::Enter, mods)),
        9 => CsiOutcome::Key(KeyEvent::new(KeyCode::Tab, mods)),
        127 | 8 => CsiOutcome::Key(KeyEvent::new(KeyCode::Backspace, mods)),
        cp => char::from_u32(cp).map_or(CsiOutcome::Ignore, |ch| {
            if ch == 'v' && mods == KeyModifiers::CONTROL {
                CsiOutcome::Ignore
            } else {
                CsiOutcome::Key(KeyEvent::new(KeyCode::Char(ch), mods))
            }
        }),
    }
}

fn arrow_or_home(final_byte: u8) -> Option<KeyCode> {
    match final_byte {
        b'A' => Some(KeyCode::Up),
        b'B' => Some(KeyCode::Down),
        b'C' => Some(KeyCode::Right),
        b'D' => Some(KeyCode::Left),
        b'H' => Some(KeyCode::Home),
        b'F' => Some(KeyCode::End),
        _ => None,
    }
}

fn parse_two_params(params: &str) -> (u32, u32) {
    let mut iter = params.split(';');
    let first = iter
        .next()
        .and_then(|p| p.split(':').next())
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(1);
    let second = iter
        .next()
        .and_then(|p| p.split(':').next())
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(1);
    (first, second)
}

fn xterm_modifiers(param: u32) -> KeyModifiers {
    let bits = param.saturating_sub(1);
    let mut mods = KeyModifiers::NONE;
    if bits & 1 != 0 {
        mods |= KeyModifiers::SHIFT;
    }
    if bits & 2 != 0 {
        mods |= KeyModifiers::ALT;
    }
    if bits & 4 != 0 {
        mods |= KeyModifiers::CONTROL;
    }
    mods
}

/// Returns `Some((Some(ch), len))` for a valid UTF-8 codepoint, `Some((None, len))`
/// to skip an invalid byte sequence, or `None` when trailing bytes are an
/// incomplete multi-byte UTF-8 sequence.
fn decode_utf8_char(rest: &[u8]) -> Option<(Option<char>, usize)> {
    let window = rest.get(..rest.len().min(4)).unwrap_or_default();
    match std::str::from_utf8(window) {
        Ok(s) => s.chars().next().map(|ch| (Some(ch), ch.len_utf8())),
        Err(err) if err.valid_up_to() > 0 => {
            std::str::from_utf8(window.get(..err.valid_up_to()).unwrap_or_default())
                .ok()
                .and_then(|s| s.chars().next())
                .map(|ch| (Some(ch), ch.len_utf8()))
        }
        Err(err) => err.error_len().map(|bad| (None, bad.max(1))),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn trailing_prefix_len(haystack: &[u8], needle: &[u8]) -> usize {
    let max_k = haystack.len().min(needle.len().saturating_sub(1));
    for k in (1..=max_k).rev() {
        if haystack.get(haystack.len() - k..) == needle.get(..k) {
            return k;
        }
    }
    0
}
