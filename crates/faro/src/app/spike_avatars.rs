//! SPIKE (PLAN.md "Profile pictures"): stand-in profile pictures, so drawing
//! a list full of them can be judged in real terminals before faro fetches
//! any from Signal. `FARO_SPIKE_AVATARS=DIR` hands the pictures in `DIR`
//! (sorted by name) to the conversations in list order, round the pool
//! again when there are more conversations than pictures. Unset: nothing.
//! To be replaced by pictures fetched through the backend.

use std::path::PathBuf;

use super::state::State;

/// The pictures in `$FARO_SPIKE_AVATARS`, if it's set (logged either way).
pub async fn pool() -> Vec<PathBuf> {
    let Some(dir) = std::env::var_os("FARO_SPIKE_AVATARS") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    match tokio::fs::read_dir(&dir).await {
        Ok(mut entries) => {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_type().await.is_ok_and(|kind| kind.is_file()) {
                    found.push(entry.path());
                }
            }
        }
        Err(error) => tracing::warn!(%error, "spike: can't read FARO_SPIKE_AVATARS"),
    }
    found.sort();
    tracing::info!(pictures = found.len(), "spike: stand-in profile pictures");
    found
}

/// Give every conversation without a picture one from `pool`.
pub fn assign(state: &mut State, pool: &[PathBuf]) {
    if pool.is_empty() {
        return;
    }
    for (n, summary) in state.conversations.iter().enumerate() {
        if let Some(path) = pool.get(n % pool.len()) {
            state
                .avatars
                .entry(summary.id)
                .or_insert_with(|| path.clone());
        }
    }
}
