// session.rs — open the notakase vault the same way the TUI does and drive
// sync headlessly. Reads pull the latest; call `sync()` again after a mutation
// to push. All sync is a no-op when the vault is local-only (no folder/relay
// configured), so the MCP works with zero setup.

use anyhow::Result;
use notakase_core::cryptobox::KEY_BYTES;
use notakase_core::{config, Config, Vault};

pub struct Session {
    pub vault: Vault,
    cfg: Config,
    key: Option<[u8; KEY_BYTES]>,
}

impl Session {
    /// Open the vault (ingesting on-disk edits) and pull from any configured
    /// sync transport.
    pub async fn open() -> Result<Session> {
        let cfg = Config::load();
        let vault = Vault::open_headless(cfg.resolved_vault_dir(), config::data_dir())?;
        let key = if cfg.encrypt {
            notakase_core::vaultkey::load_or_create(&notakase_core::keystore::LibSecretKeyStore::new()).ok()
        } else {
            None
        };
        let mut s = Session { vault, cfg, key };
        s.sync().await;
        Ok(s)
    }

    /// Bidirectional sync across configured transports (folder + relay).
    /// No-op when local-only.
    pub async fn sync(&mut self) {
        if let Some(folder) = self.cfg.sync_folder_path() {
            if let Err(e) = self.vault.sync_folder(&folder, self.key.as_ref()) {
                tracing::warn!("folder sync failed: {e}");
            }
        }
        if !self.cfg.server_base_url.trim().is_empty() && !self.cfg.server_manifest_id.trim().is_empty() {
            match notakase_core::ServerSync::new(&self.cfg.server_base_url, &self.cfg.server_manifest_id, self.key) {
                Ok(mut server) => {
                    if let Err(e) = server.sync(&mut self.vault).await {
                        tracing::warn!("server sync failed: {e}");
                    }
                }
                Err(e) => tracing::warn!("server sync init failed: {e}"),
            }
        }
    }
}
