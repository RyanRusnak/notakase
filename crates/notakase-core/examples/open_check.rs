// open_check.rs — exercise the TUI's exact open path (Vault::open, which
// ingests on-disk edits and runs deletion detection) twice in a row, to prove
// a quit+reopen no longer tombstones notes. Prints the live set each pass.
//
//   cargo run --release -p notakase-core --example open_check

use anyhow::Result;
use notakase_core::{config, Config, Vault};

fn pass(label: &str, vault_dir: &std::path::Path, data_dir: &std::path::Path) -> Result<()> {
    let vault = Vault::open(vault_dir, data_dir)?;
    let mut live: Vec<String> = vault.live_notes().iter().map(|n| n.doc.path()).collect();
    live.sort();
    let has_neovim = live.iter().any(|p| p.contains("neovim"));
    println!("[{label}] live={} neovim_present={}", live.len(), has_neovim);
    for p in &live {
        println!("    · {p}");
    }
    Ok(())
}

fn main() -> Result<()> {
    let cfg = Config::load();
    let vault_dir = cfg.resolved_vault_dir();
    let data_dir = config::data_dir();
    println!("vault_dir = {}", vault_dir.display());
    pass("open #1", &vault_dir, &data_dir)?;
    pass("open #2 (after quit+reopen)", &vault_dir, &data_dir)?;
    Ok(())
}
