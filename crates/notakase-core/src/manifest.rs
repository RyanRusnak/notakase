// manifest.rs — the index of which notes exist, for server-relay sync.
//
// A shared folder can be listed to discover note files, but the relay only
// serves documents by id (there is no "list" endpoint). So we keep a manifest
// document — itself an Automerge CRDT — that maps every note id to `true`.
// Merging two manifests unions the id sets, so devices discover each other's
// notes. Ids are opaque (`note_<random>`); we never remove them (a deleted
// note keeps its tombstoned document, which still syncs).

use anyhow::Result;
use automerge::transaction::Transactable;
use automerge::{AutoCommit, ChangeHash, ReadDoc, ROOT};

/// Note ids are stored as top-level document keys (each `note_… = true`). This
/// matters for merge correctness: a nested map object created independently on
/// two devices would produce *conflicting* container objects when merged; plain
/// scalar puts to distinct keys always union cleanly. Ids are recognised by the
/// `note_` prefix, so the reserved `version` key is never mistaken for one.
pub struct Manifest {
    doc: AutoCommit,
}

const ID_PREFIX: &str = "note_";

impl Default for Manifest {
    fn default() -> Self {
        Self::new()
    }
}

impl Manifest {
    pub fn new() -> Self {
        let mut doc = AutoCommit::new();
        let _ = doc.put(ROOT, "version", 1_i64);
        Manifest { doc }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Manifest {
            doc: AutoCommit::load(bytes)?,
        })
    }

    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    pub fn merge(&mut self, other: &mut Manifest) -> Result<()> {
        self.doc.merge(&mut other.doc)?;
        Ok(())
    }

    pub fn heads(&mut self) -> Vec<ChangeHash> {
        self.doc.get_heads()
    }

    /// Record a note id. Returns true if it wasn't already present.
    pub fn add(&mut self, id: &str) -> Result<bool> {
        if self.doc.get(ROOT, id)?.is_some() {
            return Ok(false);
        }
        self.doc.put(ROOT, id, true)?;
        Ok(true)
    }

    /// Every note id known to the manifest.
    pub fn ids(&self) -> Vec<String> {
        self.doc
            .keys(ROOT)
            .filter(|k| k.starts_with(ID_PREFIX))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_list_ids() {
        let mut m = Manifest::new();
        assert!(m.add("note_a").unwrap());
        assert!(!m.add("note_a").unwrap()); // idempotent
        assert!(m.add("note_b").unwrap());
        let mut ids = m.ids();
        ids.sort();
        assert_eq!(ids, vec!["note_a".to_string(), "note_b".to_string()]);
    }

    #[test]
    fn merge_unions_id_sets_across_devices() {
        let mut base = Manifest::new();
        base.add("note_shared").unwrap();
        let bytes = base.to_bytes();

        let mut a = Manifest::from_bytes(&bytes).unwrap();
        a.add("note_from_a").unwrap();
        let mut b = Manifest::from_bytes(&bytes).unwrap();
        b.add("note_from_b").unwrap();

        a.merge(&mut b).unwrap();
        let mut ids = a.ids();
        ids.sort();
        assert_eq!(ids, vec!["note_from_a", "note_from_b", "note_shared"]);
    }
}
