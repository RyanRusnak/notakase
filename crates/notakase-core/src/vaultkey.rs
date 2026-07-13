// vaultkey.rs — the single symmetric key that encrypts synced copies of the
// vault. Stored in the OS keyring (libsecret) under one fixed account. To sync
// encrypted across devices, the same key must be present on each — export it as
// a share link and import on the other device.

use anyhow::Result;

use crate::cryptobox::{self, KEY_BYTES};
use crate::keystore::KeyStore;

/// Keyring account holding the vault-wide key.
pub const VAULT_ACCOUNT: &str = "vault";

/// Load the vault key, generating and persisting one on first use.
pub fn load_or_create(ks: &dyn KeyStore) -> Result<[u8; KEY_BYTES]> {
    if let Some(k) = ks.load(VAULT_ACCOUNT)? {
        return Ok(k);
    }
    let k = cryptobox::generate_key();
    ks.save(VAULT_ACCOUNT, &k)?;
    Ok(k)
}

/// Import a key received from another device (e.g. decoded from a share link).
pub fn set(ks: &dyn KeyStore, key: &[u8; KEY_BYTES]) -> Result<()> {
    ks.save(VAULT_ACCOUNT, key)?;
    Ok(())
}
