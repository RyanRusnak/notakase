// folder_sync.rs — sync the vault through a shared folder (Syncthing, Dropbox,
// iCloud, …). Each note's canonical Automerge document is mirrored into the
// folder as one file, keyed by the note's stable id:
//
//   <folder>/<note-id>.automerge         (plaintext)
//   <folder>/<note-id>.automerge.enc     (ChaCha20-Poly1305 sealed envelope)
//
// A sync is pull-then-push:
//   PULL  — merge every remote note file into the local document (CRDT union),
//           then materialize the plain .md files.
//   PUSH  — write each local document back, but only when the remote copy is
//           actually behind (compared by Automerge heads, so an unchanged note
//           is never rewritten — that is what stops watcher feedback loops).
//
// Because every note is its own CRDT, two devices editing different notes never
// touch the same file, and concurrent edits to one note merge conflict-free.

use std::fs;
use std::path::Path;

use anyhow::Result;
use automerge::ChangeHash;

use crate::cryptobox::{self, KEY_BYTES};
use crate::doc::NoteDoc;
use crate::store::{write_atomic, Note, Vault};

#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    /// Remote notes that were new or advanced our copy.
    pub pulled: usize,
    /// Local notes written out because the remote copy was behind.
    pub pushed: usize,
    /// Whether the local vault changed (→ the UI should reload).
    pub changed: bool,
}

impl SyncReport {
    /// Fold another report in (e.g. combining folder + relay results).
    pub fn merge(&mut self, other: SyncReport) {
        self.pulled += other.pulled;
        self.pushed += other.pushed;
        self.changed |= other.changed;
    }
}

impl Vault {
    /// Sync the vault through `folder`. If `key` is set, files are encrypted.
    pub fn sync_folder(
        &mut self,
        folder: &Path,
        key: Option<&[u8; KEY_BYTES]>,
    ) -> Result<SyncReport> {
        fs::create_dir_all(folder)?;
        let mut report = SyncReport::default();

        // ---- PULL ----
        for (id, plain) in read_remote(folder, key)? {
            let mut remote = match NoteDoc::from_bytes(&plain) {
                Ok(d) => d,
                Err(_) => continue, // ignore junk / undecodable
            };
            match self.notes.iter().position(|n| n.id == id) {
                Some(i) => {
                    let before = self.notes[i].doc.heads();
                    self.notes[i].doc.merge(&mut remote)?;
                    if self.notes[i].doc.heads() != before {
                        report.pulled += 1;
                        report.changed = true;
                    }
                }
                None => {
                    self.notes.push(Note { id, doc: remote });
                    report.pulled += 1;
                    report.changed = true;
                }
            }
        }

        if report.changed {
            self.materialize()?;
            self.save_ledger()?;
        }

        // ---- PUSH ----
        let encrypted = key.is_some();
        for note in &mut self.notes {
            let local_heads = note.doc.heads();
            let path = folder.join(remote_filename(&note.id, encrypted));
            if remote_heads(&path, encrypted, key) == Some(local_heads.clone()) {
                continue; // remote already has exactly this state
            }
            let bytes = note.doc.to_bytes();
            let payload = match key {
                Some(k) => cryptobox::seal(&bytes, k),
                None => bytes,
            };
            write_atomic(&path, &payload)?;
            report.pushed += 1;
        }

        self.persist()?;
        Ok(report)
    }
}

fn remote_filename(id: &str, encrypted: bool) -> String {
    if encrypted {
        format!("{id}.automerge.enc")
    } else {
        format!("{id}.automerge")
    }
}

/// Read every note file in `folder`, returning (id, plaintext bytes). Encrypted
/// files are opened with `key`; if the key is missing or wrong, they're skipped
/// (we never destroy bytes we can't authenticate). Files that don't match the
/// `note_*.automerge[.enc]` shape (e.g. Dropbox "conflicted copy" names) are
/// ignored rather than mis-ingested.
fn read_remote(folder: &Path, key: Option<&[u8; KEY_BYTES]>) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(folder) else {
        return Ok(out);
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = note_id_from_filename(&name) else {
            continue;
        };
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let plain = if name.ends_with(".enc") {
            match key.and_then(|k| cryptobox::open(&bytes, k).ok()) {
                Some(p) => p,
                None => continue,
            }
        } else {
            bytes
        };
        out.push((id, plain));
    }
    Ok(out)
}

fn note_id_from_filename(name: &str) -> Option<String> {
    let base = name.strip_suffix(".enc").unwrap_or(name);
    let id = base.strip_suffix(".automerge")?;
    if id.starts_with("note_") {
        Some(id.to_string())
    } else {
        None
    }
}

fn remote_heads(path: &Path, encrypted: bool, key: Option<&[u8; KEY_BYTES]>) -> Option<Vec<ChangeHash>> {
    let bytes = fs::read(path).ok()?;
    let plain = if encrypted {
        cryptobox::open(&bytes, key?).ok()?
    } else {
        bytes
    };
    let mut doc = NoteDoc::from_bytes(&plain).ok()?;
    Some(doc.heads())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cryptobox;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn dirs(tmp: &Path, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        (tmp.join(format!("{name}-vault")), tmp.join(format!("{name}-data")))
    }

    #[test]
    fn two_devices_share_a_note_through_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("sync");
        let (va, da) = dirs(tmp.path(), "a");
        let (vb, db) = dirs(tmp.path(), "b");

        // A authors a nested note and pushes it
        write(&va, "Projects/x/plan.md", "the plan");
        let mut a = Vault::open(&va, &da).unwrap();
        a.sync_folder(&folder, None).unwrap();

        // B starts empty and pulls it — materialized at the same nested path
        let mut b = Vault::open(&vb, &db).unwrap();
        let rep = b.sync_folder(&folder, None).unwrap();
        assert!(rep.pulled >= 1 && rep.changed);
        assert_eq!(
            fs::read_to_string(vb.join("Projects/x/plan.md")).unwrap(),
            "the plan"
        );
    }

    #[test]
    fn concurrent_edits_to_one_note_converge_through_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("sync");
        let (va, da) = dirs(tmp.path(), "a");
        let (vb, db) = dirs(tmp.path(), "b");

        // shared ancestor: A creates, both sync so B has the same document id
        write(&va, "n.md", "the quick fox");
        Vault::open(&va, &da).unwrap().sync_folder(&folder, None).unwrap();
        Vault::open(&vb, &db).unwrap().sync_folder(&folder, None).unwrap();

        // A edits its copy and pushes
        write(&va, "n.md", "the very quick fox");
        Vault::open(&va, &da).unwrap().sync_folder(&folder, None).unwrap();

        // B edits a different part, then syncs — the pull merges A's edit in
        write(&vb, "n.md", "the quick fox jumps");
        let mut b = Vault::open(&vb, &db).unwrap();
        b.sync_folder(&folder, None).unwrap();
        let merged = fs::read_to_string(vb.join("n.md")).unwrap();
        assert!(merged.contains("very"), "A's edit lost: {merged}");
        assert!(merged.contains("jumps"), "B's edit lost: {merged}");

        // and it flows back to A
        let mut a = Vault::open(&va, &da).unwrap();
        a.sync_folder(&folder, None).unwrap();
        let a_body = fs::read_to_string(va.join("n.md")).unwrap();
        assert!(a_body.contains("very") && a_body.contains("jumps"));
    }

    #[test]
    fn encrypted_sync_writes_envelopes_and_round_trips_with_the_key() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("sync");
        let (va, da) = dirs(tmp.path(), "a");
        let (vb, db) = dirs(tmp.path(), "b");
        let key = cryptobox::generate_key();

        write(&va, "secret.md", "top secret");
        Vault::open(&va, &da).unwrap().sync_folder(&folder, Some(&key)).unwrap();

        // on-disk files are sealed envelopes, not plaintext
        let f = fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().ends_with(".enc"))
            .expect("an encrypted note file");
        let bytes = fs::read(f.path()).unwrap();
        assert!(cryptobox::is_envelope(&bytes));
        assert!(!String::from_utf8_lossy(&bytes).contains("top secret"));

        // B with the key can read; B without the key cannot
        let mut b_ok = Vault::open(&vb, &db).unwrap();
        b_ok.sync_folder(&folder, Some(&key)).unwrap();
        assert_eq!(fs::read_to_string(vb.join("secret.md")).unwrap(), "top secret");

        let (vc, dc) = dirs(tmp.path(), "c");
        let mut c_nokey = Vault::open(&vc, &dc).unwrap();
        let rep = c_nokey.sync_folder(&folder, Some(&cryptobox::generate_key())).unwrap();
        assert_eq!(rep.pulled, 0, "wrong key must not decrypt");
    }

    #[test]
    fn repeated_sync_is_idempotent_no_rewrites() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("sync");
        let (va, da) = dirs(tmp.path(), "a");
        write(&va, "n.md", "stable");
        let mut a = Vault::open(&va, &da).unwrap();
        a.sync_folder(&folder, None).unwrap();
        // second sync with nothing changed must not push or pull anything
        let rep = a.sync_folder(&folder, None).unwrap();
        assert_eq!(rep.pushed, 0, "unchanged note was rewritten");
        assert_eq!(rep.pulled, 0);
        assert!(!rep.changed);
    }
}
