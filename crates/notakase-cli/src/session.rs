// session.rs — open the notakase vault the same way the TUI does and drive
// sync headlessly (a no-op when the vault is local-only). Shared shape with
// notakase-mcp's session.

use anyhow::Result;
use notakase_core::cryptobox::KEY_BYTES;
use notakase_core::{config, Config, Vault};

pub struct Session {
    pub vault: Vault,
    cfg: Config,
    key: Option<[u8; KEY_BYTES]>,
}

impl Session {
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

    pub async fn sync(&mut self) {
        if let Some(folder) = self.cfg.sync_folder_path() {
            let _ = self.vault.sync_folder(&folder, self.key.as_ref());
        }
        if !self.cfg.server_base_url.trim().is_empty() && !self.cfg.server_manifest_id.trim().is_empty() {
            if let Ok(mut server) =
                notakase_core::ServerSync::new(&self.cfg.server_base_url, &self.cfg.server_manifest_id, self.key)
            {
                let _ = server.sync(&mut self.vault).await;
            }
        }
    }
}
