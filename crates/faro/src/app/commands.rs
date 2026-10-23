//! Runs the reducer's backend commands (sending, reactions, read receipts,
//! typing) one at a time, in order, and reports results back to the app loop
//! as `AppEvent`s (D39). Attachment downloads (`Command::Fetch`, D75) are
//! spawned as concurrent local tasks so they never block sends or receipts.
//!
//! Backend futures need not be `Send` (D30), so the task is local to the
//! current-thread runtime (`spawn_local`, inside the `LocalSet` set up in `main`).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use faro_core::backend::{Backend, BackendError, OutgoingMessage};
use faro_core::config::{TranscribeMode, Voice};
use faro_core::domain::{
    AttachmentRef, AttachmentStatus, MessageRef, ServiceId, Timestamp, TranscriptSource,
    transcript_paragraph,
};
use faro_core::paths;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::event::{AppEvent, Command};
use faro_app::app::commands as shared;

/// Abort spawned download tasks when the main command task is aborted.
#[derive(Default)]
struct InFlight(Vec<JoinHandle<()>>);

impl InFlight {
    fn spawn(&mut self, task: JoinHandle<()>) {
        self.0.retain(|h| !h.is_finished());
        self.0.push(task);
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// Start the task; it owns the backend and drops it when the sender is dropped
/// (or the `LocalSet` ends). Commands are queued without limit, like effects:
/// the reducer must never wait.
pub fn spawn<B: Backend + 'static>(
    backend: B,
    me: ServiceId,
    attachments_dir: PathBuf,
    results: mpsc::UnboundedSender<AppEvent>,
) -> (mpsc::UnboundedSender<Command>, JoinHandle<()>) {
    let (commands, queue) = mpsc::unbounded_channel();
    let task = tokio::task::spawn_local(run(Rc::new(backend), me, attachments_dir, queue, results));
    (commands, task)
}

async fn run<B: Backend + 'static>(
    backend: Rc<B>,
    me: ServiceId,
    attachments_dir: PathBuf,
    mut queue: mpsc::UnboundedReceiver<Command>,
    results: mpsc::UnboundedSender<AppEvent>,
) {
    let mut in_flight = InFlight::default();
    while let Some(command) = queue.recv().await {
        if let Command::Fetch {
            conversation,
            message,
            index,
            attachment,
        } = command
        {
            let backend = Rc::clone(&backend);
            let dir = attachments_dir.clone();
            let tx = results.clone();
            in_flight.spawn(tokio::task::spawn_local(async move {
                let result = fetch(backend.as_ref(), &dir, message, index, attachment).await;
                if let Err(error) = &result {
                    tracing::warn!(%error, %conversation, "attachment download failed");
                }
                let _ = tx.send(AppEvent::Fetched {
                    conversation,
                    message,
                    index,
                    result,
                });
            }));
            continue;
        }
        if let Command::Transcribe {
            conversation,
            message,
            index,
            path,
            duration_secs,
            voice,
        } = command
        {
            let dir = attachments_dir.clone();
            let tx = results.clone();
            in_flight.spawn(tokio::task::spawn_local(async move {
                let result = transcribe(&dir, message, index, &path, duration_secs, &voice)
                    .await
                    .map(|(text, source)| (transcript_paragraph(&text), source));
                if let Err(error) = &result {
                    tracing::warn!(%error, %conversation, "voice note transcription failed");
                }
                let _ = tx.send(AppEvent::Transcribed {
                    conversation,
                    message,
                    index,
                    result,
                });
            }));
            continue;
        }
        let Some(event) = execute(backend.as_ref(), me, &attachments_dir, command).await else {
            continue;
        };
        if results.send(event).is_err() {
            tracing::debug!("app loop stopped before a backend result arrived");
        }
    }
}

async fn fetch<B: Backend>(
    backend: &B,
    attachments_dir: &Path,
    message: MessageRef,
    index: usize,
    mut attachment: AttachmentRef,
) -> Result<AttachmentRef, BackendError> {
    let bytes = backend.fetch_attachment(&attachment).await?;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let name = paths::attachment_file_name(
        message,
        index,
        attachment.file_name.as_deref(),
        &attachment.content_type,
    );
    let path = attachments_dir.join(name);
    let needs_dims =
        attachment.is_image() && (attachment.width.is_none() || attachment.height.is_none());
    let needs_audio = attachment.is_voice_note()
        && (attachment.duration_secs.is_none() || attachment.waveform.is_none());
    let (path, dims, wav_info, fallback_waveform) = tokio::task::spawn_blocking(move || {
        let dims = if needs_dims {
            // Someone else's picture: a decoder panic mustn't stop faro (D151).
            crate::crash::contained(|| {
                image::ImageReader::new(std::io::Cursor::new(&bytes))
                    .with_guessed_format()
                    .ok()
                    .and_then(|r| r.into_dimensions().ok())
            })
            .flatten()
        } else {
            None
        };
        let (wav_info, fallback_waveform) = if needs_audio {
            let wav = faro_core::audio::inspect_wav(&bytes);
            let synth = wav
                .is_none()
                .then(|| faro_core::audio::synthetic_waveform(&bytes));
            (wav, synth)
        } else {
            (None, None)
        };
        paths::write_private_atomic(&path, &bytes)
            .map(|()| (path, dims, wav_info, fallback_waveform))
            .map_err(|e| BackendError::Storage(format!("writing attachment: {e}")))
    })
    .await
    .map_err(|e| BackendError::Storage(format!("attachment writer task: {e}")))??;
    if let Some((w, h)) = dims.filter(|(w, h)| *w > 0 && *h > 0) {
        attachment.width = Some(w);
        attachment.height = Some(h);
    }
    if needs_audio {
        let info = match wav_info {
            Some(info) => Some(info),
            None => inspect_audio_with_ffmpeg(&path).await,
        };
        if let Some(info) = info {
            if attachment.duration_secs.is_none() {
                attachment.duration_secs = info.duration_secs;
            }
            if attachment.waveform.is_none() {
                attachment.waveform = Some(info.waveform);
            }
        } else if attachment.waveform.is_none() {
            attachment.waveform = fallback_waveform;
        }
    }
    attachment.size = Some(size);
    attachment.status = AttachmentStatus::Downloaded(path);
    Ok(attachment)
}

async fn inspect_audio_with_ffmpeg(path: &Path) -> Option<faro_core::audio::AudioInfo> {
    use std::process::Stdio;
    let fut = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-ac", "1", "-ar", "8000", "-f", "s16le", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), fut)
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    faro_core::audio::pcm_s16le_info(&output.stdout, 8_000)
}

/// Transcribe a downloaded voice note (`local` Whisper CLI or `remote`
/// OpenAI-compatible HTTP endpoint, D82).
pub async fn transcribe(
    attachments_dir: &Path,
    message: MessageRef,
    index: usize,
    path: &Path,
    duration_secs: Option<u32>,
    voice: &Voice,
) -> Result<(String, TranscriptSource), String> {
    let secs = u64::from(duration_secs.unwrap_or(0));
    match voice.transcribe {
        TranscribeMode::Remote => {
            let text = transcribe_remote(path, voice, remote_timeout(secs)).await?;
            Ok((text, TranscriptSource::Remote))
        }
        TranscribeMode::Local | TranscribeMode::Off => {
            let timeout = local_timeout(secs);
            let text =
                transcribe_local(attachments_dir, message, index, path, voice, timeout).await?;
            Ok((text, TranscriptSource::Local))
        }
    }
}

/// How long local Whisper may take for a note of `secs` seconds (D145): two
/// minutes to load the model, then up to 8× real time (slow CPUs, larger
/// models), at most an hour.
fn local_timeout(secs: u64) -> u64 {
    (120 + 8 * secs).min(3600)
}

/// How long a remote endpoint may take (D145): a minute, plus twice the
/// note's length for the upload and the work, at most 15 minutes.
fn remote_timeout(secs: u64) -> u64 {
    (60 + 2 * secs).min(900)
}

async fn transcribe_local(
    attachments_dir: &Path,
    message: MessageRef,
    index: usize,
    path: &Path,
    voice: &Voice,
    timeout_secs: u64,
) -> Result<String, String> {
    use std::process::Stdio;
    let work_dir = attachments_dir.join(format!(
        "transcribe-{}-{}-{index}",
        message.sender,
        message.sent_at.as_millis()
    ));
    let dir_for_setup = work_dir.clone();
    tokio::task::spawn_blocking(move || prepare_transcribe_dir(&dir_for_setup))
        .await
        .map_err(|e| format!("transcription setup task: {e}"))??;

    let (program, args) = build_local_whisper_args(voice, path, &work_dir);
    let fut = tokio::process::Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let run_res = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut).await;

    let dir_for_cleanup = work_dir.clone();
    let file_text = tokio::task::spawn_blocking(move || finish_transcribe_dir(&dir_for_cleanup))
        .await
        .ok()
        .flatten();

    let output = run_res
        .map_err(|_| format!("'{program}' timed out after {timeout_secs}s"))?
        .map_err(|e| format!("failed to run '{program}': {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("'{program}' exited with {}", output.status)
        } else {
            format!("'{program}': {stderr}")
        });
    }
    let text = file_text
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    if text.is_empty() {
        return Err(format!("'{program}' produced an empty transcript"));
    }
    Ok(text)
}

fn build_local_whisper_args(voice: &Voice, path: &Path, out_dir: &Path) -> (String, Vec<String>) {
    let mut parts = voice.transcribe_command.split_whitespace();
    let program = parts.next().unwrap_or("whisper").to_owned();
    let raw_args: Vec<&str> = parts.collect();
    let file_str = path.display().to_string();
    let out_str = out_dir.display().to_string();
    let has_placeholders = raw_args.iter().any(|a| {
        a.contains("{file}")
            || a.contains("{model}")
            || a.contains("{language}")
            || a.contains("{out_dir}")
    });
    if has_placeholders {
        let args = raw_args
            .into_iter()
            .map(|a| {
                a.replace("{file}", &file_str)
                    .replace("{model}", &voice.transcribe_model)
                    .replace("{language}", &voice.transcribe_language)
                    .replace("{out_dir}", &out_str)
            })
            .collect();
        return (program, args);
    }
    let mut args: Vec<String> = raw_args.iter().map(|s| (*s).to_owned()).collect();
    let is_cpp = program.contains("whisper-cli") || program.contains("whisper-cpp");
    if is_cpp {
        if !voice.transcribe_model.is_empty()
            && !raw_args.iter().any(|a| *a == "-m" || *a == "--model")
        {
            args.extend(["-m".to_owned(), voice.transcribe_model.clone()]);
        }
        if !voice.transcribe_language.is_empty()
            && !raw_args.iter().any(|a| *a == "-l" || *a == "--language")
        {
            args.extend(["-l".to_owned(), voice.transcribe_language.clone()]);
        }
        args.extend([
            "-np".to_owned(),
            "-nt".to_owned(),
            "-f".to_owned(),
            file_str,
        ]);
    } else {
        if !voice.transcribe_model.is_empty() && !raw_args.contains(&"--model") {
            args.extend(["--model".to_owned(), voice.transcribe_model.clone()]);
        }
        if !voice.transcribe_language.is_empty() && !raw_args.contains(&"--language") {
            args.extend(["--language".to_owned(), voice.transcribe_language.clone()]);
        }
        args.extend([
            "--output_format".to_owned(),
            "txt".to_owned(),
            "--output_dir".to_owned(),
            out_str,
            file_str,
        ]);
    }
    (program, args)
}

#[expect(
    clippy::disallowed_methods,
    reason = "creating the temporary Whisper output directory runs inside spawn_blocking"
)]
fn prepare_transcribe_dir(dir: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dir);
    paths::create_private_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))
}

#[expect(
    clippy::disallowed_methods,
    reason = "reading and cleaning up the Whisper output directory runs inside spawn_blocking"
)]
fn finish_transcribe_dir(dir: &Path) -> Option<String> {
    let mut found = None;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("txt")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                let trimmed = text.trim().to_owned();
                if !trimmed.is_empty() {
                    found = Some(trimmed);
                    break;
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    found
}

async fn transcribe_remote(
    path: &Path,
    voice: &Voice,
    timeout_secs: u64,
) -> Result<String, String> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    let api_key = resolve_transcribe_api_key().await?;
    let mut args = vec![
        "-sS".to_owned(),
        "--fail-with-body".to_owned(),
        "--max-time".to_owned(),
        timeout_secs.to_string(),
        "--config".to_owned(),
        "-".to_owned(),
        "-F".to_owned(),
        format!("file=@{}", path.display()),
        "-F".to_owned(),
        format!("model={}", voice.transcribe_remote_model),
        "-F".to_owned(),
        "response_format=json".to_owned(),
    ];
    if !voice.transcribe_language.is_empty() {
        args.push("-F".to_owned());
        args.push(format!("language={}", voice.transcribe_language));
    }
    args.push(voice.transcribe_url.clone());

    let mut child = tokio::process::Command::new("curl")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to run 'curl': {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let escaped = api_key.replace('\\', "\\\\").replace('"', "\\\"");
        let cfg = format!("header = \"Authorization: Bearer {escaped}\"\n");
        let _ = stdin.write_all(cfg.as_bytes()).await;
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs + 5),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| format!("remote transcription timed out after {timeout_secs}s"))?
    .map_err(|e| format!("waiting for 'curl': {e}"))?;

    if !output.status.success() {
        if let Err(api_err) = faro_core::audio::parse_whisper_json(&output.stdout)
            && !api_err.starts_with("invalid JSON")
        {
            return Err(api_err);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            stderr
        });
    }
    faro_core::audio::parse_whisper_json(&output.stdout)
}

async fn resolve_transcribe_api_key() -> Result<String, String> {
    for var in ["FARO_TRANSCRIBE_API_KEY", "OPENAI_API_KEY"] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim().to_owned();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }
    }
    let keychain_cmd: (&str, &[&str]) = if cfg!(target_os = "macos") {
        (
            "security",
            &["find-generic-password", "-s", "faro-transcribe", "-w"],
        )
    } else {
        ("secret-tool", &["lookup", "service", "faro-transcribe"])
    };
    if let Ok(Ok(out)) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::process::Command::new(keychain_cmd.0)
            .args(keychain_cmd.1)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
        && out.status.success()
    {
        let key = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !key.is_empty() {
            return Ok(key);
        }
    }
    Err(
        "Missing transcription API key: set $FARO_TRANSCRIBE_API_KEY or $OPENAI_API_KEY (or store service 'faro-transcribe' in keychain)"
            .to_owned(),
    )
}

/// Run one command through the shared code (D113), after copying outgoing
/// attachments into `attachments/`, where the backend reads them.
async fn execute<B: Backend>(
    backend: &B,
    me: ServiceId,
    attachments_dir: &Path,
    command: Command,
) -> Option<AppEvent> {
    let command = match command {
        Command::Send {
            conversation,
            mut message,
        } => {
            let sent_at = message.sent_at;
            if let Err(error) =
                copy_outgoing_attachments(me, attachments_dir, sent_at, &mut message).await
            {
                tracing::warn!(%error, %conversation, "sending failed");
                return Some(AppEvent::SendResult {
                    sent_at,
                    result: Err(error),
                });
            }
            Command::Send {
                conversation,
                message,
            }
        }
        other @ (Command::React { .. }
        | Command::Edit { .. }
        | Command::DeleteForEveryone { .. }
        | Command::Fetch { .. }
        | Command::Transcribe { .. }
        | Command::MarkRead(_)
        | Command::Typing { .. }) => other,
    };
    shared::execute(backend, command).await
}

async fn copy_outgoing_attachments(
    me: ServiceId,
    attachments_dir: &Path,
    sent_at: Timestamp,
    message: &mut OutgoingMessage,
) -> Result<(), BackendError> {
    if message.attachments.is_empty() {
        return Ok(());
    }
    let msg_ref = MessageRef {
        sender: me,
        sent_at,
    };
    for (index, att) in message.attachments.iter_mut().enumerate() {
        let name = paths::attachment_file_name(
            msg_ref,
            index,
            att.file_name.as_deref(),
            &att.content_type,
        );
        let dest = attachments_dir.join(name);
        if att.path == dest {
            continue;
        }
        let src = att.path.clone();
        let target = dest.clone();
        // A pasted clipboard image isn't needed once copied (D93).
        let pasted = crate::app::clipboard::is_pasted(&src, attachments_dir);
        tokio::task::spawn_blocking(move || copy_attachment_file(&src, &target, pasted))
            .await
            .map_err(|e| BackendError::Storage(format!("attachment copy task: {e}")))??;
        att.path = dest;
    }
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "copying an outgoing attachment into faro's attachments/ runs inside spawn_blocking"
)]
fn copy_attachment_file(src: &Path, dest: &Path, remove_source: bool) -> Result<(), BackendError> {
    let bytes = std::fs::read(src)
        .map_err(|e| BackendError::Storage(format!("reading {}: {e}", src.display())))?;
    paths::write_private_atomic(dest, &bytes)
        .map_err(|e| BackendError::Storage(format!("writing {}: {e}", dest.display())))?;
    if remove_source && let Err(error) = std::fs::remove_file(src) {
        tracing::warn!(%error, "couldn't remove a pasted image after copying it");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts_grow_with_the_note() {
        assert_eq!(local_timeout(0), 120);
        assert_eq!(local_timeout(60), 600);
        assert_eq!(local_timeout(3 * 3600), 3600);
        assert_eq!(remote_timeout(0), 60);
        assert_eq!(remote_timeout(60), 180);
        assert_eq!(remote_timeout(3600), 900);
    }

    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "tokio::test macro expands to Runtime::block_on and test writes a mock script"
    )]
    async fn local_whisper_transcribes_from_output_dir_or_stdout() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("voice.wav");
        paths::write_private_atomic(&audio, b"RIFF").unwrap();
        let script1 = dir.path().join("mock-whisper-dir.sh");
        std::fs::write(
            &script1,
            "#!/bin/sh\nprintf 'Belay is on\\n' > \"$1/out.txt\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script1, std::fs::Permissions::from_mode(0o700)).unwrap();

        let msg_ref = MessageRef {
            sender: ServiceId::Aci(uuid::Uuid::from_u128(7)),
            sent_at: Timestamp::from_millis(1_000),
        };

        // 1) Command with {out_dir} and {file} placeholders writing a .txt file.
        let mut voice = Voice {
            transcribe: TranscribeMode::Local,
            transcribe_command: format!("{} {{out_dir}} {{file}}", script1.display()),
            ..Voice::default()
        };
        let (text, source) = transcribe(dir.path(), msg_ref, 0, &audio, Some(3), &voice)
            .await
            .unwrap();
        assert_eq!(text, "Belay is on");
        assert_eq!(source, TranscriptSource::Local);

        // 2) Command printing transcript to stdout.
        let script2 = dir.path().join("mock-whisper-stdout.sh");
        std::fs::write(
            &script2,
            "#!/bin/sh\nprintf 'On belay, climb when ready\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script2, std::fs::Permissions::from_mode(0o700)).unwrap();
        voice.transcribe_command = format!("{} {{file}}", script2.display());
        let (text, source) = transcribe(dir.path(), msg_ref, 0, &audio, Some(3), &voice)
            .await
            .unwrap();
        assert_eq!(text, "On belay, climb when ready");
        assert_eq!(source, TranscriptSource::Local);
    }
}
