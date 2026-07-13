// sync.rs — the status the UI shows and the shape both transports report.
// (Folder + server sync themselves land in M2/M3; this is the shared vocabulary.)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncStatus {
    /// A folder the OS keeps in sync across devices (Syncthing/Dropbox/iCloud).
    pub folder: String,
    pub last_synced_at: Option<i64>,
    pub last_sync_error: Option<String>,
    /// A self-hosted relay base URL (alternative or additional transport).
    pub server_base_url: String,
    /// The manifest doc id at the relay that lists every note's doc id.
    pub server_manifest_id: String,
}
