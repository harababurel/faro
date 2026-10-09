//! OSC 5522 reply parsing, shared by both approaches.
//!
//! A reply is `ESC ] 5522 ; type=read:status=OK ESC \`, then
//! `ESC ] 5522 ; type=read:status=DATA:mime=<b64> ; <b64 chunk> ESC \` packets,
//! then `ESC ] 5522 ; type=read:status=DONE ESC \` (or an error status instead
//! of `OK`).

use base64::{Engine, engine::general_purpose::STANDARD};

const START: &[u8] = b"\x1b]5522;";

/// One packet's contents.
#[derive(Debug)]
pub enum Packet {
    Status(String),
    Data { mime: String, bytes: Vec<u8> },
    Corrupt(String),
}

/// Splits a byte stream into packets; everything outside packets is `stray`.
#[derive(Default)]
pub struct Scanner {
    buf: Vec<u8>,
    pub stray: Vec<u8>,
    /// Escape sequences found inside packets (e.g. a focus report tmux slipped
    /// into the middle of the reply), cut out before decoding.
    pub interleaved: Vec<Vec<u8>>,
}

impl Scanner {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Packet> {
        self.buf.extend_from_slice(bytes);
        let mut packets = Vec::new();
        loop {
            let Some(start) = find(&self.buf, START) else {
                // Keep a tail that could be the start of a packet.
                let keep = (1..START.len())
                    .rev()
                    .find(|&n| self.buf.len() >= n && self.buf.ends_with(&START[..n]))
                    .unwrap_or(0);
                let cut = self.buf.len() - keep;
                self.stray.extend(self.buf.drain(..cut));
                break;
            };
            self.stray.extend(self.buf.drain(..start));
            let body_start = START.len();
            let Some((end, terminator)) = find_terminator(&self.buf[body_start..]) else {
                break;
            };
            let body = strip_csi(
                &self.buf[body_start..body_start + end],
                &mut self.interleaved,
            );
            self.buf.drain(..body_start + end + terminator);
            packets.push(parse(&body));
        }
        packets
    }
}

/// Packet bodies never contain `ESC` (base64 and `key=value` text), so any
/// `ESC [ … final` inside one is another sequence that got mixed in: move it to
/// `cut`. A lone `ESC` that doesn't start a CSI is left for the parser to reject.
fn strip_csi(body: &[u8], cut: &mut Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if body[i] == 0x1b && body.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < body.len() && (0x20..=0x3f).contains(&body[j]) {
                j += 1;
            }
            if j < body.len() && (0x40..=0x7e).contains(&body[j]) {
                cut.push(body[i..=j].to_vec());
                i = j + 1;
                continue;
            }
        }
        out.push(body[i]);
        i += 1;
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Offset of `ESC \` or `BEL`, and the terminator's length.
fn find_terminator(body: &[u8]) -> Option<(usize, usize)> {
    body.iter().enumerate().find_map(|(i, &b)| match b {
        0x07 => Some((i, 1)),
        0x1b if body.get(i + 1) == Some(&b'\\') => Some((i, 2)),
        _ => None,
    })
}

fn parse(body: &[u8]) -> Packet {
    let Ok(text) = std::str::from_utf8(body) else {
        return Packet::Corrupt("not UTF-8".into());
    };
    let (meta, payload) = text.split_once(';').unwrap_or((text, ""));
    let mut status = None;
    let mut mime = None;
    for pair in meta.split(':') {
        match pair.split_once('=') {
            Some(("status", value)) => status = Some(value.to_owned()),
            Some(("mime", value)) => mime = Some(value.to_owned()),
            _ => {}
        }
    }
    let Some(status) = status else {
        return Packet::Corrupt(format!("no status in {meta:?}"));
    };
    if status != "DATA" {
        return Packet::Status(status);
    }
    let mime = match mime.map(|m| STANDARD.decode(m)) {
        Some(Ok(m)) => String::from_utf8_lossy(&m).into_owned(),
        Some(Err(error)) => return Packet::Corrupt(format!("mime: {error}")),
        None => String::new(),
    };
    match STANDARD.decode(payload) {
        Ok(bytes) => Packet::Data { mime, bytes },
        Err(error) => Packet::Corrupt(format!("payload ({} chars): {error}", payload.len())),
    }
}

/// A reply assembled from packets.
#[derive(Default, Debug)]
pub struct Reply {
    pub ok: bool,
    pub done: bool,
    pub error: Option<String>,
    pub timed_out: bool,
    pub too_large: bool,
    /// `(mime, bytes)` per type, in arrival order.
    pub data: Vec<(String, Vec<u8>)>,
    pub packets: usize,
    pub corrupt: usize,
}

impl Reply {
    pub fn absorb(&mut self, packet: Packet, cap: usize) {
        self.packets += 1;
        match packet {
            Packet::Status(status) => match status.as_str() {
                "OK" => self.ok = true,
                "DONE" => self.done = true,
                _ => self.error = Some(status),
            },
            Packet::Data { mime, bytes } => {
                let total: usize = self.data.iter().map(|(_, b)| b.len()).sum();
                if total + bytes.len() > cap {
                    self.too_large = true;
                    return;
                }
                match self.data.last_mut() {
                    Some((last, data)) if *last == mime => data.extend(bytes),
                    _ => self.data.push((mime, bytes)),
                }
            }
            Packet::Corrupt(_) => self.corrupt += 1,
        }
    }

    /// Nothing more will come: `DONE`, or an error instead of `OK`.
    pub fn finished(&self) -> bool {
        self.done || self.error.is_some()
    }

    /// The type list, when this is the reply to a `.` request.
    pub fn types(&self) -> Vec<String> {
        self.data
            .iter()
            .filter(|(mime, _)| mime == ".")
            .flat_map(|(_, bytes)| {
                String::from_utf8_lossy(bytes)
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn status(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        let mut status = String::from(match (self.ok, self.done) {
            (true, true) => "OK",
            (true, false) => "INCOMPLETE",
            (false, true) => "DONE-WITHOUT-OK",
            (false, false) => "NO-REPLY",
        });
        if self.timed_out {
            status.push_str("+TIMEOUT");
        }
        if self.too_large {
            status.push_str("+TOO-LARGE");
        }
        status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact reply captured in the owner's test 4 (list types through tmux).
    const TEST_4: &[u8] = b"\x1b]5522;type=read:status=OK\x1b\\\
        \x1b]5522;type=read:status=DATA:mime=Lg==;aW1hZ2UvcG5nIHRleHQvaHRtbAo=\x1b\\\
        \x1b]5522;type=read:status=DONE\x1b\\";

    #[test]
    fn parses_the_captured_type_list_split_anywhere() {
        for split in 0..=TEST_4.len() {
            let mut scanner = Scanner::default();
            let mut reply = Reply::default();
            for chunk in [&TEST_4[..split], &TEST_4[split..]] {
                for packet in scanner.push(chunk) {
                    reply.absorb(packet, usize::MAX);
                }
            }
            assert!(reply.finished(), "split {split}");
            assert_eq!(reply.status(), "OK");
            assert_eq!(reply.types(), ["image/png", "text/html"]);
            assert_eq!(scanner.stray, b"");
        }
    }

    /// Seen on the owner's machine: tmux put kitty's focus-in report inside
    /// a data packet.
    #[test]
    fn cuts_focus_reports_out_of_packets() {
        let input = b"\x1b]5522;type=read:status=DATA:mime=aW1hZ2UvcG5n;aGVs\x1b[IbG8=\x1b\\";
        let mut scanner = Scanner::default();
        let packets = scanner.push(input);
        let [Packet::Data { bytes, .. }] = packets.as_slice() else {
            panic!("{packets:?}")
        };
        assert_eq!(bytes, b"hello");
        assert_eq!(scanner.interleaved, [b"\x1b[I".to_vec()]);
    }

    #[test]
    fn keeps_stray_bytes_around_packets() {
        let mut scanner = Scanner::default();
        let mut input = b"ab".to_vec();
        input.extend_from_slice(TEST_4);
        input.extend_from_slice(b"cd");
        let packets = scanner.push(&input);
        assert_eq!(packets.len(), 3);
        assert_eq!(scanner.stray, b"abcd");
    }
}
