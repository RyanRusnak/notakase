// recover.rs — un-tombstone notes. Reads every note doc, and for any that is
// tombstoned (deleted=true) but still has a path + body in its Automerge doc,
// resurrects it via create_note (which un-deletes with a fresh timestamp so it
// wins any later merge). Then persists + materializes.
//
//   cargo run -p notakase-core --example recover

use anyhow::Result;
use notakase_core::{config, Config, Vault};

fn main() -> Result<()> {
    let cfg = Config::load();
    let vault_dir = cfg.resolved_vault_dir();
    let data_dir = config::data_dir();
    println!("vault_dir = {}", vault_dir.display());
    println!("data_dir  = {}", data_dir.display());

    let mut vault = Vault::open(&vault_dir, &data_dir)?;

    // Collect tombstoned notes' path + body from the loaded docs.
    let dead: Vec<(String, String)> = vault
        .notes
        .iter()
        .filter(|n| n.doc.deleted())
        .map(|n| (n.doc.path(), n.doc.body()))
        .filter(|(p, _)| !p.is_empty())
        .collect();

    println!("tombstoned notes with recoverable data: {}", dead.len());
    let mut restored = 0;
    for (path, body) in &dead {
        match vault.create_note(path, body) {
            Ok(()) => {
                restored += 1;
                println!("  ✓ restored {path} ({} bytes)", body.len());
            }
            Err(e) => eprintln!("  ✗ {path}: {e}"),
        }
    }
    vault.persist()?;
    vault.materialize()?;
    println!("restored {restored}; live notes now: {}", vault.live_notes().len());
    Ok(())
}
