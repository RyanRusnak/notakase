// watch.rs — watch the sync folder and flip a flag when it changes.
//
// The watcher thread never touches the Vault; it only sets an AtomicBool. The
// main event loop notices the flag and does the actual sync + reload on its own
// thread, so there is no shared mutable state to guard.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Start watching `folder`. Returns the watcher, which must be kept alive for
/// watching to continue (dropping it stops the thread). `None` if the folder
/// can't be watched — the loop's periodic sync still covers that case.
pub fn spawn(folder: &Path, dirty: Arc<AtomicBool>) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            // any create/modify/remove in the folder means "re-sync"
            if ev.kind.is_create() || ev.kind.is_modify() || ev.kind.is_remove() {
                dirty.store(true, Ordering::Relaxed);
            }
        }
    })
    .ok()?;
    watcher.watch(folder, RecursiveMode::NonRecursive).ok()?;
    Some(watcher)
}
