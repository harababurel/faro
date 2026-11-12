use std::collections::VecDeque;
use std::io;
use std::time::Instant;

use crate::app::term_event::{Event, KeyCode, KeyEvent, KeyModifiers};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
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

/// D150: Ghostty on macOS says yes; one round trip either way.
#[test]
fn asks_whether_the_terminal_knows_the_protocol() {
    let mut yes = Script::new([b"\x1b[?5522;2$y".to_vec(), b"\x1b[0n".to_vec()]);
    let asked = tty::support(&mut yes, false);
    assert_eq!(
        asked.result.expect("an answer"),
        tty::Support {
            answered: true,
            supported: true
        }
    );
    // Ghostty 1.3.1 or on Linux: the mode is unknown.
    let mut no = Script::new([b"\x1b[?5522;0$y\x1b[0n".to_vec()]);
    let asked = tty::support(&mut no, false).result.expect("an answer");
    assert!(asked.answered && !asked.supported);
    // A terminal that ignores the question still answers the status report,
    // with a focus report on the way.
    let mut silent = Script::new([b"\x1b[I\x1b[0n".to_vec()]);
    let asked = tty::support(&mut silent, false);
    assert_eq!(asked.focus, [Focus::Gained]);
    assert!(!asked.result.expect("an answer").supported);
    // Nothing at all (tmux without allow-passthrough).
    let asked = tty::support(&mut Script::new([]), false)
        .result
        .expect("ok");
    assert!(!asked.answered);
}

/// Under tmux with several clients, one that says yes after another said no
/// is still heard; with one client faro stops at the first status report.
#[test]
fn several_tmux_clients_get_a_moment_to_say_yes() {
    let chunks = || {
        [
            b"\x1b[?5522;0$y\x1b[0n".to_vec(),
            b"\x1b[?5522;2$y\x1b[0n".to_vec(),
        ]
    };
    let several = tty::support(&mut Script::new(chunks()), true);
    assert!(several.result.expect("an answer").supported);
    let one = tty::support(&mut Script::new(chunks()), false);
    assert!(!one.result.expect("an answer").supported);
}

/// D150: a TIFF becomes a PNG of the same size before staging; other types
/// are kept as they are.
#[test]
fn tiff_is_converted_to_png() {
    let mut tiff = Vec::new();
    image::RgbImage::from_pixel(4, 3, image::Rgb([200, 10, 10]))
        .write_to(
            &mut std::io::Cursor::new(&mut tiff),
            image::ImageFormat::Tiff,
        )
        .expect("encode");
    let (mime, png) = sendable("image/tiff", tiff).expect("converted");
    assert_eq!(mime, "image/png");
    assert_eq!(
        image::guess_format(&png).expect("a format"),
        image::ImageFormat::Png
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let staged = stage(dir.path(), "image/tiff", {
        let mut again = Vec::new();
        image::RgbImage::new(4, 3)
            .write_to(
                &mut std::io::Cursor::new(&mut again),
                image::ImageFormat::Tiff,
            )
            .expect("encode");
        again
    })
    .expect("staged");
    assert_eq!(staged.content_type, "image/png");
    assert_eq!(staged.path.extension(), Some("png".as_ref()));
    assert_eq!((staged.width, staged.height), (Some(4), Some(3)));
    // Not a TIFF after all: an error, not a panic.
    assert!(matches!(
        sendable("image/tiff", b"nope".to_vec()),
        Err(ClipboardError::Failed(_))
    ));
    // Other types pass through untouched.
    assert_eq!(
        sendable("image/png", vec![1, 2, 3]).expect("kept"),
        ("image/png", vec![1, 2, 3])
    );
    assert_eq!(limit_for("image/tiff"), MAX_TIFF);
    assert_eq!(limit_for("image/png"), MAX_IMAGE);
}

#[test]
fn notices_say_what_went_wrong() {
    assert_eq!(
        ClipboardError::OverSsh.notice().text,
        "Pasting images over SSH needs kitty or a newer Ghostty"
    );
    assert_eq!(
        ClipboardError::Read(ReadError::Denied).notice().text,
        "Clipboard access denied"
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

#[derive(Default)]
struct RecordingObserver {
    inputs: Vec<Event>,
    progress: Vec<u64>,
    midstream_cancels: usize,
}

impl tty::Observer for RecordingObserver {
    fn on_input(&mut self, event: Event) {
        self.inputs.push(event);
    }

    fn on_progress(&mut self, bytes: u64) {
        self.progress.push(bytes);
    }

    fn on_midstream_cancel(&mut self) {
        self.midstream_cancels += 1;
    }
}

#[test]
fn key_decoder_streams_utf8_editing_keys_pastes_and_withholds_bare_enter() {
    let mut decoder = super::keys::KeyDecoder::default();

    // Printable ASCII, Backspace, Tab, Ctrl+J, Alt+Enter, Shift+Enter, arrows, Delete.
    let batch =
        decoder.feed(b"Hi\x7f\t\n\x1b\r\x1b[13;2u\x1b[A\x1bOH\x1b[3~\r\x1b[13u\x16\x1b[118;5u");
    assert!(!batch.cancelled);
    assert_eq!(
        batch.events,
        vec![
            key(KeyCode::Char('H'), KeyModifiers::SHIFT),
            key(KeyCode::Char('i'), KeyModifiers::NONE),
            key(KeyCode::Backspace, KeyModifiers::NONE),
            key(KeyCode::Tab, KeyModifiers::NONE),
            key(KeyCode::Char('j'), KeyModifiers::CONTROL),
            key(KeyCode::Enter, KeyModifiers::ALT),
            key(KeyCode::Enter, KeyModifiers::SHIFT),
            key(KeyCode::Up, KeyModifiers::NONE),
            key(KeyCode::Home, KeyModifiers::NONE),
            key(KeyCode::Delete, KeyModifiers::NONE),
        ]
    );

    // Multi-byte UTF-8 fed one byte at a time.
    let mut utf8_events = Vec::new();
    for &b in "é🧗".as_bytes() {
        let step = decoder.feed(&[b]);
        assert!(!step.cancelled);
        utf8_events.extend(step.events);
    }
    assert_eq!(
        utf8_events,
        vec![
            key(KeyCode::Char('é'), KeyModifiers::NONE),
            key(KeyCode::Char('🧗'), KeyModifiers::NONE),
        ]
    );

    // Bracketed paste split mid-terminator across two reads.
    let first = decoder.feed(b"\x1b[200~hello\r\nworld\x1b[20");
    assert_eq!(first.events, vec![]);
    let second = decoder.feed(b"1~!");
    assert_eq!(
        second.events,
        vec![
            Event::Paste("hello\r\nworld".to_owned()),
            key(KeyCode::Char('!'), KeyModifiers::NONE),
        ]
    );

    // Keys before Ctrl+C / Esc are preserved when cancelling.
    for cancel_seq in [
        &b"ok\x03ignored"[..],
        &b"ok\x1b"[..],
        &b"ok\x1b[27u"[..],
        &b"ok\x1b[99;5u"[..],
    ] {
        let mut dec = super::keys::KeyDecoder::default();
        let res = dec.feed(cancel_seq);
        assert!(res.cancelled, "expected cancel for {cancel_seq:?}");
        assert_eq!(
            res.events,
            vec![
                key(KeyCode::Char('o'), KeyModifiers::NONE),
                key(KeyCode::Char('k'), KeyModifiers::NONE),
            ]
        );
    }
}

#[test]
fn list_and_data_forward_live_keys_and_progress_across_phases() {
    let list_id = "faro-list";
    let data_id = "faro-data";
    let mut list_chunk = b"hi".to_vec();
    list_chunk.extend(reply(Some(list_id), LIST, b"image/png"));
    list_chunk.extend_from_slice(b" ");

    let mut observer = RecordingObserver::default();
    let listed = tty::list_with(&mut Script::new([list_chunk]), list_id, &mut observer)
        .result
        .expect("listed");
    assert_eq!(listed.types, ["image/png"]);

    let image: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    let meta = format!("status=DATA:mime={}", STANDARD.encode("image/png"));
    let mut chunks = vec![packet(Some(data_id), "status=OK", None)];
    chunks.push(packet(Some(data_id), &meta, Some(&image[..5000])));
    chunks.push("🧗".as_bytes().to_vec());
    let mut last = packet(Some(data_id), &meta, Some(&image[5000..]));
    last.extend(packet(Some(data_id), "status=DONE", None));
    last.extend_from_slice(b"!");
    chunks.push(last);

    let read = tty::data_with(
        &mut Script::new(chunks),
        data_id,
        MAX_IMAGE,
        &listed.rest,
        &mut observer,
    );
    let entry = read.result.expect("data").take("image/png").expect("png");
    assert_eq!(entry.data, image);
    assert_eq!(observer.progress.last().copied(), Some(10_000));
    assert_eq!(
        observer.inputs,
        vec![
            key(KeyCode::Char('h'), KeyModifiers::NONE),
            key(KeyCode::Char('i'), KeyModifiers::NONE),
            key(KeyCode::Char(' '), KeyModifiers::NONE),
            key(KeyCode::Char('🧗'), KeyModifiers::NONE),
            key(KeyCode::Char('!'), KeyModifiers::NONE),
        ]
    );
}

#[test]
fn esc_cancels_pre_stream_immediately_and_drains_cleanly_mid_stream() {
    let id = "faro-cancel";

    // Pre-stream standalone Esc: flushed when no continuation arrives.
    let mut pre_obs = RecordingObserver::default();
    let pre = tty::data_with(
        &mut Script::new([vec![0x1b]]),
        id,
        MAX_IMAGE,
        &[],
        &mut pre_obs,
    );
    assert!(matches!(
        pre.result,
        Err(Failure::Cancelled { drained: false })
    ));
    assert_eq!(pre_obs.midstream_cancels, 0);

    // Mid-stream Ctrl+C: notifies immediately, discards data, and drains to DONE.
    let meta = format!("status=DATA:mime={}", STANDARD.encode("image/png"));
    let chunks = vec![
        packet(Some(id), "status=OK", None),
        packet(Some(id), &meta, Some(&[1; 4096])),
        b"a\x03".to_vec(),
        packet(Some(id), &meta, Some(&[2; 4096])),
        packet(Some(id), "status=DONE", None),
    ];
    let mut mid_obs = RecordingObserver::default();
    let mid = tty::data_with(&mut Script::new(chunks), id, MAX_IMAGE, &[], &mut mid_obs);
    assert!(matches!(
        mid.result,
        Err(Failure::Cancelled { drained: true })
    ));
    assert_eq!(mid_obs.midstream_cancels, 1);
    assert_eq!(
        mid_obs.inputs,
        vec![key(KeyCode::Char('a'), KeyModifiers::NONE)]
    );
}

#[test]
#[expect(clippy::disallowed_methods, reason = "tests may block")]
fn stage_cancellable_leaves_no_orphaned_file_when_cancelled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut png = Vec::new();
    image::RgbaImage::new(2, 2)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode");

    let cancelled = AtomicBool::new(true);
    let err = stage_cancellable(dir.path(), "image/png", png.clone(), &cancelled);
    assert!(matches!(err, Err(ClipboardError::Cancelled)));
    assert_eq!(std::fs::read_dir(dir.path()).expect("readdir").count(), 0);

    let active = AtomicBool::new(false);
    let staged = stage_cancellable(dir.path(), "image/png", png, &active).expect("staged");
    assert!(staged.path.exists());
    remove_file_blocking(&staged.path);
    assert!(!staged.path.exists());
}
