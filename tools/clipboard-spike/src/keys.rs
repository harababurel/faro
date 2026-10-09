//! Approach B: rebuild an OSC 5522 reply from crossterm's key events.
//!
//! crossterm 0.29 parses `ESC ]` as `Alt+]`, each following byte as a `Char`
//! (uppercase letters get `SHIFT`), and `ESC \` as `Alt+\`. When a read ends
//! right after an `ESC`, the `ESC` comes out as a plain `Esc` key and the next
//! byte as a plain `Char`, so both forms are accepted.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    #[default]
    Idle,
    /// A plain `Esc` held back: it may start a packet.
    Esc,
    Packet,
    /// A plain `Esc` inside a packet: it may end it.
    PacketEsc,
}

#[derive(Default)]
pub struct KeyCollector {
    state: State,
    held: Option<Event>,
    bytes: Vec<u8>,
    consumed: usize,
}

/// What one event turned into.
pub struct Fed {
    /// Events for the app (not part of a reply).
    pub passthrough: Vec<Event>,
    /// A complete packet, `ESC ] … ESC \`.
    pub packet: Option<Vec<u8>>,
}

fn plain_char(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c)
            if c.is_ascii_graphic()
                && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
        {
            Some(c)
        }
        _ => None,
    }
}

fn alt_char(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if key.modifiers == KeyModifiers::ALT => Some(c),
        _ => None,
    }
}

fn is_plain_esc(key: &KeyEvent) -> bool {
    key.code == KeyCode::Esc && key.modifiers.is_empty()
}

impl KeyCollector {
    /// Inside a packet (a reply has started arriving as keys).
    pub fn in_packet(&self) -> bool {
        matches!(self.state, State::Packet | State::PacketEsc)
    }

    /// Forget any partial packet (after the rest was drained from the tty).
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.held = None;
        self.bytes.clear();
    }

    /// Events swallowed since the last call.
    pub fn take_consumed(&mut self) -> usize {
        std::mem::take(&mut self.consumed)
    }

    pub fn feed(&mut self, event: Event) -> Fed {
        let mut fed = Fed {
            passthrough: Vec::new(),
            packet: None,
        };
        self.step(event, &mut fed);
        fed
    }

    fn step(&mut self, event: Event, fed: &mut Fed) {
        let Event::Key(key) = &event else {
            // Focus, mouse, resize, paste: never part of a reply.
            fed.passthrough.push(event);
            return;
        };
        match self.state {
            State::Idle => {
                if alt_char(key) == Some(']') {
                    self.begin();
                } else if is_plain_esc(key) {
                    self.state = State::Esc;
                    self.held = Some(event);
                } else {
                    fed.passthrough.push(event);
                }
            }
            State::Esc => {
                if plain_char(key) == Some(']') {
                    self.held = None;
                    self.consumed += 1;
                    self.begin();
                } else {
                    // Not a packet after all: release the Esc, then look again.
                    self.state = State::Idle;
                    fed.passthrough.extend(self.held.take());
                    self.step(event, fed);
                }
            }
            State::Packet => {
                if let Some(c) = plain_char(key) {
                    self.consumed += 1;
                    self.bytes.push(c as u8);
                } else if alt_char(key) == Some('\\') {
                    self.consumed += 1;
                    self.end(fed);
                } else if is_plain_esc(key) {
                    self.consumed += 1;
                    self.state = State::PacketEsc;
                } else {
                    self.abort(event, fed);
                }
            }
            State::PacketEsc => {
                if plain_char(key) == Some('\\') {
                    self.consumed += 1;
                    self.end(fed);
                } else {
                    self.abort(event, fed);
                }
            }
        }
    }

    fn begin(&mut self) {
        self.consumed += 1;
        self.state = State::Packet;
        self.bytes.clear();
        self.bytes.extend_from_slice(b"\x1b]");
    }

    fn end(&mut self, fed: &mut Fed) {
        self.state = State::Idle;
        let mut packet = std::mem::take(&mut self.bytes);
        packet.extend_from_slice(b"\x1b\\");
        fed.packet = Some(packet);
    }

    /// Something that can't be in a reply arrived mid-packet: hand the partial
    /// packet over anyway (the parser marks it corrupt) and pass the key on.
    fn abort(&mut self, event: Event, fed: &mut Fed) {
        self.end(fed);
        fed.passthrough.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    /// Key events crossterm makes from `bytes` (only what replies contain).
    fn events(bytes: &[u8], esc_split: bool) -> Vec<Event> {
        let mut out = Vec::new();
        let mut iter = bytes.iter().peekable();
        while let Some(&b) = iter.next() {
            if b == 0x1b {
                let next = *iter.next().expect("ESC is followed by a byte");
                if esc_split {
                    out.push(key(KeyCode::Esc, KeyModifiers::NONE));
                    out.push(key(KeyCode::Char(next as char), KeyModifiers::NONE));
                } else {
                    out.push(key(KeyCode::Char(next as char), KeyModifiers::ALT));
                }
            } else {
                let c = b as char;
                let m = if c.is_uppercase() {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                };
                out.push(key(KeyCode::Char(c), m));
            }
        }
        out
    }

    #[test]
    fn rebuilds_packets_from_both_esc_forms() {
        let packet =
            b"\x1b]5522;type=read:status=DATA:mime=Lg==;aW1hZ2UvcG5nIHRleHQvaHRtbAo=\x1b\\";
        for esc_split in [false, true] {
            let mut collector = KeyCollector::default();
            let mut got = None;
            let mut passthrough = 0;
            for event in events(packet, esc_split) {
                let fed = collector.feed(event);
                passthrough += fed.passthrough.len();
                got = got.or(fed.packet);
            }
            assert_eq!(got.as_deref(), Some(&packet[..]), "esc_split {esc_split}");
            assert_eq!(passthrough, 0);
        }
    }

    #[test]
    fn passes_other_keys_and_a_lone_esc_through() {
        let mut collector = KeyCollector::default();
        let mut out = Vec::new();
        for event in [
            key(KeyCode::Char('x'), KeyModifiers::NONE),
            key(KeyCode::Esc, KeyModifiers::NONE),
            key(KeyCode::Char('y'), KeyModifiers::NONE),
        ] {
            out.extend(collector.feed(event).passthrough);
        }
        assert_eq!(out.len(), 3);
    }
}
