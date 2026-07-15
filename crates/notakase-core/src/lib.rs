// notakase-core — the data + sync layer, cleanly separated from the TUI.
//
// Layers:
//   • generic transport/crypto (copied from todarchy, proven): cryptobox,
//     keystore, sharelink, server_client
//   • notes-native document model: doc (per-note Automerge CRDT) + store
//     (the Vault: notes ⇄ plain .md files) + sync + config

pub mod config;
pub mod cryptobox;
pub mod doc;
pub mod folder_sync;
pub mod keystore;
pub mod manifest;
pub mod server_client;
pub mod server_sync;
pub mod sharelink;
pub mod store;
pub mod sync;
pub mod todokase;
pub mod util;
pub mod vaultkey;

pub use config::Config;
pub use doc::NoteDoc;
pub use folder_sync::SyncReport;
pub use manifest::Manifest;
pub use server_sync::ServerSync;
pub use store::{Note, Vault};
pub use sync::SyncStatus;

/// Bridge from core → frontend. The live sync loops (M2/M3) call these when a
/// merge brings in remote changes; the TUI implements it to refresh the view.
pub trait EventSink: Send + Sync {
    fn vault_changed(&self) {}
    fn sync_status(&self, _status: &SyncStatus) {}
    fn notify(&self, _title: &str, _body: &str) {}
}
