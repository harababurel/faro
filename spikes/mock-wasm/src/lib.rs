//! M13 Phase 0 spike (throwaway): run faro-core's `MockBackend` + `Sim` on wasm32.
//!
//! `first_events` is the same code on both targets; `main.rs` runs it natively on a
//! paused tokio clock and `run.mjs` runs it in Node from wasm, so the two event
//! sequences can be compared line by line.

use std::time::Duration;

use faro_core::backend::{Backend, BackendEvent, OutgoingMessage};
use faro_core::domain::{DeliveryStatus, MessageBody, MessageRef, Timestamp};
use faro_core::mock::{MockBackend, MockConfig};

fn config() -> MockConfig {
    MockConfig {
        // Fast enough to see a few hundred events in a second or two of real time.
        activity: Some(Duration::from_millis(5)),
        disconnects: false,
        errors: false,
        ..MockConfig::deterministic(42)
    }
}

/// One line per event: the variant name plus its (redacting) `Debug` form.
fn line(event: &BackendEvent) -> String {
    let text = format!("{event:?}");
    text.chars().take(160).collect()
}

/// The first `n` events of the deterministic mock, one line each.
pub async fn first_events(n: usize) -> Vec<String> {
    let (_backend, mut events) = MockBackend::new(&config());
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        match events.recv().await {
            Some(event) => out.push(line(&event)),
            None => break,
        }
    }
    out
}

/// Send one message and report its delivery statuses (exercises the mock's
/// request latency timer and command → driver wake-up path).
pub async fn send_round_trip() -> Vec<String> {
    let (backend, mut events) = MockBackend::new(&MockConfig {
        activity: None,
        backlog: 0,
        ..config()
    });
    let mut log = Vec::new();
    // Wait for the connection.
    while let Some(event) = events.recv().await {
        let connected = matches!(event, BackendEvent::Connection(_));
        log.push(line(&event));
        if connected && log.len() > 1 {
            break;
        }
    }
    let Ok(me) = backend.whoami().await else {
        return vec!["whoami failed".into()];
    };
    let Some(to) = backend.contacts().await.ok().and_then(|c| c.first().map(|c| c.service_id))
    else {
        return vec!["no contacts".into()];
    };
    let sent_at = Timestamp::from_millis(faro_core::mock::DETERMINISTIC_EPOCH.as_millis() + 1);
    let msg = OutgoingMessage {
        sent_at,
        body: MessageBody::new("hello from wasm"),
        mentions: vec![],
        quote: None,
        attachments: vec![],
    };
    if let Err(e) = backend.send(to.into(), msg).await {
        return vec![format!("send failed: {e}")];
    }
    log.push("send: ok".into());
    let target = MessageRef { sender: me.service_id, sent_at };
    while let Some(event) = events.recv().await {
        if let BackendEvent::DeliveryUpdate { message, status, .. } = &event {
            if *message == target {
                log.push(format!("status: {status:?}"));
                if *status == DeliveryStatus::Read {
                    break;
                }
            }
        }
    }
    log
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub async fn first_events(n: usize) -> String {
        super::first_events(n).await.join("\n")
    }

    #[wasm_bindgen]
    pub async fn send_round_trip() -> String {
        super::send_round_trip().await.join("\n")
    }
}
