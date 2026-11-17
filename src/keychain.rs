//! Keychain: generating, importing and storing SSH keys, and reading
//! OpenSSH user certificates.
//!
//! Private keys are written in OpenSSH format, one file per key, in the
//! `keys` folder next to the data file. They never go into `data.json`.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use russh::keys::{
    Algorithm, Certificate, EcdsaCurve, HashAlg, PrivateKey, PublicKey, decode_secret_key,
    ssh_key::{LineEnding, certificate::CertType},
};
use uuid::Uuid;

use crate::model::KeyEntry;

/// Folder holding one private key file per keychain entry.
#[derive(Debug, Clone)]
pub struct KeyStore {
    dir: PathBuf,
}

impl KeyStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the private key of entry `id` is kept.
    pub fn path(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}.key"))
    }

    /// Write a private key so that only the current user can read it.
    fn write(&self, id: Uuid, pem: &str) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        restrict_dir(&self.dir)?;
        let path = self.path(id);
        let tmp = path.with_extension("key.tmp");
        write_private(&tmp, pem.as_bytes())?;
        fs::rename(&tmp, &path)
    }

    /// Delete a private key file. A file that is already gone is not an error.
    pub fn remove(&self, id: Uuid) -> io::Result<()> {
        match fs::remove_file(self.path(id)) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }

    /// Copy a private key to `dest`, readable only by the current user.
    pub fn export(&self, id: Uuid, dest: &Path) -> io::Result<()> {
        let pem = fs::read(self.path(id))?;
        write_private(dest, &pem)
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    // Remove first so the mode below applies even if the file existed.
    let _ = fs::remove_file(path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// On Windows the file inherits the access list of the per-user profile
/// folder, which already limits it to the current user.
#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

#[cfg(unix)]
fn restrict_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// Key types that can be generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Ed25519,
    EcdsaP256,
    EcdsaP384,
    Rsa4096,
}

impl KeyKind {
    pub const ALL: [KeyKind; 4] = [
        Self::Ed25519,
        Self::EcdsaP256,
        Self::EcdsaP384,
        Self::Rsa4096,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Ed25519 => "ED25519 (recommended)",
            Self::EcdsaP256 => "ECDSA P-256",
            Self::EcdsaP384 => "ECDSA P-384",
            Self::Rsa4096 => "RSA 4096",
        }
    }

    /// One line on when to choose this type.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Ed25519 => "Small, fast and secure. Works with all current servers.",
            Self::EcdsaP256 | Self::EcdsaP384 => {
                "NIST curve. Choose it when a policy asks for ECDSA."
            }
            Self::Rsa4096 => "For old servers without ED25519. Takes a while to generate.",
        }
    }

    fn algorithm(self) -> Algorithm {
        match self {
            Self::Ed25519 => Algorithm::Ed25519,
            Self::EcdsaP256 => Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
            Self::EcdsaP384 => Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP384,
            },
            Self::Rsa4096 => Algorithm::Rsa { hash: None },
        }
    }
}

#[derive(Debug)]
pub enum KeyError {
    Io(io::Error),
    /// The text is not a private key in a format this app can read.
    Unreadable(String),
    /// The key file is encrypted and needs its passphrase to be converted.
    NeedsPassphrase,
    WrongPassphrase,
    Generate(String),
    /// The text is not an OpenSSH certificate.
    BadCertificate(String),
    /// The certificate is a host certificate, not a user certificate.
    HostCertificate,
    /// The certificate was issued for a different key.
    CertificateMismatch,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "Could not write the key: {err}"),
            Self::Unreadable(reason) => write!(f, "This is not a usable private key ({reason})."),
            Self::NeedsPassphrase => {
                f.write_str("This key file is encrypted. Enter its passphrase to import it.")
            }
            Self::WrongPassphrase => f.write_str(
                "Could not unlock this key file. Check the passphrase; \
                 the file may also use encryption this app cannot read.",
            ),
            Self::Generate(reason) => write!(f, "Could not generate the key: {reason}"),
            Self::BadCertificate(reason) => {
                write!(f, "This is not an OpenSSH certificate ({reason}).")
            }
            Self::HostCertificate => {
                f.write_str("This is a host certificate. A user certificate is needed here.")
            }
            Self::CertificateMismatch => {
                f.write_str("This certificate was issued for a different key.")
            }
        }
    }
}

impl From<io::Error> for KeyError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Generate a key pair, store the private key and return its entry.
/// An empty `passphrase` leaves the private key unencrypted.
pub fn generate(
    store: &KeyStore,
    kind: KeyKind,
    name: &str,
    passphrase: &str,
) -> Result<KeyEntry, KeyError> {
    let key = PrivateKey::random(&mut rand::rng(), kind.algorithm())
        .map_err(|err| KeyError::Generate(err.to_string()))?;
    save(store, key, name, passphrase)
}

/// Import a private key from the text of a key file.
///
/// OpenSSH keys are kept exactly as they are, encrypted or not. PuTTY and
/// PEM keys are converted to OpenSSH format, which needs `passphrase` when
/// the file is encrypted; the converted key is encrypted with the same one.
pub fn import(
    store: &KeyStore,
    text: &str,
    name: &str,
    passphrase: &str,
) -> Result<KeyEntry, KeyError> {
    if let Ok(key) = PrivateKey::from_openssh(text.trim()) {
        return save(store, key, name, "");
    }
    let encrypted = looks_encrypted(text);
    if encrypted && passphrase.is_empty() {
        return Err(KeyError::NeedsPassphrase);
    }
    let key = decode_secret_key(text, encrypted.then_some(passphrase)).map_err(|err| {
        if encrypted {
            KeyError::WrongPassphrase
        } else {
            KeyError::Unreadable(err.to_string())
        }
    })?;
    save(store, key, name, passphrase)
}

/// Whether a PuTTY or PEM key file says its private part is encrypted.
fn looks_encrypted(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line == "-----BEGIN ENCRYPTED PRIVATE KEY-----"
            || line.starts_with("Proc-Type:") && line.contains("ENCRYPTED")
            || line
                .strip_prefix("Encryption:")
                .is_some_and(|cipher| cipher.trim() != "none")
    })
}

fn save(
    store: &KeyStore,
    mut key: PrivateKey,
    name: &str,
    passphrase: &str,
) -> Result<KeyEntry, KeyError> {
    let name = name.trim();
    if !key.is_encrypted() {
        key.set_comment(name);
        if !passphrase.is_empty() {
            key = key
                .encrypt(&mut rand::rng(), passphrase)
                .map_err(|err| KeyError::Generate(err.to_string()))?;
        }
    }
    let public = key.public_key();
    let entry = KeyEntry {
        id: Uuid::new_v4(),
        name: name.to_string(),
        algorithm: algorithm_name(public),
        public_key: public
            .to_openssh()
            .map_err(|err| KeyError::Unreadable(err.to_string()))?,
        fingerprint: public.fingerprint(HashAlg::Sha256).to_string(),
        encrypted: key.is_encrypted(),
        certificate: None,
    };
    let pem = key
        .to_openssh(LineEnding::LF)
        .map_err(|err| KeyError::Unreadable(err.to_string()))?;
    store.write(entry.id, &pem)?;
    Ok(entry)
}

fn algorithm_name(key: &PublicKey) -> String {
    match key.algorithm() {
        Algorithm::Ed25519 => "ED25519".to_string(),
        Algorithm::Ecdsa { curve } => match curve {
            EcdsaCurve::NistP256 => "ECDSA P-256".to_string(),
            EcdsaCurve::NistP384 => "ECDSA P-384".to_string(),
            EcdsaCurve::NistP521 => "ECDSA P-521".to_string(),
        },
        Algorithm::Rsa { .. } => match key.key_data().rsa() {
            Some(rsa) => format!("RSA {}", rsa.key_size()),
            None => "RSA".to_string(),
        },
        other => other.as_str().to_string(),
    }
}

/// What a user certificate says, for display.
#[derive(Debug, Clone, PartialEq)]
pub struct CertificateInfo {
    pub key_id: String,
    pub serial: u64,
    /// Usernames the certificate may sign in as. Empty means any.
    pub principals: Vec<String>,
    pub valid_after: u64,
    pub valid_before: u64,
    /// Fingerprint of the certificate authority that signed it.
    pub ca_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateStatus {
    Valid,
    NotYetValid,
    Expired,
}

/// `valid_before` value OpenSSH uses for a certificate that never expires.
const FOREVER: u64 = u64::MAX;

impl CertificateInfo {
    /// Read the details of a stored certificate line.
    pub fn parse(certificate: &str) -> Option<Self> {
        let cert = Certificate::from_openssh(certificate.trim()).ok()?;
        Some(Self {
            key_id: cert.key_id().to_string(),
            serial: cert.serial(),
            principals: cert.valid_principals().to_vec(),
            valid_after: cert.valid_after(),
            valid_before: cert.valid_before(),
            ca_fingerprint: cert
                .signature_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
        })
    }

    pub fn status(&self, now: SystemTime) -> CertificateStatus {
        let now = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        if now < self.valid_after {
            CertificateStatus::NotYetValid
        } else if now >= self.valid_before {
            CertificateStatus::Expired
        } else {
            CertificateStatus::Valid
        }
    }

    /// When the certificate stops working, e.g. `2026-12-01 08:00 UTC`.
    pub fn expires(&self) -> String {
        if self.valid_before == FOREVER {
            return "never".to_string();
        }
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(self.valid_before);
        format!("{} UTC", crate::sftp::format_time(time))
    }

    pub fn principals_text(&self) -> String {
        if self.principals.is_empty() {
            "any user".to_string()
        } else {
            self.principals.join(", ")
        }
    }
}

/// Check that `text` is a user certificate issued for `key`, and return it
/// as a single clean line to store.
///
/// The CA signature is checked for integrity only. Whether the CA is trusted
/// is up to each server.
pub fn check_certificate(text: &str, key: &KeyEntry) -> Result<String, KeyError> {
    let cert = Certificate::from_openssh(text.trim())
        .map_err(|err| KeyError::BadCertificate(err.to_string()))?;
    if cert.cert_type() != CertType::User {
        return Err(KeyError::HostCertificate);
    }
    let public = PublicKey::from_openssh(&key.public_key)
        .map_err(|err| KeyError::Unreadable(err.to_string()))?;
    if cert.public_key() != public.key_data() {
        return Err(KeyError::CertificateMismatch);
    }
    cert.verify_signature()
        .map_err(|err| KeyError::BadCertificate(format!("bad signature: {err}")))?;
    cert.to_openssh()
        .map_err(|err| KeyError::BadCertificate(err.to_string()))
}

#[cfg(test)]
mod tests {
    use russh::keys::ssh_key::certificate::Builder;

    use super::*;

    fn store(name: &str) -> KeyStore {
        KeyStore::new(crate::ssh::tests::temp_dir(name))
    }

    fn read(store: &KeyStore, entry: &KeyEntry) -> PrivateKey {
        PrivateKey::read_openssh_file(store.path(entry.id)).unwrap()
    }

    #[test]
    fn generates_each_elliptic_kind() {
        let store = store("keychain-generate");
        for (kind, algorithm) in [
            (KeyKind::Ed25519, "ED25519"),
            (KeyKind::EcdsaP256, "ECDSA P-256"),
            (KeyKind::EcdsaP384, "ECDSA P-384"),
        ] {
            let entry = generate(&store, kind, " laptop ", "").unwrap();
            assert_eq!(entry.name, "laptop");
            assert_eq!(entry.algorithm, algorithm);
            assert!(!entry.encrypted);
            assert!(entry.fingerprint.starts_with("SHA256:"));
            assert!(entry.public_key.ends_with(" laptop"));
            let key = read(&store, &entry);
            assert_eq!(key.public_key().to_openssh().unwrap(), entry.public_key);
        }
    }

    #[test]
    #[ignore = "RSA generation takes a long time in debug builds"]
    fn generates_rsa_4096() {
        let store = store("keychain-rsa");
        let entry = generate(&store, KeyKind::Rsa4096, "legacy", "").unwrap();
        assert_eq!(entry.algorithm, "RSA 4096");
        assert!(entry.public_key.starts_with("ssh-rsa "));
    }

    #[test]
    fn passphrase_encrypts_the_private_key_file() {
        let store = store("keychain-encrypted");
        let entry = generate(&store, KeyKind::Ed25519, "locked", "s3cret").unwrap();
        assert!(entry.encrypted);
        let key = read(&store, &entry);
        assert!(key.is_encrypted());
        assert!(key.decrypt("wrong").is_err());
        let open = key.decrypt("s3cret").unwrap();
        assert_eq!(
            open.public_key().fingerprint(HashAlg::Sha256).to_string(),
            entry.fingerprint
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_key_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let store = store("keychain-mode");
        let entry = generate(&store, KeyKind::Ed25519, "k", "").unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&store.path(entry.id)), 0o600);
        assert_eq!(mode(store.dir()), 0o700);

        let exported = store.dir().join("exported");
        fs::write(&exported, "old").unwrap();
        store.export(entry.id, &exported).unwrap();
        assert_eq!(mode(&exported), 0o600);
        assert_eq!(
            fs::read(&exported).unwrap(),
            fs::read(store.path(entry.id)).unwrap()
        );
    }

    #[test]
    fn imports_openssh_keys_as_they_are() {
        let store = store("keychain-import");
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let plain = key.to_openssh(LineEnding::LF).unwrap();
        let entry = import(&store, &plain, "plain", "").unwrap();
        assert!(!entry.encrypted);
        assert_eq!(
            entry.fingerprint,
            key.public_key().fingerprint(HashAlg::Sha256).to_string()
        );

        let locked = key.encrypt(&mut rand::rng(), "pw").unwrap();
        let text = locked.to_openssh(LineEnding::CRLF).unwrap();
        let entry = import(&store, &text, "locked", "").unwrap();
        assert!(entry.encrypted);
        assert_eq!(read(&store, &entry).decrypt("pw").unwrap(), key);
    }

    /// Throwaway PKCS#8 test key made with `openssl genpkey -algorithm ed25519`.
    const PEM_KEY: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJ9993/B/m3bt3qeg9iIbULQNwy9rMhB1JT/b/wM0DFt
-----END PRIVATE KEY-----
";
    /// The same key encrypted with the passphrase `pw`.
    const PEM_KEY_ENCRYPTED: &str = "-----BEGIN ENCRYPTED PRIVATE KEY-----
MIGbMFcGCSqGSIb3DQEFDTBKMCkGCSqGSIb3DQEFDDAcBAhjOx8Q1a9BGQICCAAw
DAYIKoZIhvcNAgkFADAdBglghkgBZQMEASoEEAZZMoqOrISvKhMtR+g2Bs8EQNK4
ggDAzmZTpgoe5ffBoeYf9YjVKY4Ok8T0xpLXEiy/OyBXHPorIwqHrkkEcsXe7xpv
/uyXzXTU2NLIuuq+Trc=
-----END ENCRYPTED PRIVATE KEY-----
";

    #[test]
    fn pem_keys_are_converted_to_openssh_format() {
        let store = store("keychain-pem");
        let plain = import(&store, PEM_KEY, "pem", "").unwrap();
        assert_eq!(plain.algorithm, "ED25519");
        assert!(!plain.encrypted);
        assert!(!read(&store, &plain).is_encrypted());

        assert!(matches!(
            import(&store, PEM_KEY_ENCRYPTED, "pem", ""),
            Err(KeyError::NeedsPassphrase)
        ));
        assert!(matches!(
            import(&store, PEM_KEY_ENCRYPTED, "pem", "nope"),
            Err(KeyError::WrongPassphrase)
        ));
        // The converted key stays protected by the same passphrase.
        let locked = import(&store, PEM_KEY_ENCRYPTED, "pem", "pw").unwrap();
        assert!(locked.encrypted);
        assert_eq!(locked.fingerprint, plain.fingerprint);
        assert!(read(&store, &locked).decrypt("pw").is_ok());
    }

    #[test]
    fn rejects_text_that_is_not_a_key() {
        let store = store("keychain-bad");
        assert!(matches!(
            import(&store, "hello", "x", ""),
            Err(KeyError::Unreadable(_))
        ));
        assert_eq!(fs::read_dir(store.dir()).unwrap().count(), 0);
    }

    #[test]
    fn remove_deletes_the_file_and_tolerates_a_missing_one() {
        let store = store("keychain-remove");
        let entry = generate(&store, KeyKind::Ed25519, "k", "").unwrap();
        store.remove(entry.id).unwrap();
        assert!(!store.path(entry.id).exists());
        store.remove(entry.id).unwrap();
    }

    fn certificate(
        ca: &PrivateKey,
        subject: &PublicKey,
        kind: CertType,
        valid: (u64, u64),
    ) -> String {
        let mut builder =
            Builder::new_with_random_nonce(&mut rand::rng(), subject, valid.0, valid.1).unwrap();
        builder.cert_type(kind).unwrap();
        builder.key_id("alice@laptop").unwrap();
        builder.serial(7).unwrap();
        builder.valid_principal("deploy").unwrap();
        builder.sign(ca).unwrap().to_openssh().unwrap()
    }

    #[test]
    fn certificate_must_be_a_user_certificate_for_this_key() {
        let store = store("keychain-cert");
        let entry = generate(&store, KeyKind::Ed25519, "k", "").unwrap();
        let other = generate(&store, KeyKind::Ed25519, "other", "").unwrap();
        let subject = PublicKey::from_openssh(&entry.public_key).unwrap();
        let ca = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();

        let user = certificate(&ca, &subject, CertType::User, (1_000, 2_000));
        let stored = check_certificate(&format!("  {user}\n"), &entry).unwrap();
        assert!(matches!(
            check_certificate(&user, &other),
            Err(KeyError::CertificateMismatch)
        ));
        let host = certificate(&ca, &subject, CertType::Host, (1_000, 2_000));
        assert!(matches!(
            check_certificate(&host, &entry),
            Err(KeyError::HostCertificate)
        ));
        assert!(matches!(
            check_certificate(&entry.public_key, &entry),
            Err(KeyError::BadCertificate(_))
        ));

        let info = CertificateInfo::parse(&stored).unwrap();
        assert_eq!(info.key_id, "alice@laptop");
        assert_eq!(info.serial, 7);
        assert_eq!(info.principals_text(), "deploy");
        assert_eq!(
            info.ca_fingerprint,
            ca.public_key().fingerprint(HashAlg::Sha256).to_string()
        );
        assert_eq!(info.expires(), "1970-01-01 00:33 UTC");
        let at = |secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(info.status(at(999)), CertificateStatus::NotYetValid);
        assert_eq!(info.status(at(1_500)), CertificateStatus::Valid);
        assert_eq!(info.status(at(2_000)), CertificateStatus::Expired);
    }

    #[test]
    fn certificate_without_expiry_or_principals() {
        let info = CertificateInfo {
            key_id: String::new(),
            serial: 0,
            principals: Vec::new(),
            valid_after: 0,
            valid_before: FOREVER,
            ca_fingerprint: String::new(),
        };
        assert_eq!(info.expires(), "never");
        assert_eq!(info.principals_text(), "any user");
        assert_eq!(info.status(SystemTime::now()), CertificateStatus::Valid);
    }
}
