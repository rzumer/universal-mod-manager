//! Secrets UMM keeps for the user (the Nexus Mods API key): in Windows' Credential Manager,
//! never in the data folder. Tests use an in-memory store.

use std::collections::HashMap;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// Kept in memory only, for tests and systems without a credential store.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

/// Windows' Credential Manager, one entry per secret and data folder (so a second data
/// folder, a test one say, has its own).
#[cfg(windows)]
pub struct SystemSecrets {
    scope: String,
}

#[cfg(windows)]
impl SystemSecrets {
    pub fn new(data_root: &std::path::Path) -> Self {
        SystemSecrets {
            scope: data_root.display().to_string().to_lowercase(),
        }
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new("UMM", &format!("{name} ({})", self.scope)).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
impl SecretStore for SystemSecrets {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        match self.entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.entry(name)?
            .set_password(value)
            .map_err(|e| e.to_string())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// The platform's store for `data_root`.
pub fn system_secrets(data_root: &std::path::Path) -> Box<dyn SecretStore> {
    #[cfg(windows)]
    {
        Box::new(SystemSecrets::new(data_root))
    }
    #[cfg(not(windows))]
    {
        let _ = data_root;
        Box::new(MemorySecrets::default())
    }
}
