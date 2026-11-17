//! X-02: tokens live in a platform secret store behind this trait.
//!
//! `FileSecretStore` (mode 0600 file in the app's private data dir) is the portable default and
//! what the CLI and tests use. The desktop app wires libsecret / Windows Credential Manager and
//! the Android shell wires the Keystore behind the same trait.

use crate::error::{ClientError, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

pub struct FileSecretStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileSecretStore {
    pub fn new(path: PathBuf) -> Self {
        FileSecretStore { path, lock: Mutex::new(()) }
    }

    fn read(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write(&self, m: &BTreeMap<String, String>) -> Result<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(m)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        Ok(self.read().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = self.read();
        m.insert(key.to_string(), value.to_string());
        self.write(&m)
    }
    fn delete(&self, key: &str) -> Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = self.read();
        m.remove(key);
        self.write(&m)
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<BTreeMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().map_err(|_| ClientError::Local("poisoned".into()))?.get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().map_err(|_| ClientError::Local("poisoned".into()))?.insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().map_err(|_| ClientError::Local("poisoned".into()))?.remove(key);
        Ok(())
    }
}
