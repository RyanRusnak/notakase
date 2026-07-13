// util.rs — small shared helpers.

use base64::Engine;
use rand::RngCore;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A stable, opaque note id: `note_<22 base64url chars>` (128 bits of entropy).
/// Independent of the note's path, so moving/renaming a note never changes its
/// identity across devices.
pub fn new_note_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    format!("note_{b64}")
}
