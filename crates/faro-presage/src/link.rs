//! Linking this computer as a new device of a Signal account (M7 task 2).

use futures::channel::oneshot;
use presage::Manager;
use presage::libsignal_service::configuration::SignalServers;
use presage_store_sqlite::SqliteStore;
use url::Url;

use crate::{Error, Linked};

/// `faro (<short hostname>)`, as it shows in the phone's list of linked devices.
pub fn default_device_name(hostname: Option<&str>) -> String {
    let short = hostname.map(|h| h.trim().split('.').next().unwrap_or_default());
    match short.filter(|h| !h.is_empty()) {
        Some(host) => format!("faro ({host})"),
        None => "faro".to_owned(),
    }
}

/// Link a new device. `show` gets the provisioning URL to present as a QR
/// code; this returns once the phone has scanned it and confirmed, or fails.
/// Any earlier link in `store` is cleared first.
pub async fn link(
    store: SqliteStore,
    device_name: String,
    show: impl FnOnce(&Url),
) -> Result<Linked, Error> {
    let (url_tx, url_rx) = oneshot::channel();
    let linking =
        Manager::link_secondary_device(store, SignalServers::Production, device_name, url_tx);
    let presenting = async move {
        if let Ok(url) = url_rx.await {
            show(&url);
        }
    };
    let (linked, ()) = futures::future::join(Box::pin(linking), presenting).await;
    Ok(linked?)
}
