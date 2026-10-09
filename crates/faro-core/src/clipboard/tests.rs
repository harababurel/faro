use super::*;

/// The exact reply captured in the owner's test 4 (type list, through tmux).
const TEST_4: &[u8] = b"\x1b]5522;type=read:status=OK\x1b\\\
    \x1b]5522;type=read:status=DATA:mime=Lg==;aW1hZ2UvcG5nIHRleHQvaHRtbAo=\x1b\\\
    \x1b]5522;type=read:status=DONE\x1b\\";

const MB: usize = 1024 * 1024;

fn packet(meta: &str, payload: Option<&[u8]>) -> Vec<u8> {
    let mut out = format!("\x1b]5522;type=read:{meta}").into_bytes();
    if let Some(payload) = payload {
        out.push(b';');
        out.extend(STANDARD.encode(payload).bytes());
    }
    out.extend_from_slice(b"\x1b\\");
    out
}

/// A reply the way kitty sends it: OK, 4096-byte chunks, DONE.
fn reply(mime: &str, data: &[u8]) -> Vec<u8> {
    let mime = format!("status=DATA:mime={}", STANDARD.encode(mime));
    let mut out = packet("status=OK", None);
    for chunk in data.chunks(4096) {
        out.extend(packet(&mime, Some(chunk)));
    }
    out.extend(packet("status=DONE", None));
    out
}

fn image(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = fastrand::Rng::with_seed(seed);
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    data.extend((8..len).map(|_| rng.u8(..)));
    data
}

fn read_all(input: &[u8], limit: usize) -> (Result<Contents, ReadError>, Vec<Focus>) {
    let mut reader = Reader::new(limit, None);
    reader.feed(input);
    let focus = reader.take_focus();
    (reader.finish(), focus)
}

#[test]
fn builds_requests_like_the_owners_hand_test() {
    assert_eq!(list_request(None), "\x1b]5522;type=read;Lg==\x1b\\");
    assert_eq!(
        read_request(&["image/png"], None, None),
        "\x1b]5522;type=read;aW1hZ2UvcG5n\x1b\\"
    );
}

#[test]
fn builds_requests_with_an_id_and_credentials() {
    let credentials = Credentials::new("secret", "faro");
    assert_eq!(
        read_request(
            &["image/png", "image/jpeg"],
            Some("faro-1"),
            Some(&credentials)
        ),
        format!(
            "\x1b]5522;type=read:id=faro-1:pw={}:name={};{}\x1b\\",
            STANDARD.encode("secret"),
            STANDARD.encode("faro"),
            STANDARD.encode("image/png image/jpeg"),
        )
    );
    // Characters kitty would strip never go out, so the echoed id matches.
    assert_eq!(
        list_request(Some("a b:c;d")),
        "\x1b]5522;type=read:id=abcd;Lg==\x1b\\"
    );
    assert_eq!(list_request(Some(" ;")), list_request(None));
}

#[test]
fn credentials_debug_hides_the_password() {
    let shown = format!("{:?}", Credentials::new("secret", "faro"));
    assert!(!shown.contains("secret"), "{shown}");
    assert!(shown.contains("faro"), "{shown}");
}

#[test]
fn reads_the_captured_type_list_split_anywhere() {
    for split in 0..=TEST_4.len() {
        let mut reader = Reader::new(MB, None);
        reader.feed(&TEST_4[..split]);
        assert!(!reader.finished() || split == TEST_4.len(), "split {split}");
        reader.feed(&TEST_4[split..]);
        assert!(reader.finished(), "split {split}");
        assert_eq!(reader.dropped(), 0);
        assert_eq!(reader.buffered(), 0);
        let contents = reader.finish().expect("a complete reply");
        assert_eq!(contents.types(), ["image/png", "text/html"]);
    }
}

#[test]
fn reads_a_multi_chunk_image_in_random_pieces() {
    let data = image(285_083, 1);
    let input = reply("image/png", &data);
    let mut rng = fastrand::Rng::with_seed(2);
    for _ in 0..20 {
        let mut reader = Reader::new(25 * MB, None);
        let mut rest = input.as_slice();
        while !rest.is_empty() {
            let (piece, tail) = rest.split_at(rng.usize(1..=rest.len().min(3000)));
            assert!(!reader.finished());
            reader.feed(piece);
            rest = tail;
        }
        assert!(reader.finished());
        let entry = reader
            .finish()
            .expect("complete")
            .take("image/png")
            .expect("png data");
        assert_eq!(entry.data, data);
    }
}

#[test]
fn keeps_types_apart() {
    let mut input = reply("image/png", b"png bytes");
    input.truncate(input.len() - packet("status=DONE", None).len());
    input.extend(packet(
        &format!("status=DATA:mime={}", STANDARD.encode("text/html")),
        Some(b"<b>"),
    ));
    input.extend(packet("status=DONE", None));
    let (contents, _) = read_all(&input, MB);
    let contents = contents.expect("complete");
    assert_eq!(contents.entries.len(), 2);
    assert_eq!(
        contents.clone().take("text/html").map(|e| e.data),
        Some(b"<b>".to_vec())
    );
    assert_eq!(
        contents.take("image/png").map(|e| e.data),
        Some(b"png bytes".to_vec())
    );
}

#[test]
fn error_statuses_end_the_reply() {
    for (status, error) in [
        ("EPERM", ReadError::Denied),
        ("EBUSY", ReadError::Busy),
        ("ENOSYS", ReadError::Unavailable),
        ("EWHAT", ReadError::Status("EWHAT".into())),
    ] {
        let mut reader = Reader::new(MB, None);
        reader.feed(&packet(&format!("status={status}"), None));
        assert!(reader.finished(), "{status}");
        assert_eq!(reader.finish(), Err(error));
    }
}

/// Seen on the owner's machine (D90): the focus-out report came before the
/// reply and tmux put the focus-in report inside a data packet; mouse reports
/// arrived in between.
#[test]
fn turns_focus_reports_back_into_events_wherever_they_are() {
    let data = image(10_000, 3);
    let mut input = b"\x1b[O\x1b[<35;52;39M".to_vec();
    let body = reply("image/png", &data);
    let middle = body.len() / 2;
    input.extend_from_slice(&body[..middle]);
    input.extend_from_slice(b"\x1b[I");
    input.extend_from_slice(&body[middle..]);
    let (contents, focus) = read_all(&input, MB);
    assert_eq!(focus, [Focus::Lost, Focus::Gained]);
    assert_eq!(
        contents
            .expect("complete")
            .take("image/png")
            .map(|e| e.data),
        Some(data)
    );
}

#[test]
fn drops_keys_and_other_replies_around_packets() {
    let mut input = b"ab\x1bx\x1b]11;rgb:0000/0000/0000\x1b\\".to_vec();
    input.extend_from_slice(TEST_4);
    input.extend_from_slice(b"cd\x1b[A");
    let mut reader = Reader::new(MB, None);
    reader.feed(&input);
    // What comes after the reply is handed back, not dropped.
    assert_eq!(reader.take_rest(), b"cd\x1b[A");
    assert_eq!(
        reader.dropped(),
        input.len() - TEST_4.len() - b"cd\x1b[A".len()
    );
    assert_eq!(reader.finish().map(|c| c.types().len()), Ok(2));
}

#[test]
fn too_large_frees_the_data_but_reads_to_done() {
    let data = image(20_000, 4);
    let input = reply("image/png", &data);
    let done = packet("status=DONE", None);
    let mut reader = Reader::new(10_000, None);
    reader.feed(&input[..input.len() - done.len()]);
    assert!(!reader.finished(), "must keep reading until DONE");
    reader.feed(&done);
    assert!(reader.finished());
    assert_eq!(reader.finish(), Err(ReadError::TooLarge { limit: 10_000 }));
}

#[test]
fn bad_base64_is_malformed_after_done() {
    let mut input = packet("status=OK", None);
    let mime = STANDARD.encode("image/png");
    input.extend_from_slice(
        format!("\x1b]5522;type=read:status=DATA:mime={mime};aGVsbG8\x1b\\").as_bytes(),
    );
    input.extend(packet("status=DONE", None));
    let (contents, _) = read_all(&input, MB);
    assert!(
        matches!(contents, Err(ReadError::Malformed(_))),
        "{contents:?}"
    );
}

#[test]
fn no_reply_and_incomplete_replies() {
    assert_eq!(Reader::new(MB, None).finish(), Err(ReadError::NoReply));
    let mut reader = Reader::new(MB, None);
    reader.feed(&packet("status=OK", None));
    assert!(reader.started());
    assert!(!reader.finished());
    assert_eq!(reader.finish(), Err(ReadError::Incomplete));
}

#[test]
fn ignores_packets_for_another_request() {
    let mut reader = Reader::new(MB, Some("faro-2"));
    // A late reply to an earlier request, then ours, then a write reply.
    reader.feed(&packet("id=faro-1:status=EPERM", None));
    reader.feed(b"\x1b]5522;type=write:status=DONE\x1b\\");
    assert!(!reader.finished());
    assert_eq!(reader.ignored(), 2);
    reader.feed(&packet("id=faro-2:status=OK", None));
    // Terminals that don't echo the id are still understood.
    reader.feed(&packet("status=DONE", None));
    assert_eq!(reader.finish(), Ok(Contents::default()));
}

/// Under tmux every attached kitty answers (D91): a reader stops at the end
/// of the first reply and hands back the second.
#[test]
fn stops_at_the_end_of_its_reply() {
    let mut second = reply(LIST, b"image/png");
    let mut input = reply(LIST, b"text/plain");
    input.append(&mut second.clone());
    let mut reader = Reader::new(MB, None);
    reader.feed(&input[..10]);
    assert_eq!(
        reader.take_rest(),
        b"",
        "nothing to hand back before the end"
    );
    reader.feed(&input[10..]);
    assert!(reader.finished());
    let rest = reader.take_rest();
    assert_eq!(rest, second);
    assert_eq!(
        reader.finish().map(|c| c.types()),
        Ok(vec!["text/plain".to_owned()])
    );
    let mut next = Reader::new(MB, None);
    next.feed(&rest);
    second.clear();
    assert_eq!(
        next.finish().map(|c| c.types()),
        Ok(vec!["image/png".to_owned()])
    );
}

#[test]
fn picks_the_best_image_type() {
    let types = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        best_image_type(&types(&["text/plain", "image/jpeg", "image/png"])),
        Some("image/png")
    );
    assert_eq!(
        best_image_type(&types(&["image/webp", "image/gif"])),
        Some("image/gif")
    );
    assert_eq!(best_image_type(&types(&["text/plain", "image/tiff"])), None);
    assert_eq!(image_extension("image/jpeg"), Some("jpg"));
    assert_eq!(image_extension("text/plain"), None);
}

/// Rule 9 (D27): whatever arrives, the reader never panics, never stores more
/// than its limit, and never buffers more than one packet.
#[test]
fn survives_corrupted_and_random_input() {
    let limit = 50_000;
    let data = image(60_000, 5);
    let clean = reply("image/png", &data);
    let mut rng = fastrand::Rng::with_seed(6);
    for round in 0..300 {
        let mut input = clean.clone();
        for _ in 0..rng.usize(0..20) {
            let at = rng.usize(..input.len());
            match rng.u8(..4) {
                0 => input[at] = rng.u8(..),
                1 => {
                    input.remove(at);
                }
                2 => input.insert(
                    at,
                    *rng.choice(&[0x1b, b'[', b']', b'\\', 0x07, b';', b':'])
                        .expect("not empty"),
                ),
                _ => input
                    .splice(at..at, b"\x1b[I".iter().copied())
                    .for_each(drop),
            }
        }
        if round % 10 == 0 {
            input = (0..rng.usize(..100_000)).map(|_| rng.u8(..)).collect();
        }
        let mut reader = Reader::new(limit, None);
        let mut rest = input.as_slice();
        while !rest.is_empty() && !reader.finished() {
            let (piece, tail) = rest.split_at(rng.usize(1..=rest.len().min(5000)));
            reader.feed(piece);
            rest = tail;
            assert!(
                reader.buffered() <= MAX_PACKET + 1,
                "round {round}: {}",
                reader.buffered()
            );
            assert!(reader.stored <= limit);
        }
        if let Ok(contents) = reader.finish() {
            assert!(contents.entries.iter().map(|e| e.data.len()).sum::<usize>() <= limit);
        }
    }
}

#[test]
fn an_endless_packet_is_dropped_not_buffered() {
    let mut reader = Reader::new(MB, None);
    reader.feed(&packet("status=OK", None));
    reader.feed(b"\x1b]5522;type=read:status=DATA:mime=Lg==;");
    for _ in 0..100 {
        reader.feed(&[b'A'; 1000]);
        assert!(reader.buffered() <= MAX_PACKET + 1000);
    }
    reader.feed(&packet("status=DONE", None));
    assert!(matches!(reader.finish(), Err(ReadError::Malformed(_))));
}
