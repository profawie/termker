//! Saved passwords and key passphrases.
//!
//! Secrets are kept in the operating system's credential store: Windows
//! Credential Manager, the macOS Keychain, or the Secret Service on Linux.
//! This app only keeps an index of what it saved (names, never secrets) so
//! the user can see the list and remove entries.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

/// Service name the entries are filed under in the credential store.
pub const SERVICE: &str = crate::APP_NAME;

/// One saved secret, as shown to the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSecret {
    /// Entry name in the credential store.
    pub key: String,
    /// What it is, e.g. `Password for root@example.com`.
    pub label: String,
}

/// Entry name for the password of `username` at `address:port`.
pub fn password_key(username: &str, address: &str, port: u16) -> String {
    format!("password:{username}@{address}:{port}")
}

/// Entry name for the passphrase of the private key file at `path`.
pub fn passphrase_key(path: &str) -> String {
    format!("passphrase:{path}")
}

trait Backend: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// The operating system's credential store, for entries filed under
/// `service`.
struct OsBackend {
    service: &'static str,
}

impl OsBackend {
    fn entry(&self, key: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(self.service, key).map_err(|err| err.to_string())
    }
}

impl Backend for OsBackend {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.entry(key)?
            .set_password(secret)
            .map_err(|err| err.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

/// Move the saved secrets listed in the index file at `index_path` from the
/// credential store entries filed under `old_service` to this app's. Used
/// once, after the app was renamed. Returns how many moved and how many
/// could not be moved; the ones that could not be moved stay where they were.
pub fn move_from_service(index_path: &Path, old_service: &'static str) -> (usize, usize) {
    if keyring::Entry::store_status().is_err() {
        return (0, 0);
    }
    let index: Vec<SavedSecret> = fs::read_to_string(index_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    move_entries(
        &index,
        &OsBackend {
            service: old_service,
        },
        &OsBackend { service: SERVICE },
    )
}

fn move_entries(index: &[SavedSecret], from: &dyn Backend, to: &dyn Backend) -> (usize, usize) {
    let (mut moved, mut failed) = (0, 0);
    for saved in index {
        match from.get(&saved.key) {
            // Already moved, or removed outside the app.
            Ok(None) => {}
            Ok(Some(secret)) => match to.set(&saved.key, &secret) {
                Ok(()) => {
                    // The copy is in place, so the old entry can go.
                    let _ = from.delete(&saved.key);
                    moved += 1;
                }
                Err(_) => failed += 1,
            },
            Err(_) => failed += 1,
        }
    }
    (moved, failed)
}

struct Inner {
    backend: Box<dyn Backend>,
    index_path: Option<PathBuf>,
    index: Mutex<Vec<SavedSecret>>,
}

/// Handle to the saved secrets. Cheap to clone.
#[derive(Clone)]
pub struct SecretStore {
    /// `None` when no credential store is available on this system.
    inner: Option<Arc<Inner>>,
}

impl SecretStore {
    /// The operating system's credential store, with the index of saved
    /// entries at `index_path`. Disabled if the system has no usable store.
    pub fn os(index_path: PathBuf) -> Self {
        if keyring::Entry::store_status().is_err() {
            return Self::disabled();
        }
        let index = fs::read_to_string(&index_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            inner: Some(Arc::new(Inner {
                backend: Box::new(OsBackend { service: SERVICE }),
                index_path: Some(index_path),
                index: Mutex::new(index),
            })),
        }
    }

    /// A store that saves nothing and finds nothing.
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Whether secrets can be saved on this system.
    pub fn available(&self) -> bool {
        self.inner.is_some()
    }

    /// The saved secret under `key`, if there is one. May block briefly.
    pub fn get(&self, key: &str) -> Option<String> {
        let inner = self.inner.as_ref()?;
        match inner.backend.get(key) {
            Ok(Some(secret)) => Some(secret),
            Ok(None) => {
                // Removed outside the app; keep the list honest.
                inner.unlist(key);
                None
            }
            Err(_) => None,
        }
    }

    /// Save `secret` under `key`. May block briefly.
    pub fn save(&self, key: &str, label: &str, secret: &str) -> Result<(), String> {
        let Some(inner) = &self.inner else {
            return Err("No credential store is available on this system.".to_string());
        };
        inner.backend.set(key, secret)?;
        let mut index = inner.index.lock().unwrap();
        index.retain(|saved| saved.key != key);
        index.push(SavedSecret {
            key: key.to_string(),
            label: label.to_string(),
        });
        inner.write_index(&index).map_err(|err| err.to_string())
    }

    /// Remove the secret under `key`. Removing one that is not there is fine.
    pub fn forget(&self, key: &str) -> Result<(), String> {
        let Some(inner) = &self.inner else {
            return Ok(());
        };
        inner.backend.delete(key)?;
        inner.unlist(key);
        Ok(())
    }

    /// What has been saved, in the order it was saved.
    pub fn saved(&self) -> Vec<SavedSecret> {
        match &self.inner {
            Some(inner) => inner.index.lock().unwrap().clone(),
            None => Vec::new(),
        }
    }
}

impl Inner {
    fn unlist(&self, key: &str) {
        let mut index = self.index.lock().unwrap();
        let before = index.len();
        index.retain(|saved| saved.key != key);
        if index.len() != before {
            let _ = self.write_index(&index);
        }
    }

    fn write_index(&self, index: &[SavedSecret]) -> io::Result<()> {
        let Some(path) = &self.index_path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(index).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }
}

/// An in-memory credential store for tests.
#[cfg(any(test, feature = "test-support"))]
mod memory {
    use std::collections::BTreeMap;

    use super::*;

    /// In-memory stand-in for the credential store.
    #[derive(Default)]
    pub(crate) struct MemoryBackend {
        pub(crate) entries: Mutex<BTreeMap<String, String>>,
    }

    impl Backend for Arc<MemoryBackend> {
        fn get(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self.entries.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, secret: &str) -> Result<(), String> {
            self.entries
                .lock()
                .unwrap()
                .insert(key.to_string(), secret.to_string());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            self.entries.lock().unwrap().remove(key);
            Ok(())
        }
    }

    impl SecretStore {
        /// A working store that keeps everything in memory.
        pub fn memory() -> Self {
            Self::memory_with_index(None).0
        }

        pub(crate) fn memory_with_index(index_path: Option<PathBuf>) -> (Self, Arc<MemoryBackend>) {
            let backend = Arc::new(MemoryBackend::default());
            let store = Self {
                inner: Some(Arc::new(Inner {
                    backend: Box::new(backend.clone()),
                    index_path,
                    index: Mutex::default(),
                })),
            };
            (store, backend)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_get_and_forget() {
        let store = SecretStore::memory();
        assert!(store.available());
        assert_eq!(store.get("k"), None);
        store.save("k", "Password for a", "one").unwrap();
        store.save("k", "Password for a", "two").unwrap();
        assert_eq!(store.get("k").as_deref(), Some("two"));
        assert_eq!(store.saved().len(), 1);
        store.forget("k").unwrap();
        store.forget("k").unwrap();
        assert_eq!(store.get("k"), None);
        assert!(store.saved().is_empty());
    }

    #[test]
    fn index_file_holds_names_only() {
        let path = crate::testing::temp_dir("secrets").join("secrets.json");
        let (store, _) = SecretStore::memory_with_index(Some(path.clone()));
        store
            .save("password:root@h:22", "Password for root@h", "hunter22")
            .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Password for root@h"));
        assert!(!text.contains("hunter22"));
        store.forget("password:root@h:22").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().trim(), "[]");
    }

    #[test]
    fn entry_removed_outside_the_app_leaves_the_list() {
        let (store, backend) = SecretStore::memory_with_index(None);
        store.save("k", "label", "s").unwrap();
        backend.entries.lock().unwrap().clear();
        assert_eq!(store.get("k"), None);
        assert!(store.saved().is_empty());
    }

    #[test]
    fn disabled_store_saves_nothing() {
        let store = SecretStore::disabled();
        assert!(!store.available());
        assert!(store.save("k", "l", "s").is_err());
        assert_eq!(store.get("k"), None);
        store.forget("k").unwrap();
    }

    #[test]
    fn keys_are_distinct_per_user_host_and_port() {
        assert_eq!(password_key("root", "h", 22), "password:root@h:22");
        assert_ne!(
            password_key("root", "h", 22),
            password_key("root", "h", 2222)
        );
        assert_eq!(passphrase_key("/k/a.key"), "passphrase:/k/a.key");
    }

    #[test]
    fn secrets_move_to_the_new_service() {
        let old = Arc::new(memory::MemoryBackend::default());
        let new = Arc::new(memory::MemoryBackend::default());
        old.set("a", "one").unwrap();
        old.set("b", "two").unwrap();
        old.set("not-listed", "three").unwrap();
        let index = [
            SavedSecret {
                key: "a".to_string(),
                label: "A".to_string(),
            },
            SavedSecret {
                key: "b".to_string(),
                label: "B".to_string(),
            },
            SavedSecret {
                key: "gone".to_string(),
                label: "Gone".to_string(),
            },
        ];
        assert_eq!(move_entries(&index, &old, &new), (2, 0));
        assert_eq!(new.get("a").unwrap().as_deref(), Some("one"));
        assert_eq!(new.get("b").unwrap().as_deref(), Some("two"));
        assert_eq!(old.get("a").unwrap(), None);
        // Only what the app saved is touched.
        assert_eq!(old.get("not-listed").unwrap().as_deref(), Some("three"));
        // Running it again finds nothing left to move.
        assert_eq!(move_entries(&index, &old, &new), (0, 0));
    }
}
