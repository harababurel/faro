//! Attachments in memory (D117): the demo has no file system, so a
//! downloaded attachment's bytes live here, under a made-up path that the
//! app treats as the file's (`AttachmentStatus::Downloaded`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use faro_core::domain::{AttachmentId, MessageRef};

/// Where the demo's "files" are; never a real directory.
const ROOT: &str = "/faro-demo";

#[derive(Debug, Clone)]
pub struct Blob {
    /// The backend's id for the attachment (the canned transcripts use it).
    pub id: AttachmentId,
    pub bytes: Arc<[u8]>,
}

/// The attachments downloaded this visit, shared by the command task, the
/// router (audio) and the image cache. (Shared across tasks, so `Send`:
/// natively the tasks run on tokio; the page has one thread anyway.)
#[derive(Debug, Clone, Default)]
pub struct Blobs(Arc<Mutex<HashMap<PathBuf, Blob>>>);

impl Blobs {
    /// Keep `bytes` for attachment `index` of `message`; the path to use.
    pub fn insert(
        &self,
        message: MessageRef,
        index: usize,
        id: AttachmentId,
        bytes: Vec<u8>,
    ) -> PathBuf {
        let path = PathBuf::from(format!(
            "{ROOT}/{}-{}-{index}",
            message.sender,
            message.sent_at.as_millis()
        ));
        let blob = Blob {
            id,
            bytes: bytes.into(),
        };
        self.lock().insert(path.clone(), blob);
        path
    }

    pub fn get(&self, path: &Path) -> Option<Blob> {
        self.lock().get(path).cloned()
    }

    /// Never poisoned in practice: nothing panics while holding it.
    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, Blob>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `text` names one of these (a path faro would copy).
    pub fn holds(&self, text: &str) -> bool {
        self.lock().contains_key(Path::new(text))
    }
}

#[cfg(test)]
mod tests {
    use faro_core::domain::Timestamp;

    use super::*;

    #[test]
    fn bytes_come_back_by_path() {
        let blobs = Blobs::default();
        let message = MessageRef {
            sender: "a0000000-0000-4000-8000-000000000001".parse().unwrap(),
            sent_at: Timestamp::from_millis(42),
        };
        let path = blobs.insert(message, 1, AttachmentId("mock:x".into()), vec![1, 2, 3]);
        assert!(path.starts_with(ROOT));
        let blob = blobs.get(&path).unwrap();
        assert_eq!(&*blob.bytes, &[1, 2, 3]);
        assert_eq!(blob.id.0, "mock:x");
        assert!(blobs.holds(&path.display().to_string()));
        assert!(!blobs.holds("/etc/hosts"));
    }
}
