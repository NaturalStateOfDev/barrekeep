//! Encrypted persistence for credentials (Sling token; later: email +
//! password for autofill). Uses tauri-plugin-stronghold's Rust API
//! directly — we don't go through its JS bridge because the token
//! has to be available to Rust commands at app start (no JS
//! round-trip).
//!
//! The snapshot lives at `<app_data_dir>/barrekeep.stronghold`,
//! encrypted with a static-derived key. Threat model: single-user
//! local laptop. The key being baked into the binary is acceptable
//! because the binary is readable only by the same user. If we ever
//! ship to a multi-user laptop, swap the static derivation for an
//! OS-credential-derived key.

use anyhow::{anyhow, Context, Result};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_stronghold::stronghold::Stronghold;

const CLIENT_PATH: &[u8] = b"barrekeep";

pub const KEY_SLING_TOKEN: &[u8] = b"sling_token";
pub const KEY_SLING_EMAIL: &[u8] = b"sling_email";
pub const KEY_SLING_PASSWORD: &[u8] = b"sling_password";

pub struct Secrets(Mutex<Stronghold>);

impl Secrets {
    pub fn open(app: &AppHandle) -> Result<Self> {
        let dir = app
            .path()
            .app_data_dir()
            .context("app_data_dir unavailable")?;
        std::fs::create_dir_all(&dir).context("create app data dir")?;
        let snapshot_path = dir.join("barrekeep.stronghold");

        let password = derive_key(b"barrekeep-default");
        let s = Stronghold::new(&snapshot_path, password).context("open stronghold")?;

        // Ensure a client exists. load_client succeeds only if the snapshot
        // already contained one; otherwise we create a fresh client.
        if s.load_client(CLIENT_PATH.to_vec()).is_err() {
            s.create_client(CLIENT_PATH.to_vec())
                .context("create stronghold client")?;
        }
        Ok(Self(Mutex::new(s)))
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<String>> {
        let s = self.0.lock().map_err(|_| anyhow!("secrets poisoned"))?;
        let client = s
            .get_client(CLIENT_PATH.to_vec())
            .context("get stronghold client")?;
        let raw = client.store().get(key).context("stronghold store.get")?;
        Ok(raw.map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    pub fn set(&self, key: &[u8], value: &str) -> Result<()> {
        let s = self.0.lock().map_err(|_| anyhow!("secrets poisoned"))?;
        let client = s
            .get_client(CLIENT_PATH.to_vec())
            .context("get stronghold client")?;
        client
            .store()
            .insert(key.to_vec(), value.as_bytes().to_vec(), None)
            .context("stronghold store.insert")?;
        s.write_client(CLIENT_PATH.to_vec())
            .context("write client to snapshot")?;
        s.save().context("commit snapshot")?;
        Ok(())
    }

    pub fn remove(&self, key: &[u8]) -> Result<()> {
        let s = self.0.lock().map_err(|_| anyhow!("secrets poisoned"))?;
        let client = s
            .get_client(CLIENT_PATH.to_vec())
            .context("get stronghold client")?;
        client
            .store()
            .delete(key)
            .context("stronghold store.delete")?;
        s.write_client(CLIENT_PATH.to_vec())
            .context("write client to snapshot")?;
        s.save().context("commit snapshot")?;
        Ok(())
    }
}

fn derive_key(password: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"barrekeep-v1-vault-key:");
    hasher.update(password);
    hasher.finalize().to_vec()
}
