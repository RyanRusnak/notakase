// doc.rs — a single note as an Automerge CRDT document.
//
// Every note is its own document. This is the key difference from todarchy
// (which keeps one big document for all tasks): notes are independent, so two
// devices editing *different* notes never touch the same CRDT, and two devices
// editing the *same* note merge conflict-free at the character level (no
// conflict-copies, unlike plain file sync).
//
// Schema (all peers must agree):
//   version  : int = 1
//   path     : str          — relative path within the vault, e.g.
//                             "Projects/notakase/spec.md". Slashes give
//                             arbitrarily deep nesting; it is a mutable field
//                             so a move/rename is just an edit that merges.
//   body     : Text         — the markdown content (character-wise CRDT)
//   created  : int (ms)
//   modified : int (ms)
//   deleted  : bool         — tombstone; kept so a delete propagates and can
//                             lose to a concurrent edit deterministically.

use anyhow::{anyhow, Result};
use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjType, ReadDoc, ScalarValue, Value, ROOT};

pub const SCHEMA_VERSION: i64 = 1;

pub struct NoteDoc {
    doc: AutoCommit,
}

impl NoteDoc {
    /// Create a brand-new note document at `path` holding `body`.
    pub fn create(path: &str, body: &str, now_ms: i64) -> Result<Self> {
        let mut doc = AutoCommit::new();
        doc.put(ROOT, "version", SCHEMA_VERSION)?;
        doc.put(ROOT, "path", path)?;
        doc.put(ROOT, "created", now_ms)?;
        doc.put(ROOT, "modified", now_ms)?;
        doc.put(ROOT, "deleted", false)?;
        let text = doc.put_object(ROOT, "body", ObjType::Text)?;
        doc.splice_text(&text, 0, 0, body)?;
        Ok(Self { doc })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            doc: AutoCommit::load(bytes)?,
        })
    }

    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    /// Merge another copy of this note into ours (CRDT union). Concurrent edits
    /// to the body interleave; concurrent scalar edits resolve deterministically.
    pub fn merge(&mut self, other: &mut NoteDoc) -> Result<()> {
        self.doc.merge(&mut other.doc)?;
        Ok(())
    }

    /// Content-addressed change fingerprint — cheap way to tell if a merge or
    /// edit actually changed anything.
    pub fn heads(&mut self) -> Vec<automerge::ChangeHash> {
        self.doc.get_heads()
    }

    // ---- field accessors ----

    pub fn path(&self) -> String {
        self.scalar_str("path").unwrap_or_default()
    }

    pub fn body(&self) -> String {
        match self.doc.get(ROOT, "body") {
            Ok(Some((_, id))) => self.doc.text(&id).unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn created(&self) -> i64 {
        self.scalar_i64("created").unwrap_or(0)
    }

    pub fn modified(&self) -> i64 {
        self.scalar_i64("modified").unwrap_or(0)
    }

    pub fn deleted(&self) -> bool {
        self.scalar_bool("deleted").unwrap_or(false)
    }

    // ---- mutations ----

    /// Replace the body with `new`, applying the change as a minimal text
    /// splice so concurrent edits from another device still merge cleanly.
    /// Returns true if anything changed.
    pub fn set_body(&mut self, new: &str, now_ms: i64) -> Result<bool> {
        let (_, id) = self
            .doc
            .get(ROOT, "body")?
            .ok_or_else(|| anyhow!("note has no body object"))?;
        let cur = self.doc.text(&id)?;
        if cur == new {
            return Ok(false);
        }
        let (pos, del, ins) = diff_splice(&cur, new);
        self.doc.splice_text(&id, pos, del, &ins)?;
        self.doc.put(ROOT, "modified", now_ms)?;
        Ok(true)
    }

    /// Move/rename: change the relative path (parent folders and depth are
    /// encoded in the string, so nesting is unbounded).
    pub fn set_path(&mut self, path: &str, now_ms: i64) -> Result<()> {
        self.doc.put(ROOT, "path", path)?;
        self.doc.put(ROOT, "modified", now_ms)?;
        Ok(())
    }

    pub fn set_deleted(&mut self, deleted: bool, now_ms: i64) -> Result<()> {
        self.doc.put(ROOT, "deleted", deleted)?;
        self.doc.put(ROOT, "modified", now_ms)?;
        Ok(())
    }

    // ---- scalar helpers ----

    fn scalar_str(&self, key: &str) -> Option<String> {
        match self.doc.get(ROOT, key).ok().flatten()?.0 {
            Value::Scalar(s) => match s.as_ref() {
                ScalarValue::Str(v) => Some(v.to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    fn scalar_i64(&self, key: &str) -> Option<i64> {
        match self.doc.get(ROOT, key).ok().flatten()?.0 {
            Value::Scalar(s) => match s.as_ref() {
                ScalarValue::Int(v) => Some(*v),
                ScalarValue::Uint(v) => Some(*v as i64),
                ScalarValue::Timestamp(v) => Some(*v),
                _ => None,
            },
            _ => None,
        }
    }

    fn scalar_bool(&self, key: &str) -> Option<bool> {
        match self.doc.get(ROOT, key).ok().flatten()?.0 {
            Value::Scalar(s) => match s.as_ref() {
                ScalarValue::Boolean(v) => Some(*v),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Compute a minimal single-splice edit turning `cur` into `new`: shared prefix
/// and suffix are left untouched, only the differing middle is spliced. Works
/// in Unicode scalar (char) positions, which is what Automerge text uses.
fn diff_splice(cur: &str, new: &str) -> (usize, isize, String) {
    let a: Vec<char> = cur.chars().collect();
    let b: Vec<char> = new.chars().collect();

    let mut p = 0;
    while p < a.len() && p < b.len() && a[p] == b[p] {
        p += 1;
    }
    let mut s = 0;
    while s < a.len() - p && s < b.len() - p && a[a.len() - 1 - s] == b[b.len() - 1 - s] {
        s += 1;
    }
    let del = (a.len() - p - s) as isize;
    let ins: String = b[p..b.len() - s].iter().collect();
    (p, del, ins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_roundtrips_fields() {
        let mut d = NoteDoc::create("A/B/c.md", "# hi\n\nbody", 100).unwrap();
        assert_eq!(d.path(), "A/B/c.md");
        assert_eq!(d.body(), "# hi\n\nbody");
        assert_eq!(d.created(), 100);
        assert!(!d.deleted());
        let bytes = d.to_bytes();
        let re = NoteDoc::from_bytes(&bytes).unwrap();
        assert_eq!(re.body(), "# hi\n\nbody");
        assert_eq!(re.path(), "A/B/c.md");
    }

    #[test]
    fn set_body_updates_and_reports_change() {
        let mut d = NoteDoc::create("n.md", "hello world", 1).unwrap();
        assert!(d.set_body("hello brave world", 2).unwrap());
        assert_eq!(d.body(), "hello brave world");
        assert_eq!(d.modified(), 2);
        // no-op edit reports false
        assert!(!d.set_body("hello brave world", 3).unwrap());
    }

    #[test]
    fn concurrent_edits_to_one_note_merge_without_conflict() {
        // shared ancestor
        let mut base = NoteDoc::create("n.md", "the quick fox", 1).unwrap();
        let bytes = base.to_bytes();

        // device A prepends, device B appends — disjoint edits
        let mut a = NoteDoc::from_bytes(&bytes).unwrap();
        a.set_body("the very quick fox", 2).unwrap();
        let mut b = NoteDoc::from_bytes(&bytes).unwrap();
        b.set_body("the quick fox jumps", 2).unwrap();

        a.merge(&mut b).unwrap();
        let merged = a.body();
        // both edits survived the merge
        assert!(merged.contains("very"), "A's edit lost: {merged}");
        assert!(merged.contains("jumps"), "B's edit lost: {merged}");
    }
}
