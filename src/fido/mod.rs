//! FIDO2 security keys as SSH keys (`sk-ssh-ed25519@openssh.com` and
//! `sk-ecdsa-sha2-nistp256@openssh.com`), following OpenSSH's PROTOCOL.u2f.
//!
//! The private key never leaves the security key. What is stored on disk is
//! a key handle that only that security key can use, in the same file format
//! `ssh-keygen -t ed25519-sk` writes.

#[cfg(not(windows))]
mod hid;

use std::{fmt, sync::Arc};

use russh::keys::{
    PrivateKey,
    ssh_key::{
        private::{KeypairData, SkEcdsaSha2NistP256, SkEd25519},
        public::{self, EcdsaPublicKey, Ed25519PublicKey},
    },
};

/// Application (relying party) OpenSSH uses for SSH keys.
pub const APPLICATION: &str = "ssh:";

/// Key file flag: a touch is required for every signature.
const SK_USER_PRESENCE_REQUIRED: u8 = 0x01;

/// Signature algorithms a security key can hold an SSH key for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkKind {
    Ed25519,
    EcdsaP256,
}

impl SkKind {
    pub const ALL: [SkKind; 2] = [Self::Ed25519, Self::EcdsaP256];

    pub fn title(self) -> &'static str {
        match self {
            Self::Ed25519 => "ED25519-SK (recommended)",
            Self::EcdsaP256 => "ECDSA-SK",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Ed25519 => {
                "Needs a FIDO2 key with ED25519 support, such as YubiKey 5.2.3 or newer."
            }
            Self::EcdsaP256 => "Works with every FIDO2 and U2F security key.",
        }
    }

    fn signature_name(self) -> &'static str {
        match self {
            Self::Ed25519 => "sk-ssh-ed25519@openssh.com",
            Self::EcdsaP256 => "sk-ecdsa-sha2-nistp256@openssh.com",
        }
    }
}

/// A credential a security key just created.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCredential {
    pub kind: SkKind,
    /// ED25519: the 32-byte public key. ECDSA: the SEC1 uncompressed point.
    pub public_key: Vec<u8>,
    /// Opaque handle the security key needs to sign with this credential.
    pub key_handle: Vec<u8>,
}

/// What a security key returns when it signs.
#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    /// ED25519: the raw 64-byte signature. ECDSA: ASN.1 DER `(r, s)`.
    pub signature: Vec<u8>,
    /// Authenticator data flags (bit 0: user present, bit 2: user verified).
    pub flags: u8,
    /// Signature counter.
    pub counter: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FidoError {
    /// No security key is plugged in.
    NoDevice,
    /// The security key wants its PIN for this operation.
    PinRequired,
    PinInvalid,
    /// The user did not touch the key in time, or cancelled.
    NotConfirmed,
    /// The key or this system cannot do what was asked.
    Unsupported(String),
    Other(String),
}

impl fmt::Display for FidoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => f.write_str("No security key was found. Plug it in and try again."),
            Self::PinRequired => f.write_str("The security key needs its PIN."),
            Self::PinInvalid => f.write_str("The security key rejected the PIN."),
            Self::NotConfirmed => f.write_str(
                "The security key was not touched in time, or the request was cancelled.",
            ),
            Self::Unsupported(reason) => write!(f, "Not supported: {reason}"),
            Self::Other(reason) => write!(f, "Security key error: {reason}"),
        }
    }
}

/// A way to talk to the user's security key. Calls block until the user
/// touches the key, so they must run off the UI and async threads.
pub trait Authenticator: Send + Sync {
    /// Create a non-resident credential for `application`.
    fn make_credential(
        &self,
        kind: SkKind,
        application: &str,
        user: &str,
        pin: Option<&str>,
    ) -> Result<NewCredential, FidoError>;

    /// Sign `message` with the credential behind `key_handle`. The security
    /// key signs the SHA-256 of `message` as its client data hash, which is
    /// what an SSH server checks.
    fn get_assertion(
        &self,
        kind: SkKind,
        application: &str,
        key_handle: &[u8],
        message: &[u8],
        pin: Option<&str>,
    ) -> Result<Assertion, FidoError>;

    /// Whether the system shows its own PIN and touch dialogs, so the app
    /// must not ask for a PIN itself.
    fn has_own_dialogs(&self) -> bool {
        false
    }
}

/// Used where no security key backend exists.
#[cfg(any(windows, test))]
pub struct Unavailable;

#[cfg(any(windows, test))]
impl Authenticator for Unavailable {
    fn make_credential(
        &self,
        _: SkKind,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<NewCredential, FidoError> {
        Err(unavailable())
    }

    fn get_assertion(
        &self,
        _: SkKind,
        _: &str,
        _: &[u8],
        _: &[u8],
        _: Option<&str>,
    ) -> Result<Assertion, FidoError> {
        Err(unavailable())
    }
}

#[cfg(any(windows, test))]
fn unavailable() -> FidoError {
    FidoError::Unsupported("security keys are not available in this build".to_string())
}

/// The security key backend for this platform.
pub fn platform() -> Arc<dyn Authenticator> {
    #[cfg(not(windows))]
    {
        Arc::new(hid::HidAuthenticator)
    }
    #[cfg(windows)]
    {
        Arc::new(Unavailable)
    }
}

/// The OpenSSH private key (really: public key plus key handle) for a new
/// credential.
pub fn private_key(
    credential: &NewCredential,
    application: &str,
    comment: &str,
) -> Result<PrivateKey, FidoError> {
    let bad = |what: &str| FidoError::Other(format!("the security key returned {what}"));
    let data = match credential.kind {
        SkKind::Ed25519 => {
            let bytes: [u8; 32] = credential
                .public_key
                .as_slice()
                .try_into()
                .map_err(|_| bad("an ED25519 public key of the wrong size"))?;
            let public = public::SkEd25519::new(Ed25519PublicKey(bytes), application);
            KeypairData::SkEd25519(
                SkEd25519::new(
                    public,
                    SK_USER_PRESENCE_REQUIRED,
                    credential.key_handle.clone(),
                )
                .map_err(|_| bad("a key handle that is too long"))?,
            )
        }
        SkKind::EcdsaP256 => {
            let point = match EcdsaPublicKey::from_sec1_bytes(&credential.public_key) {
                Ok(EcdsaPublicKey::NistP256(point)) => point,
                _ => return Err(bad("an ECDSA public key that is not a P-256 point")),
            };
            let public = public::SkEcdsaSha2NistP256::new(point, application);
            KeypairData::SkEcdsaSha2NistP256(
                SkEcdsaSha2NistP256::new(
                    public,
                    SK_USER_PRESENCE_REQUIRED,
                    credential.key_handle.clone(),
                )
                .map_err(|_| bad("a key handle that is too long"))?,
            )
        }
    };
    PrivateKey::new(data, comment).map_err(|err| FidoError::Other(err.to_string()))
}

/// What is needed to sign with a security key, read from its key file.
#[derive(Debug, Clone, PartialEq)]
pub struct SkKey {
    pub kind: SkKind,
    pub application: String,
    pub key_handle: Vec<u8>,
}

impl SkKey {
    /// `None` if `key` is not a security key.
    pub fn from_private(key: &PrivateKey) -> Option<Self> {
        match key.key_data() {
            KeypairData::SkEd25519(sk) => Some(Self {
                kind: SkKind::Ed25519,
                application: sk.public().application().to_string(),
                key_handle: sk.key_handle().to_vec(),
            }),
            KeypairData::SkEcdsaSha2NistP256(sk) => Some(Self {
                kind: SkKind::EcdsaP256,
                application: sk.public().application().to_string(),
                key_handle: sk.key_handle().to_vec(),
            }),
            _ => None,
        }
    }

    /// Have the security key sign `message` and return the SSH signature,
    /// in the form `string signature` of a userauth request.
    pub fn sign(
        &self,
        authenticator: &dyn Authenticator,
        message: &[u8],
        pin: Option<&str>,
    ) -> Result<Vec<u8>, FidoError> {
        let assertion = authenticator.get_assertion(
            self.kind,
            &self.application,
            &self.key_handle,
            message,
            pin,
        )?;
        encode_signature(self.kind, &assertion)
    }
}

/// SSH wire form of a security key signature (PROTOCOL.u2f):
/// `string name, string signature, byte flags, uint32 counter`, all wrapped
/// in one outer string.
pub fn encode_signature(kind: SkKind, assertion: &Assertion) -> Result<Vec<u8>, FidoError> {
    let signature = match kind {
        SkKind::Ed25519 => {
            if assertion.signature.len() != 64 {
                return Err(FidoError::Other(
                    "the security key returned an ED25519 signature of the wrong size".to_string(),
                ));
            }
            assertion.signature.clone()
        }
        SkKind::EcdsaP256 => {
            let (r, s) = parse_der_signature(&assertion.signature).ok_or_else(|| {
                FidoError::Other(
                    "the security key returned an ECDSA signature that cannot be read".to_string(),
                )
            })?;
            let mut inner = Vec::new();
            put_string(&mut inner, r);
            put_string(&mut inner, s);
            inner
        }
    };
    let mut body = Vec::new();
    put_string(&mut body, kind.signature_name().as_bytes());
    put_string(&mut body, &signature);
    body.push(assertion.flags);
    body.extend_from_slice(&assertion.counter.to_be_bytes());

    let mut out = Vec::new();
    put_string(&mut out, &body);
    Ok(out)
}

fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Split an ASN.1 DER `SEQUENCE { INTEGER r, INTEGER s }` into the two
/// integers. A DER integer is already in SSH `mpint` form: big-endian,
/// minimal, with a leading zero when the top bit is set.
fn parse_der_signature(der: &[u8]) -> Option<(&[u8], &[u8])> {
    let (body, rest) = der_element(der, 0x30)?;
    if !rest.is_empty() {
        return None;
    }
    let (r, body) = der_element(body, 0x02)?;
    let (s, body) = der_element(body, 0x02)?;
    (body.is_empty() && !r.is_empty() && !s.is_empty()).then_some((r, s))
}

/// Read one DER element with the given tag; returns its contents and what
/// follows it. Only the short and one-byte long length forms can occur in a
/// P-256 signature.
fn der_element(input: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    let (&found, rest) = input.split_first()?;
    if found != tag {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = match first {
        0..=0x7f => (first as usize, rest),
        0x81 => {
            let (&len, rest) = rest.split_first()?;
            (len as usize, rest)
        }
        _ => return None,
    };
    (rest.len() >= len).then(|| rest.split_at(len))
}

/// Flags and counter from WebAuthn/CTAP authenticator data
/// (`rpIdHash[32] flags[1] counter[4] ...`).
pub fn parse_authenticator_data(data: &[u8]) -> Option<(u8, u32)> {
    let flags = *data.get(32)?;
    let counter = u32::from_be_bytes(data.get(33..37)?.try_into().ok()?);
    Some((flags, counter))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use russh::keys::{
        Algorithm, HashAlg,
        signature::{Signer, Verifier},
        ssh_key::{
            Signature,
            private::{EcdsaKeypair, Ed25519Keypair},
            sha2::{Digest, Sha256},
        },
    };

    use super::*;

    /// The kind of an SSH security key algorithm, if it is one.
    fn kind_of(algorithm: &Algorithm) -> Option<SkKind> {
        match algorithm {
            Algorithm::SkEd25519 => Some(SkKind::Ed25519),
            Algorithm::SkEcdsaSha2NistP256 => Some(SkKind::EcdsaP256),
            _ => None,
        }
    }

    /// A security key made of software: it keeps real private keys in
    /// memory and signs exactly what a FIDO2 authenticator signs.
    #[derive(Default)]
    pub(crate) struct SoftwareKey {
        pub(crate) credentials: Mutex<Vec<(Vec<u8>, KeypairData)>>,
        pub(crate) counter: Mutex<u32>,
        /// When set, every request needs this PIN.
        pub(crate) pin: Option<&'static str>,
        /// Simulate the user not touching the key.
        pub(crate) refuse_touch: bool,
    }

    impl SoftwareKey {
        fn check_pin(&self, pin: Option<&str>) -> Result<(), FidoError> {
            match (self.pin, pin) {
                (None, _) => Ok(()),
                (Some(_), None) => Err(FidoError::PinRequired),
                (Some(expected), Some(given)) if expected == given => Ok(()),
                _ => Err(FidoError::PinInvalid),
            }
        }
    }

    /// DER `SEQUENCE { INTEGER r, INTEGER s }` from two SSH mpints.
    fn der_signature(r: &[u8], s: &[u8]) -> Vec<u8> {
        let mut body = vec![0x02, r.len() as u8];
        body.extend_from_slice(r);
        body.extend_from_slice(&[0x02, s.len() as u8]);
        body.extend_from_slice(s);
        let mut out = vec![0x30, body.len() as u8];
        out.extend(body);
        out
    }

    fn take_string(input: &[u8]) -> (&[u8], &[u8]) {
        let len = u32::from_be_bytes(input[..4].try_into().unwrap()) as usize;
        input[4..].split_at(len)
    }

    impl Authenticator for SoftwareKey {
        fn make_credential(
            &self,
            kind: SkKind,
            _application: &str,
            _user: &str,
            pin: Option<&str>,
        ) -> Result<NewCredential, FidoError> {
            self.check_pin(pin)?;
            if self.refuse_touch {
                return Err(FidoError::NotConfirmed);
            }
            let mut rng = rand::rng();
            let (public_key, keypair) = match kind {
                SkKind::Ed25519 => {
                    let keypair = Ed25519Keypair::random(&mut rng);
                    (keypair.public.0.to_vec(), KeypairData::from(keypair))
                }
                SkKind::EcdsaP256 => {
                    let keypair =
                        EcdsaKeypair::random(&mut rng, russh::keys::EcdsaCurve::NistP256).unwrap();
                    let point = match &keypair {
                        EcdsaKeypair::NistP256 { public, .. } => public.as_bytes().to_vec(),
                        _ => unreachable!(),
                    };
                    (point, KeypairData::from(keypair))
                }
            };
            let mut credentials = self.credentials.lock().unwrap();
            let key_handle = format!("handle-{}", credentials.len()).into_bytes();
            credentials.push((key_handle.clone(), keypair));
            Ok(NewCredential {
                kind,
                public_key,
                key_handle,
            })
        }

        fn get_assertion(
            &self,
            kind: SkKind,
            application: &str,
            key_handle: &[u8],
            message: &[u8],
            pin: Option<&str>,
        ) -> Result<Assertion, FidoError> {
            self.check_pin(pin)?;
            if self.refuse_touch {
                return Err(FidoError::NotConfirmed);
            }
            let credentials = self.credentials.lock().unwrap();
            let keypair = credentials
                .iter()
                .find(|(handle, _)| handle == key_handle)
                .map(|(_, keypair)| keypair)
                .ok_or(FidoError::Other("unknown key handle".to_string()))?;
            let mut counter = self.counter.lock().unwrap();
            *counter += 1;
            let flags = 0x01;

            // authenticatorData || clientDataHash
            let mut signed = Sha256::digest(application.as_bytes()).to_vec();
            signed.push(flags);
            signed.extend_from_slice(&counter.to_be_bytes());
            signed.extend_from_slice(&Sha256::digest(message));
            let signature: Signature = keypair.try_sign(&signed).unwrap();
            let signature = match kind {
                SkKind::Ed25519 => signature.as_bytes().to_vec(),
                SkKind::EcdsaP256 => {
                    let (r, rest) = take_string(signature.as_bytes());
                    let (s, _) = take_string(rest);
                    der_signature(r, s)
                }
            };
            Ok(Assertion {
                signature,
                flags,
                counter: *counter,
            })
        }
    }

    #[test]
    fn new_credential_becomes_an_openssh_security_key() {
        let device = SoftwareKey::default();
        for (kind, prefix) in [
            (SkKind::Ed25519, "sk-ssh-ed25519@openssh.com "),
            (SkKind::EcdsaP256, "sk-ecdsa-sha2-nistp256@openssh.com "),
        ] {
            let credential = device
                .make_credential(kind, APPLICATION, "me", None)
                .unwrap();
            let key = private_key(&credential, APPLICATION, "yubikey").unwrap();
            assert!(key.public_key().to_openssh().unwrap().starts_with(prefix));

            // The file round-trips and still holds the key handle.
            let pem = key.to_openssh(Default::default()).unwrap();
            let read = PrivateKey::from_openssh(pem.as_bytes()).unwrap();
            let sk = SkKey::from_private(&read).unwrap();
            assert_eq!(sk.kind, kind);
            assert_eq!(sk.application, "ssh:");
            assert_eq!(sk.key_handle, credential.key_handle);
            assert_eq!(kind_of(&read.algorithm()), Some(kind));
        }
    }

    #[test]
    fn ordinary_keys_are_not_security_keys() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        assert_eq!(SkKey::from_private(&key), None);
        assert_eq!(kind_of(&key.algorithm()), None);
    }

    /// The signature a security key produces must verify the way an SSH
    /// server verifies it.
    #[test]
    fn signatures_verify_as_ssh_security_key_signatures() {
        let device = SoftwareKey::default();
        for kind in SkKind::ALL {
            let credential = device
                .make_credential(kind, APPLICATION, "me", None)
                .unwrap();
            let key = private_key(&credential, APPLICATION, "").unwrap();
            let sk = SkKey::from_private(&key).unwrap();
            let message = b"session id and userauth request";

            let wire = sk.sign(&device, message, None).unwrap();
            let (body, rest) = take_string(&wire);
            assert!(rest.is_empty());
            let signature = Signature::try_from(body).unwrap();
            assert_eq!(kind_of(&signature.algorithm()), Some(kind));
            key.public_key()
                .key_data()
                .verify(message, &signature)
                .unwrap();
            assert!(
                key.public_key()
                    .key_data()
                    .verify(b"another message", &signature)
                    .is_err()
            );
            let _ = key.public_key().fingerprint(HashAlg::Sha256);
        }
    }

    #[test]
    fn pin_and_touch_errors_are_passed_on() {
        let mut device = SoftwareKey {
            pin: Some("1234"),
            ..Default::default()
        };
        let make = |device: &SoftwareKey, pin| {
            device.make_credential(SkKind::Ed25519, APPLICATION, "me", pin)
        };
        assert_eq!(make(&device, None), Err(FidoError::PinRequired));
        assert_eq!(make(&device, Some("0000")), Err(FidoError::PinInvalid));
        let credential = make(&device, Some("1234")).unwrap();
        let key = private_key(&credential, APPLICATION, "").unwrap();
        let sk = SkKey::from_private(&key).unwrap();
        assert_eq!(sk.sign(&device, b"m", None), Err(FidoError::PinRequired));
        assert!(sk.sign(&device, b"m", Some("1234")).is_ok());
        device.refuse_touch = true;
        assert_eq!(
            sk.sign(&device, b"m", Some("1234")),
            Err(FidoError::NotConfirmed)
        );
    }

    #[test]
    fn der_signatures_are_parsed_strictly() {
        let der = der_signature(&[0x00, 0x80, 0x01], &[0x7f]);
        assert_eq!(
            parse_der_signature(&der),
            Some((&[0x00, 0x80, 0x01][..], &[0x7f][..]))
        );
        // Long-form length, as used when the body is 128 bytes or more.
        let mut long = vec![0x30, 0x81, der.len() as u8 - 2];
        long.extend_from_slice(&der[2..]);
        assert!(parse_der_signature(&long).is_some());

        let mut trailing = der.clone();
        trailing.push(0);
        assert_eq!(parse_der_signature(&trailing), None);
        assert_eq!(parse_der_signature(&der[..der.len() - 1]), None);
        assert_eq!(parse_der_signature(&[0x31, 0x00]), None);
        assert_eq!(parse_der_signature(&[]), None);
    }

    #[test]
    fn bad_signature_sizes_are_rejected() {
        let short = Assertion {
            signature: vec![0; 63],
            flags: 1,
            counter: 1,
        };
        assert!(encode_signature(SkKind::Ed25519, &short).is_err());
        assert!(encode_signature(SkKind::EcdsaP256, &short).is_err());
    }

    #[test]
    fn authenticator_data_flags_and_counter() {
        let mut data = vec![0xaa; 32];
        data.push(0x05);
        data.extend_from_slice(&[0, 0, 1, 2]);
        assert_eq!(parse_authenticator_data(&data), Some((0x05, 258)));
        assert_eq!(parse_authenticator_data(&data[..36]), None);
    }

    #[test]
    fn missing_backend_reports_unsupported() {
        let result = Unavailable.make_credential(SkKind::Ed25519, APPLICATION, "me", None);
        assert!(matches!(result, Err(FidoError::Unsupported(_))));
    }
}
