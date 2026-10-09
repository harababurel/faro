use std::collections::VecDeque;
use std::io;
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use faro_core::clipboard::{LIST, ReadError};

use super::guard::{Filtered, LateGuard};
use super::tty::{self, Failure, Source};
use super::*;

/// Plays back bytes as a terminal would; `None` (the deadline) once out.
struct Script(VecDeque<Vec<u8>>);

impl Script {
    fn new(chunks: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self(chunks.into_iter().collect())
    }
}

impl Source for Script {
    fn read(&mut self, _deadline: Instant) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.pop_front())
    }
}

fn packet(id: Option<&str>, meta: &str, payload: Option<&[u8]>) -> Vec<u8> {
    let id = id.map(|id| format!("id={id}:")).unwrap_or_default();
    let mut out = format!("\x1b]5522;type=read:{id}{meta}").into_bytes();
    if let Some(payload) = payload {
        out.push(b';');
        out.extend(STANDARD.encode(payload).bytes());
    }
    out.extend_from_slice(b"\x1b\\");
    out
}

fn reply(id: Option<&str>, mime: &str, data: &[u8]) -> Vec<u8> {
    let meta = format!("status=DATA:mime={}", STANDARD.encode(mime));
    let mut out = packet(id, "status=OK", None);
    for chunk in data.chunks(4096) {
        out.extend(packet(id, &meta, Some(chunk)));
    }
    out.extend(packet(id, "status=DONE", None));
    out
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

/// The key events crossterm makes from a reply's bytes (D90).
fn as_keys(bytes: &[u8]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut iter = bytes.iter();
    while let Some(&b) = iter.next() {
        if b == 0x1b {
            let next = *iter.next().expect("ESC is followed by a byte");
            out.push(key(KeyCode::Char(next as char), KeyModifiers::ALT));
        } else {
            let c = b as char;
            let shift = if c.is_ascii_uppercase() {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            out.push(key(KeyCode::Char(c), shift));
        }
    }
    out
}

/// Rule 9 (D27): a recorded reply fed through the input path produces no key
/// events for the reducer; it asks for a drain instead.
#[test]
fn a_late_reply_never_reaches_the_reducer() {
    let recorded = b"\x1b]5522;type=read:status=OK\x1b\\\
        \x1b]5522;type=read:status=DATA:mime=Lg==;aW1hZ2UvcG5nIHRleHQvaHRtbAo=\x1b\\";
    for esc_split in [false, true] {
        let mut guard = LateGuard::default();
        guard.arm();
        let mut keys = as_keys(recorded);
        if esc_split {
            // A read ended right after the ESC: `Esc`, then a plain `]`.
            keys.splice(
                0..1,
                [
                    key(KeyCode::Esc, KeyModifiers::NONE),
                    key(KeyCode::Char(']'), KeyModifiers::NONE),
                ],
            );
        }
        let mut passed = Vec::new();
        let mut drained = false;
        for event in keys {
            match guard.filter(event) {
                Filtered::Pass(events) => passed.extend(events),
                Filtered::Drain => {
                    drained = true;
                    break;
                }
            }
        }
        assert!(drained, "esc_split {esc_split}");
        assert_eq!(passed, vec![], "esc_split {esc_split}");
    }
}

#[test]
fn the_guard_gives_other_keys_back_in_order() {
    let mut guard = LateGuard::default();
    // Not armed: nothing is ever held, not even Alt+].
    let alt = key(KeyCode::Char(']'), KeyModifiers::ALT);
    assert_eq!(guard.filter(alt.clone()), Filtered::Pass(vec![alt.clone()]));
    guard.arm();
    let esc = key(KeyCode::Esc, KeyModifiers::NONE);
    let five = key(KeyCode::Char('5'), KeyModifiers::NONE);
    let x = key(KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(guard.filter(x.clone()), Filtered::Pass(vec![x.clone()]));
    // Alt+] 5 x: held, held, then all three back.
    assert_eq!(guard.filter(alt.clone()), Filtered::Pass(vec![]));
    assert!(guard.deadline().is_some());
    assert_eq!(guard.filter(five.clone()), Filtered::Pass(vec![]));
    assert_eq!(
        guard.filter(x.clone()),
        Filtered::Pass(vec![alt.clone(), five, x.clone()])
    );
    assert_eq!(guard.deadline(), None);
    // Esc Esc: the first comes back, the second is held until the deadline.
    assert_eq!(guard.filter(esc.clone()), Filtered::Pass(vec![]));
    assert_eq!(guard.filter(esc.clone()), Filtered::Pass(vec![esc.clone()]));
    assert_eq!(guard.flush(), vec![esc]);
    // Focus and mouse events are never held.
    assert_eq!(
        guard.filter(Event::FocusGained),
        Filtered::Pass(vec![Event::FocusGained])
    );
}

#[test]
fn lists_types_from_the_first_reply_with_an_image() {
    let id = "faro-1";
    let mut script = Script::new([
        b"\x1b[O".to_vec(),
        reply(Some(id), LIST, b"image/png text/html"),
    ]);
    let listed = tty::list(&mut script, id);
    assert_eq!(listed.focus, [Focus::Lost]);
    assert_eq!(listed.result.expect("a list"), ["image/png", "text/html"]);
}

/// D91: two kitty windows attached; the one answering first has only text.
#[test]
fn waits_for_another_kitty_when_the_first_list_has_no_image() {
    let id = "faro-1";
    let mut both = reply(Some(id), LIST, b"text/plain");
    both.extend(reply(Some(id), LIST, b"image/png"));
    let listed = tty::list(&mut Script::new([both]), id);
    assert_eq!(listed.result.expect("a list"), ["text/plain", "image/png"]);
    // The second reply in a later read.
    let mut script = Script::new([
        reply(Some(id), LIST, b"text/plain"),
        reply(Some(id), LIST, b"image/png"),
    ]);
    let listed = tty::list(&mut script, id);
    assert_eq!(
        protocol::best_image_type(&listed.result.expect("a list")),
        Some("image/png")
    );
}

#[test]
fn a_list_with_no_answer_or_only_errors_fails() {
    let id = "faro-1";
    let listed = tty::list(&mut Script::new([]), id);
    assert!(matches!(
        listed.result,
        Err(Failure::Read(ReadError::NoReply))
    ));
    let listed = tty::list(
        &mut Script::new([packet(Some(id), "status=EBUSY", None)]),
        id,
    );
    assert!(matches!(listed.result, Err(Failure::Read(ReadError::Busy))));
    // A late answer to an older request is ignored.
    let listed = tty::list(
        &mut Script::new([reply(Some("faro-0"), LIST, b"image/png")]),
        id,
    );
    assert!(matches!(
        listed.result,
        Err(Failure::Read(ReadError::NoReply))
    ));
}

#[test]
fn reads_image_data_with_focus_reports_around_it() {
    let id = "faro-2";
    let image: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    let body = reply(Some(id), "image/png", &image);
    let (first, second) = body.split_at(body.len() / 2);
    let mut script = Script::new([
        b"\x1b[O".to_vec(),
        first.to_vec(),
        b"\x1b[I".to_vec(),
        second.to_vec(),
    ]);
    let read = tty::data(&mut script, id, MAX_IMAGE);
    assert_eq!(read.focus, [Focus::Lost, Focus::Gained]);
    let entry = read.result.expect("data").take("image/png").expect("png");
    assert_eq!(entry.data, image);
}

#[test]
fn data_errors_come_through() {
    let id = "faro-2";
    let read = tty::data(
        &mut Script::new([packet(Some(id), "status=EPERM", None)]),
        id,
        MAX_IMAGE,
    );
    assert!(matches!(read.result, Err(Failure::Read(ReadError::Denied))));
    let read = tty::data(&mut Script::new([]), id, MAX_IMAGE);
    assert!(matches!(
        read.result,
        Err(Failure::Read(ReadError::NoReply))
    ));
    let read = tty::data(
        &mut Script::new([reply(Some(id), "image/png", &[0; 100])]),
        id,
        10,
    );
    assert!(matches!(
        read.result,
        Err(Failure::Read(ReadError::TooLarge { limit: 10 }))
    ));
}

#[test]
fn draining_reads_to_the_end_of_a_reply_that_started_mid_packet() {
    let body = reply(None, "image/png", &[7; 9000]);
    // crossterm already took the start of the reply.
    let rest = body[3000..].to_vec();
    let (a, b) = rest.split_at(2000);
    let drained = tty::drain_source(&mut Script::new([a.to_vec(), b.to_vec()])).expect("drained");
    assert!(drained.done);
    assert_eq!(drained.bytes, rest.len());
}

#[test]
fn notices_say_what_went_wrong() {
    assert_eq!(
        ClipboardError::NotKitty.notice().text,
        "Pasting images needs kitty"
    );
    assert_eq!(
        ClipboardError::Read(ReadError::Denied).notice().text,
        "Clipboard access denied in kitty"
    );
    assert_eq!(
        ClipboardError::Read(ReadError::TooLarge { limit: MAX_IMAGE })
            .notice()
            .text,
        "Image too large (over 25.0 MB)"
    );
}

#[test]
#[expect(clippy::disallowed_methods, reason = "tests may block")]
fn saves_the_image_private_with_its_size() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let mut png = Vec::new();
    image::RgbaImage::new(3, 2)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode");
    let first = save(dir.path(), "image/png", &png).expect("saved");
    assert!(first.name.starts_with("clipboard-"), "{}", first.name);
    assert_eq!(first.path.extension(), Some("png".as_ref()));
    assert_eq!((first.width, first.height), (Some(3), Some(2)));
    assert_eq!(first.size, png.len() as u64);
    assert_eq!(first.content_type, "image/png");
    let mode = std::fs::metadata(&first.path)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(std::fs::read(&first.path).expect("read"), png);
    // A second paste in the same second gets its own file.
    let second = save(dir.path(), "image/png", &png).expect("saved");
    assert_ne!(second.path, first.path);
}

#[test]
#[expect(clippy::disallowed_methods, reason = "tests may block")]
fn pasted_images_are_recognised_and_swept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pasted = dir.path().join("clipboard-20261009-142233.png");
    let received = dir.path().join("1760000000000-abcdef01-0-photo.png");
    let elsewhere = dir.path().join("sub").join("clipboard-x.png");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    for path in [&pasted, &received, &elsewhere] {
        std::fs::write(path, b"x").expect("write");
    }
    assert!(is_pasted(&pasted, dir.path()));
    assert!(!is_pasted(&received, dir.path()));
    assert!(
        !is_pasted(&elsewhere, dir.path()),
        "only directly in attachments/"
    );
    assert_eq!(sweep_blocking(dir.path()), 1);
    assert!(!pasted.exists());
    assert!(received.exists() && elsewhere.exists());
}
