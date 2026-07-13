// store.rs — the Vault: the set of notes, and the bridge between the canonical
// per-note Automerge documents and the user-facing tree of plain markdown files.
//
// Canonical storage (never synced directly):
//   <data_dir>/notes/<note-id>.automerge      one CRDT document per note
//
// Vault (what the TUI browses and $EDITOR edits — a materialized *view*):
//   <vault_dir>/<path>                         e.g. Projects/notakase/spec.md
//
// This mirrors todarchy's "canonical Automerge → derived plain view" split. The
// note's relative `path` (stored inside its document) carries arbitrary folder
// nesting, so complex projects nest as deep as you like.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::doc::NoteDoc;
use crate::util::{new_note_id, now_ms};

pub struct Note {
    pub id: String,
    pub doc: NoteDoc,
}

pub struct Vault {
    pub vault_dir: PathBuf,
    pub data_dir: PathBuf,
    pub notes: Vec<Note>,
}

impl Vault {
    /// Open (or initialize) a vault: load the canonical documents, ingest any
    /// changes made to the plain files on disk, then re-materialize so the
    /// vault and the documents agree. Persists the documents before returning.
    pub fn open(vault_dir: impl AsRef<Path>, data_dir: impl AsRef<Path>) -> Result<Vault> {
        let vault_dir = vault_dir.as_ref().to_path_buf();
        let data_dir = data_dir.as_ref().to_path_buf();
        fs::create_dir_all(&vault_dir)
            .with_context(|| format!("creating vault dir {}", vault_dir.display()))?;
        fs::create_dir_all(notes_dir(&data_dir))
            .with_context(|| format!("creating data dir {}", data_dir.display()))?;

        let mut vault = Vault {
            vault_dir,
            data_dir,
            notes: Vec::new(),
        };
        vault.load_docs()?;
        // The ledger records which note paths this device materialized on the
        // previous run. It lets us tell a *user deletion* (a path we wrote
        // before that is now gone) apart from a *note freshly arrived via sync*
        // (a document with no file yet — must be materialized, not tombstoned).
        let prev = vault.load_ledger();
        vault.ingest_from_disk(&prev)?;
        vault.materialize()?;
        vault.save_ledger()?;
        vault.persist()?;
        Ok(vault)
    }

    /// Notes that currently exist (tombstones filtered out), sorted by path.
    pub fn live_notes(&self) -> Vec<&Note> {
        let mut v: Vec<&Note> = self.notes.iter().filter(|n| !n.doc.deleted()).collect();
        v.sort_by(|a, b| a.doc.path().cmp(&b.doc.path()));
        v
    }

    pub fn note_by_path(&self, path: &str) -> Option<&Note> {
        self.notes.iter().find(|n| n.doc.path() == path)
    }

    // ---- canonical <-> disk ----

    fn load_docs(&mut self) -> Result<()> {
        let dir = notes_dir(&self.data_dir);
        for entry in fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("automerge") {
                continue;
            }
            let id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let bytes = fs::read(&path)?;
            match NoteDoc::from_bytes(&bytes) {
                Ok(doc) => self.notes.push(Note { id, doc }),
                // don't discard bytes we can't parse — skip and leave on disk
                Err(e) => tracing::warn!("skipping unreadable note {id}: {e}"),
            }
        }
        Ok(())
    }

    /// Fold changes made to the plain files back into the documents: new files
    /// become new notes, edited files splice into the CRDT, removed files
    /// become tombstones. The vault dir is authoritative for local edits.
    fn ingest_from_disk(&mut self, prev_materialized: &HashSet<String>) -> Result<()> {
        let now = now_ms();
        let disk = collect_markdown(&self.vault_dir)?;

        for (rel, content) in &disk {
            match self.index_of_path(rel) {
                Some(i) => {
                    // resurrect if it had been tombstoned, then sync the body
                    if self.notes[i].doc.deleted() {
                        self.notes[i].doc.set_deleted(false, now)?;
                    }
                    self.notes[i].doc.set_body(content, now)?;
                }
                None => {
                    let id = new_note_id();
                    let doc = NoteDoc::create(rel, content, now)?;
                    self.notes.push(Note { id, doc });
                }
            }
        }

        // A live note with no file on disk is a deletion only if *this device*
        // had materialized it before (it is in the ledger). Otherwise it is a
        // note that just arrived via sync and still needs materializing — leave
        // it live.
        for note in &mut self.notes {
            let path = note.doc.path();
            if !note.doc.deleted() && !disk.contains_key(&path) && prev_materialized.contains(&path)
            {
                note.doc.set_deleted(true, now)?;
            }
        }
        Ok(())
    }

    // ---- materialized-paths ledger ----

    fn ledger_path(&self) -> PathBuf {
        self.data_dir.join("materialized.json")
    }

    fn load_ledger(&self) -> HashSet<String> {
        fs::read_to_string(self.ledger_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default()
    }

    pub(crate) fn save_ledger(&self) -> Result<()> {
        let paths: Vec<String> = self
            .notes
            .iter()
            .filter(|n| !n.doc.deleted())
            .map(|n| n.doc.path())
            .collect();
        write_atomic(&self.ledger_path(), serde_json::to_vec(&paths)?.as_slice())?;
        Ok(())
    }

    /// Write every live note's body to its path in the vault (creating parent
    /// folders — this is where deep nesting materializes), and remove the files
    /// of tombstoned notes. Only rewrites files whose content actually changed.
    pub fn materialize(&self) -> Result<()> {
        for note in &self.notes {
            let target = self.vault_dir.join(note.doc.path());
            if note.doc.deleted() {
                let _ = fs::remove_file(&target);
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let body = note.doc.body();
            // Write when the file is missing (so empty notes still appear) or
            // its content differs — never let an unwritten empty note look
            // "unchanged" against a non-existent file.
            if !target.exists() || fs::read_to_string(&target).unwrap_or_default() != body {
                write_atomic(&target, body.as_bytes())?;
            }
        }
        Ok(())
    }

    /// Save every note's canonical document to <data_dir>/notes/<id>.automerge.
    pub fn persist(&mut self) -> Result<()> {
        let dir = notes_dir(&self.data_dir);
        for note in &mut self.notes {
            let bytes = note.doc.to_bytes();
            let path = dir.join(format!("{}.automerge", note.id));
            write_atomic(&path, &bytes)?;
        }
        Ok(())
    }

    // ---- in-app mutations (M4) ----

    /// Create a new note at `rel_path` (relative to the vault). Parent folders
    /// are created on materialize. Errors if a live note already lives there.
    pub fn create_note(&mut self, rel_path: &str, body: &str) -> Result<()> {
        let rel = rel_path.trim().trim_start_matches('/');
        if rel.is_empty() {
            bail!("empty path");
        }
        if self.notes.iter().any(|n| !n.doc.deleted() && n.doc.path() == rel) {
            bail!("a note already exists at {rel}");
        }
        let now = now_ms();
        // resurrect a tombstone at this path rather than orphaning its history
        if let Some(i) = self.index_of_path(rel) {
            self.notes[i].doc.set_deleted(false, now)?;
            self.notes[i].doc.set_body(body, now)?;
        } else {
            let doc = NoteDoc::create(rel, body, now)?;
            self.notes.push(Note { id: new_note_id(), doc });
        }
        self.materialize()?;
        self.save_ledger()?;
        self.persist()?;
        Ok(())
    }

    /// Move/rename a note, keeping its stable id (and history) — the path field
    /// inside the document changes, so the move merges across devices.
    pub fn rename_note(&mut self, old_rel: &str, new_rel: &str) -> Result<()> {
        let new_rel = new_rel.trim().trim_start_matches('/');
        if new_rel.is_empty() {
            bail!("empty path");
        }
        if old_rel == new_rel {
            return Ok(());
        }
        if self.notes.iter().any(|n| !n.doc.deleted() && n.doc.path() == new_rel) {
            bail!("a note already exists at {new_rel}");
        }
        let Some(i) = self.index_of_path(old_rel) else {
            bail!("no note at {old_rel}");
        };
        let now = now_ms();
        self.notes[i].doc.set_path(new_rel, now)?;
        // move the file directly; materialize will also reconcile if needed
        let from = self.vault_dir.join(old_rel);
        let to = self.vault_dir.join(new_rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        let _ = fs::rename(&from, &to);
        self.materialize()?;
        self.save_ledger()?;
        self.persist()?;
        Ok(())
    }

    /// Keep the filename in sync with the note's title (Obsidian-style): if the
    /// first line of the body implies a different, filesystem-safe name that is
    /// free in the same folder, rename to it. Returns the new relative path if
    /// a rename happened. Never clobbers another note.
    pub fn retitle_from_body(&mut self, rel: &str) -> Result<Option<String>> {
        let Some(i) = self.index_of_path(rel) else {
            return Ok(None);
        };
        let Some(title) = title_of(&self.notes[i].doc.body()) else {
            return Ok(None);
        };
        let parent = match rel.rsplit_once('/') {
            Some((p, _)) => format!("{p}/"),
            None => String::new(),
        };
        let new_rel = format!("{parent}{title}.md");
        if new_rel == rel {
            return Ok(None);
        }
        if self.notes.iter().any(|n| !n.doc.deleted() && n.doc.path() == new_rel) {
            return Ok(None); // a different note already owns that name
        }
        self.rename_note(rel, &new_rel)?;
        Ok(Some(new_rel))
    }

    /// Delete a note: tombstone the document (so the deletion propagates via
    /// sync) and remove its file.
    pub fn delete_note(&mut self, rel: &str) -> Result<()> {
        if let Some(i) = self.index_of_path(rel) {
            self.notes[i].doc.set_deleted(true, now_ms())?;
        }
        self.materialize()?;
        self.save_ledger()?;
        self.persist()?;
        Ok(())
    }

    /// Re-fold on-disk edits (e.g. after an $EDITOR session) into the documents.
    pub fn rescan(&mut self) -> Result<()> {
        let prev = self.load_ledger();
        self.ingest_from_disk(&prev)?;
        self.materialize()?;
        self.save_ledger()?;
        self.persist()?;
        Ok(())
    }

    fn index_of_path(&self, path: &str) -> Option<usize> {
        self.notes.iter().position(|n| n.doc.path() == path)
    }
}

fn notes_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("notes")
}

/// The note's title for filename purposes: the first non-empty line, with
/// leading markdown heading marks stripped, sanitized to a safe filename.
/// `None` if there's no usable title.
pub fn title_of(body: &str) -> Option<String> {
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let t = t.trim_start_matches('#').trim();
        let safe = sanitize_filename(t);
        return (!safe.is_empty()).then_some(safe);
    }
    None
}

/// Make a single path component safe: no separators or control chars, no
/// leading/trailing dots or spaces, length-capped. Spaces and unicode are kept
/// (so "Groceries list" → "Groceries list").
fn sanitize_filename(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '/' | '\\' => out.push('-'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let out: String = out.trim().trim_matches('.').trim().chars().take(120).collect();
    out
}

/// Recursively collect `*.md` files under `root`, keyed by their `/`-joined
/// relative path. Hidden entries (dotfiles/dirs) are skipped.
fn collect_markdown(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    collect_into(root, root, &mut out)?;
    Ok(out)
}

fn collect_into(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_into(root, &path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let content = fs::read_to_string(&path).unwrap_or_default();
            out.insert(rel, content);
        }
    }
    Ok(())
}

/// Crash-safe write: temp file in the same directory, then atomic rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    #[test]
    fn ingests_plain_files_into_per_note_docs() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        write(&vault, "inbox.md", "# Inbox\n");
        write(&vault, "Journal/2026-07-13.md", "today");

        let v = Vault::open(&vault, &data).unwrap();
        assert_eq!(v.live_notes().len(), 2);
        assert_eq!(v.note_by_path("inbox.md").unwrap().doc.body(), "# Inbox\n");
        // canonical docs were written
        assert_eq!(fs::read_dir(data.join("notes")).unwrap().count(), 2);
    }

    #[test]
    fn deeply_nested_paths_roundtrip_end_to_end() {
        // six levels deep — infinite nesting, front to back
        let deep = "Projects/client/2026/q3/research/sources/paper.md";

        // Device A authors the deep note and captures it into canonical docs.
        let a = tempfile::tempdir().unwrap();
        let (vault_a, data_a) = (a.path().join("vault"), a.path().join("data"));
        write(&vault_a, deep, "nested body");
        {
            let v = Vault::open(&vault_a, &data_a).unwrap();
            assert_eq!(v.note_by_path(deep).unwrap().doc.path(), deep);
        }

        // Simulate sync delivering A's canonical documents to a fresh device B
        // (an empty vault, no ledger) — exactly what folder/server sync ships.
        let b = tempfile::tempdir().unwrap();
        let (vault_b, data_b) = (b.path().join("vault"), b.path().join("data"));
        fs::create_dir_all(data_b.join("notes")).unwrap();
        for entry in fs::read_dir(data_a.join("notes")).unwrap().flatten() {
            fs::copy(
                entry.path(),
                data_b.join("notes").join(entry.file_name()),
            )
            .unwrap();
        }

        // Device B opens: the note must materialize at the exact deep path,
        // creating every parent folder — not be mistaken for a deletion.
        let v = Vault::open(&vault_b, &data_b).unwrap();
        let file = vault_b.join(deep);
        assert!(file.exists(), "deep note did not materialize at {deep}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "nested body");
        assert_eq!(v.note_by_path(deep).unwrap().doc.body(), "nested body");
    }

    #[test]
    fn edits_persist_across_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        write(&vault, "n.md", "v1");
        Vault::open(&vault, &data).unwrap();

        // external editor changes the file
        write(&vault, "n.md", "v1 plus more");
        let v = Vault::open(&vault, &data).unwrap();
        assert_eq!(v.note_by_path("n.md").unwrap().doc.body(), "v1 plus more");
    }

    #[test]
    fn create_empty_note_materializes_the_file() {
        // regression: an empty new note must still appear on disk (and reload
        // into the tree), not be skipped because "" == "" against no file.
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        let mut v = Vault::open(&vault, &data).unwrap();
        v.create_note("fresh.md", "").unwrap();
        assert!(vault.join("fresh.md").exists(), "empty note was not written");
    }

    #[test]
    fn create_then_rename_keeps_note_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        let mut v = Vault::open(&vault, &data).unwrap();

        v.create_note("a.md", "hello").unwrap();
        assert!(vault.join("a.md").exists());
        let id = v.note_by_path("a.md").unwrap().id.clone();

        v.rename_note("a.md", "Folder/deep/b.md").unwrap();
        assert!(!vault.join("a.md").exists());
        assert_eq!(fs::read_to_string(vault.join("Folder/deep/b.md")).unwrap(), "hello");
        // same document id — history/CRDT lineage preserved across the move
        assert_eq!(v.note_by_path("Folder/deep/b.md").unwrap().id, id);
        assert!(v.note_by_path("a.md").is_none());
    }

    #[test]
    fn rescan_folds_an_external_edit_into_the_document() {
        // models the $EDITOR round-trip: file changes on disk, then rescan()
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        let mut v = Vault::open(&vault, &data).unwrap();
        v.create_note("n.md", "draft").unwrap();

        fs::write(vault.join("n.md"), "draft, now revised").unwrap();
        v.rescan().unwrap();
        assert_eq!(v.note_by_path("n.md").unwrap().doc.body(), "draft, now revised");
    }

    #[test]
    fn title_extraction_strips_heading_and_sanitizes() {
        assert_eq!(title_of("# Groceries\n\nbody").as_deref(), Some("Groceries"));
        assert_eq!(title_of("\n\n  plain first line\nx").as_deref(), Some("plain first line"));
        assert_eq!(title_of("# a/b: notes").as_deref(), Some("a-b: notes"));
        assert_eq!(title_of("#\n\n"), None);
        assert_eq!(title_of(""), None);
    }

    #[test]
    fn retitle_renames_file_to_match_title_keeping_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        let mut v = Vault::open(&vault, &data).unwrap();
        v.create_note("Notes/New note.md", "# New note\n").unwrap();
        let id = v.note_by_path("Notes/New note.md").unwrap().id.clone();

        // user edits the title, then we sync the filename to it
        fs::write(vault.join("Notes/New note.md"), "# Groceries\n\nmilk").unwrap();
        v.rescan().unwrap();
        let new_rel = v.retitle_from_body("Notes/New note.md").unwrap();

        assert_eq!(new_rel.as_deref(), Some("Notes/Groceries.md"));
        assert!(vault.join("Notes/Groceries.md").exists());
        assert!(!vault.join("Notes/New note.md").exists());
        // same folder, same identity, body intact
        assert_eq!(v.note_by_path("Notes/Groceries.md").unwrap().id, id);
        assert!(v.note_by_path("Notes/Groceries.md").unwrap().doc.body().contains("milk"));
    }

    #[test]
    fn retitle_is_noop_when_name_matches_or_would_clobber() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        let mut v = Vault::open(&vault, &data).unwrap();
        // title already matches the filename → no rename
        v.create_note("Groceries.md", "# Groceries\n").unwrap();
        assert_eq!(v.retitle_from_body("Groceries.md").unwrap(), None);

        // another note's title collides with an existing file → keep filename
        v.create_note("other.md", "# Groceries\n").unwrap();
        assert_eq!(v.retitle_from_body("other.md").unwrap(), None);
        assert!(v.note_by_path("other.md").is_some());
    }

    #[test]
    fn delete_note_tombstones_and_removes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        let mut v = Vault::open(&vault, &data).unwrap();
        v.create_note("x.md", "bye").unwrap();
        v.delete_note("x.md").unwrap();
        assert!(!vault.join("x.md").exists());
        assert_eq!(v.live_notes().len(), 0);
        assert!(v.note_by_path("x.md").unwrap().doc.deleted());
    }

    #[test]
    fn removed_file_becomes_tombstone() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let data = tmp.path().join("data");
        write(&vault, "gone.md", "bye");
        Vault::open(&vault, &data).unwrap();

        fs::remove_file(vault.join("gone.md")).unwrap();
        let v = Vault::open(&vault, &data).unwrap();
        assert_eq!(v.live_notes().len(), 0);
        assert!(v.note_by_path("gone.md").unwrap().doc.deleted());
    }
}
