// config.rs — hand-edited TOML config, true to the "no settings screen" ethos.
// Lives at ~/.config/notakase/config.toml. Every field is optional; an absent
// file means "local only, default vault".

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// The vault: a directory of plain markdown files. Defaults to
    /// ~/Documents/notakase.
    #[serde(default)]
    pub vault_dir: String,
    /// A folder the OS keeps in sync across devices.
    #[serde(default)]
    pub sync_folder: String,
    /// A self-hosted notakase relay (server sync).
    #[serde(default)]
    pub server_base_url: String,
    /// Manifest doc id at the relay (identical across your devices).
    #[serde(default)]
    pub server_manifest_id: String,
    /// Encrypt synced copies with ChaCha20-Poly1305 (recommended for any
    /// third-party sync folder or relay).
    #[serde(default)]
    pub encrypt: bool,
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("notakase/config.toml")
    }

    /// Load config, tolerating an absent or malformed file (returns defaults).
    pub fn load() -> Config {
        let path = Self::path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Config::default();
        };
        toml::from_str(&text).unwrap_or_default()
    }

    /// The resolved vault directory (config value or the default).
    pub fn resolved_vault_dir(&self) -> PathBuf {
        if self.vault_dir.trim().is_empty() {
            home().join("Documents/notakase")
        } else {
            PathBuf::from(expand_tilde(&self.vault_dir))
        }
    }

    /// The sync folder, tilde-expanded — `None` when unset (local-only).
    pub fn sync_folder_path(&self) -> Option<PathBuf> {
        let s = self.sync_folder.trim();
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(expand_tilde(s)))
        }
    }
}

/// Where the canonical per-note Automerge docs live (never synced directly;
/// the vault of plain files and the encrypted mirrors are the synced views).
pub fn data_dir() -> PathBuf {
    dirs_data().join("notakase")
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        return home().join(rest).to_string_lossy().into_owned();
    }
    p.to_string()
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| home().join(".config"))
}

fn dirs_data() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| home().join(".local/share"))
}
