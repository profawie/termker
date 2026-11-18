//! Trusted host keys, in the OpenSSH `known_hosts` format.
//!
//! The file belongs to rterminalfy (in its config folder), so the user's
//! `~/.ssh/known_hosts` is never modified.

use std::{fs, io, path::PathBuf};

use russh::keys::{HashAlg, PublicKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// This key is already trusted for this host.
    Trusted,
    /// This host has never been seen before.
    Unknown,
    /// A key is saved for this host, but it is not the one presented now.
    /// This may be a man-in-the-middle attack.
    Changed,
}

#[derive(Debug, Clone)]
pub struct KnownHosts {
    path: PathBuf,
}

impl KnownHosts {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn entries(&self) -> io::Result<Vec<(String, PublicKey)>> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        Ok(text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                let (pattern, key) = line.split_once(' ')?;
                let key = PublicKey::from_openssh(key.trim()).ok()?;
                Some((pattern.to_string(), key))
            })
            .collect())
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> io::Result<HostKeyStatus> {
        let pattern = pattern(host, port);
        let mut seen = false;
        for (entry, known) in self.entries()? {
            if entry != pattern {
                continue;
            }
            if known.key_data() == key.key_data() {
                return Ok(HostKeyStatus::Trusted);
            }
            seen = true;
        }
        Ok(if seen {
            HostKeyStatus::Changed
        } else {
            HostKeyStatus::Unknown
        })
    }

    /// Trust `key` for this host, replacing any older key.
    pub fn trust(&self, host: &str, port: u16, key: &PublicKey) -> io::Result<()> {
        let pattern = pattern(host, port);
        let mut lines: Vec<String> = match fs::read_to_string(&self.path) {
            Ok(text) => text
                .lines()
                .filter(|line| line.split_once(' ').map(|(entry, _)| entry) != Some(&pattern))
                .map(str::to_string)
                .collect(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err),
        };
        let key = PublicKey::new(key.key_data().clone(), "");
        let encoded = key.to_openssh().map_err(io::Error::other)?;
        lines.push(format!("{pattern} {}", encoded.trim()));

        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, lines.join("\n") + "\n")?;
        fs::rename(&tmp, &self.path)
    }
}

/// `host` for port 22, `[host]:port` for other ports, like OpenSSH.
fn pattern(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// SHA-256 fingerprint, e.g. `SHA256:abc...`, as printed by
/// `ssh-keygen -lf`.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey};

    fn key() -> PublicKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone()
    }

    fn store(name: &str) -> KnownHosts {
        KnownHosts::new(
            std::env::temp_dir()
                .join(format!("rterminalfy-kh-{}-{name}", uuid::Uuid::new_v4()))
                .join("known_hosts"),
        )
    }

    #[test]
    fn unknown_then_trusted_after_trust() {
        let store = store("trust");
        let key = key();
        assert_eq!(
            store.check("a.test", 22, &key).unwrap(),
            HostKeyStatus::Unknown
        );
        store.trust("a.test", 22, &key).unwrap();
        assert_eq!(
            store.check("a.test", 22, &key).unwrap(),
            HostKeyStatus::Trusted
        );
    }

    #[test]
    fn different_key_for_known_host_is_changed() {
        let store = store("changed");
        store.trust("a.test", 22, &key()).unwrap();
        assert_eq!(
            store.check("a.test", 22, &key()).unwrap(),
            HostKeyStatus::Changed
        );
    }

    #[test]
    fn port_is_part_of_the_identity() {
        let store = store("port");
        let key = key();
        store.trust("a.test", 22, &key).unwrap();
        assert_eq!(
            store.check("a.test", 2222, &key).unwrap(),
            HostKeyStatus::Unknown
        );
        store.trust("a.test", 2222, &key).unwrap();
        assert_eq!(
            store.check("a.test", 2222, &key).unwrap(),
            HostKeyStatus::Trusted
        );
        let text = fs::read_to_string(&store.path).unwrap();
        assert!(text.contains("[a.test]:2222 ssh-ed25519 "));
    }

    #[test]
    fn trusting_a_new_key_replaces_only_that_host() {
        let store = store("replace");
        let (old, new, other) = (key(), key(), key());
        store.trust("a.test", 22, &old).unwrap();
        store.trust("b.test", 22, &other).unwrap();
        store.trust("a.test", 22, &new).unwrap();
        assert_eq!(
            store.check("a.test", 22, &new).unwrap(),
            HostKeyStatus::Trusted
        );
        assert_eq!(
            store.check("a.test", 22, &old).unwrap(),
            HostKeyStatus::Changed
        );
        assert_eq!(
            store.check("b.test", 22, &other).unwrap(),
            HostKeyStatus::Trusted
        );
    }

    #[test]
    fn fingerprint_uses_sha256() {
        assert!(fingerprint(&key()).starts_with("SHA256:"));
    }
}
