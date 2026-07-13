// server_sync.rs — sync the vault through a self-hosted relay (notakase-server).
//
// The relay stores opaque documents by id (GET/PUT/DELETE /doc/:id) with ETag
// caching; it has no "list". So one document is the manifest (id set), and each
// note is a document keyed by its note id. A sync is:
//
//   1. GET + merge the manifest, add our local ids, PUT it back if it changed.
//   2. For every id (local ∪ manifest): GET + merge the note document.
//   3. Materialize any pulled changes to plain files.
//   4. PUT every note whose local state is ahead of what the relay has.
//
// "Ahead of the relay" is judged by comparing Automerge heads to a per-session
// record of what we last saw/sent (`server_known`) — so idle syncs are 304s and
// cost nothing, and we never PUT a note the relay already has. Everything is
// sealed with the vault key when encryption is on.

use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use automerge::ChangeHash;

use crate::cryptobox::{self, KEY_BYTES};
use crate::doc::NoteDoc;
use crate::folder_sync::SyncReport;
use crate::manifest::Manifest;
use crate::server_client::ServerSyncClient;
use crate::store::{Note, Vault};

pub struct ServerSync {
    client: ServerSyncClient,
    manifest_id: String,
    key: Option<[u8; KEY_BYTES]>,
    manifest: Manifest,
    /// Note-id → the heads we know the relay holds (updated on GET-200 and PUT).
    server_known: HashMap<String, Vec<ChangeHash>>,
    /// Manifest heads we last pushed, so an unchanged manifest isn't re-PUT.
    manifest_known: Option<Vec<ChangeHash>>,
}

impl ServerSync {
    pub fn new(base_url: &str, manifest_id: &str, key: Option<[u8; KEY_BYTES]>) -> Result<Self> {
        Ok(ServerSync {
            client: ServerSyncClient::new(base_url)?,
            manifest_id: manifest_id.to_string(),
            key,
            manifest: Manifest::new(),
            server_known: HashMap::new(),
            manifest_known: None,
        })
    }

    pub async fn healthz(&self) -> bool {
        self.client.healthz().await
    }

    pub async fn sync(&mut self, vault: &mut Vault) -> Result<SyncReport> {
        let mut report = SyncReport::default();

        // ---- 1. manifest ----
        if let Some((bytes, _)) = self.client.get(&self.manifest_id).await? {
            if let Some(plain) = self.open(&bytes) {
                if let Ok(mut remote) = Manifest::from_bytes(&plain) {
                    self.manifest.merge(&mut remote)?;
                }
            }
        }
        for note in &vault.notes {
            self.manifest.add(&note.id)?;
        }
        let mheads = self.manifest.heads();
        if self.manifest_known.as_ref() != Some(&mheads) {
            let bytes = self.manifest.to_bytes();
            let payload = self.seal(bytes);
            self.client.put(&self.manifest_id, payload).await?;
            self.manifest_known = Some(mheads);
        }

        // ---- 2. pull every known note ----
        let ids: BTreeSet<String> = vault
            .notes
            .iter()
            .map(|n| n.id.clone())
            .chain(self.manifest.ids())
            .collect();

        for id in &ids {
            let Some((bytes, _)) = self.client.get(id).await? else {
                continue; // 304 (already have it) or 404 (not there / no key)
            };
            let Some(plain) = self.open(&bytes) else {
                continue; // can't decrypt — leave it, don't destroy it
            };
            let mut remote = match NoteDoc::from_bytes(&plain) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let rheads = remote.heads();
            match vault.notes.iter().position(|n| &n.id == id) {
                Some(i) => {
                    let before = vault.notes[i].doc.heads();
                    vault.notes[i].doc.merge(&mut remote)?;
                    if vault.notes[i].doc.heads() != before {
                        report.pulled += 1;
                        report.changed = true;
                    }
                }
                None => {
                    vault.notes.push(Note { id: id.clone(), doc: remote });
                    report.pulled += 1;
                    report.changed = true;
                }
            }
            self.server_known.insert(id.clone(), rheads);
        }

        // ---- 3. materialize pulled changes ----
        if report.changed {
            vault.materialize()?;
            vault.save_ledger()?;
        }

        // ---- 4. push notes the relay is behind on ----
        for note in &mut vault.notes {
            let lheads = note.doc.heads();
            if self.server_known.get(&note.id) == Some(&lheads) {
                continue;
            }
            let payload = self.seal(note.doc.to_bytes());
            self.client.put(&note.id, payload).await?;
            self.server_known.insert(note.id.clone(), lheads);
            report.pushed += 1;
        }

        vault.persist()?;
        Ok(report)
    }

    fn seal(&self, bytes: Vec<u8>) -> Vec<u8> {
        match &self.key {
            Some(k) => cryptobox::seal(&bytes, k),
            None => bytes,
        }
    }

    fn open(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        match &self.key {
            Some(k) => cryptobox::open(bytes, k).ok(),
            None => Some(bytes.to_vec()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    /// A minimal in-memory stand-in for notakase-server: stores PUT bytes by
    /// doc id, serves them on GET with ETag + If-None-Match (304) support.
    #[derive(Clone, Default)]
    struct Relay {
        store: Arc<Mutex<HashMap<String, (Vec<u8>, u64)>>>,
    }

    impl Respond for Relay {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let path = req.url.path().to_string();
            if path.ends_with("/healthz") {
                return ResponseTemplate::new(200).set_body_string("ok");
            }
            let id = path.rsplit('/').next().unwrap_or("").to_string();
            let method = req.method.to_string().to_uppercase();
            let mut store = self.store.lock().unwrap();
            match method.as_str() {
                "GET" => match store.get(&id) {
                    Some((bytes, ver)) => {
                        let etag = format!("\"{ver}\"");
                        let inm = req
                            .headers
                            .get("if-none-match")
                            .and_then(|v| v.to_str().ok());
                        if inm == Some(etag.as_str()) {
                            ResponseTemplate::new(304)
                        } else {
                            ResponseTemplate::new(200)
                                .insert_header("ETag", etag.as_str())
                                .set_body_bytes(bytes.clone())
                        }
                    }
                    None => ResponseTemplate::new(404).set_body_string("{}"),
                },
                "PUT" => {
                    let ver = store.get(&id).map(|(_, v)| v + 1).unwrap_or(1);
                    store.insert(id, (req.body.clone(), ver));
                    ResponseTemplate::new(204).insert_header("ETag", format!("\"{ver}\"").as_str())
                }
                "DELETE" => {
                    store.remove(&id);
                    ResponseTemplate::new(204)
                }
                _ => ResponseTemplate::new(400),
            }
        }
    }

    async fn relay_server() -> (MockServer, String) {
        let server = MockServer::start().await;
        Mock::given(any()).respond_with(Relay::default()).mount(&server).await;
        let uri = server.uri();
        (server, uri)
    }

    fn vault_with(tmp: &Path, name: &str, files: &[(&str, &str)]) -> Vault {
        let vault = tmp.join(format!("{name}-v"));
        let data = tmp.join(format!("{name}-d"));
        std::fs::create_dir_all(&vault).unwrap();
        for (rel, body) in files {
            let p = vault.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        Vault::open(&vault, &data).unwrap()
    }

    #[tokio::test]
    async fn two_devices_sync_notes_through_the_relay() {
        let (_server, base) = relay_server().await;
        let tmp = tempfile::tempdir().unwrap();

        // A authors a nested note and pushes it to the relay
        let mut a = vault_with(tmp.path(), "a", &[("Projects/x/plan.md", "the plan")]);
        let mut sa = ServerSync::new(&base, "manifest_main", None).unwrap();
        let rep = sa.sync(&mut a).await.unwrap();
        assert!(rep.pushed >= 1);

        // B starts empty, discovers the note via the manifest, pulls it
        let mut b = vault_with(tmp.path(), "b", &[]);
        let mut sb = ServerSync::new(&base, "manifest_main", None).unwrap();
        let rep = sb.sync(&mut b).await.unwrap();
        assert!(rep.pulled >= 1 && rep.changed);
        assert_eq!(
            std::fs::read_to_string(b.vault_dir.join("Projects/x/plan.md")).unwrap(),
            "the plan"
        );

        // idempotent: re-sync on A pushes nothing (relay already has it)
        let rep = sa.sync(&mut a).await.unwrap();
        assert_eq!(rep.pushed, 0);
        assert_eq!(rep.pulled, 0);
    }

    #[tokio::test]
    async fn encrypted_relay_sync_requires_the_key() {
        let (_server, base) = relay_server().await;
        let tmp = tempfile::tempdir().unwrap();
        let key = cryptobox::generate_key();

        let mut a = vault_with(tmp.path(), "a", &[("secret.md", "top secret")]);
        ServerSync::new(&base, "m_enc", Some(key)).unwrap().sync(&mut a).await.unwrap();

        // right key → reads
        let mut b = vault_with(tmp.path(), "b", &[]);
        ServerSync::new(&base, "m_enc", Some(key)).unwrap().sync(&mut b).await.unwrap();
        assert_eq!(std::fs::read_to_string(b.vault_dir.join("secret.md")).unwrap(), "top secret");

        // wrong key → can't decrypt the manifest or the note, pulls nothing
        let mut c = vault_with(tmp.path(), "c", &[]);
        let rep = ServerSync::new(&base, "m_enc", Some(cryptobox::generate_key()))
            .unwrap()
            .sync(&mut c)
            .await
            .unwrap();
        assert_eq!(rep.pulled, 0);
    }
}
