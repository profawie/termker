//! Security keys over USB HID, speaking CTAP2 directly through the
//! `ctap-hid-fido2` crate. Used on macOS and Linux.

use ctap_hid_fido2::{
    Cfg, FidoKeyHid, FidoKeyHidFactory,
    fidokey::{CredentialSupportedKeyType, GetAssertionArgsBuilder, MakeCredentialArgsBuilder},
    public_key::PublicKeyType,
    public_key_credential_user_entity::PublicKeyCredentialUserEntity,
};

use super::{Assertion, Authenticator, FidoError, NewCredential, SkKind};

pub struct HidAuthenticator;

fn open() -> Result<FidoKeyHid, FidoError> {
    if ctap_hid_fido2::get_fidokey_devices().is_empty() {
        return Err(FidoError::NoDevice);
    }
    let mut cfg = Cfg::init();
    // The library would otherwise print a "touch the sensor" line to stdout.
    cfg.enable_keep_alive_msg = false;
    FidoKeyHidFactory::create(&cfg).map_err(|err| {
        let text = err.to_string();
        if text.contains("Multiple") {
            FidoError::Unsupported(
                "more than one security key is plugged in; leave only one connected".to_string(),
            )
        } else {
            FidoError::Other(text)
        }
    })
}

/// Turn the library's error text into an error the app can act on. The
/// library reports CTAP status codes only as text such as
/// `0x36 CTAP2_ERR_PIN_REQUIRED`.
pub(super) fn classify(text: &str) -> FidoError {
    const PIN_REQUIRED: [&str; 2] = ["CTAP2_ERR_PIN_REQUIRED", "CTAP2_ERR_PUAT_REQUIRED"];
    const PIN_INVALID: [&str; 2] = ["CTAP2_ERR_PIN_INVALID", "CTAP2_ERR_PIN_AUTH_INVALID"];
    const NOT_CONFIRMED: [&str; 4] = [
        "CTAP2_ERR_ACTION_TIMEOUT",
        "CTAP2_ERR_USER_ACTION_TIMEOUT",
        "CTAP2_ERR_OPERATION_DENIED",
        "CTAP2_ERR_KEEPALIVE_CANCEL",
    ];
    let has = |codes: &[&str]| codes.iter().any(|code| text.contains(code));
    if has(&PIN_REQUIRED) {
        FidoError::PinRequired
    } else if has(&PIN_INVALID) {
        FidoError::PinInvalid
    } else if text.contains("CTAP2_ERR_PIN_BLOCKED") || text.contains("CTAP2_ERR_PIN_AUTH_BLOCKED")
    {
        FidoError::Other(
            "the PIN is blocked after too many wrong attempts; unplug the key and plug it \
             back in, or reset its PIN"
                .to_string(),
        )
    } else if text.contains("CTAP2_ERR_PIN_NOT_SET") {
        FidoError::Unsupported("this security key has no PIN set".to_string())
    } else if has(&NOT_CONFIRMED) {
        FidoError::NotConfirmed
    } else if text.contains("CTAP2_ERR_UNSUPPORTED_ALGORITHM") {
        FidoError::Unsupported("this security key cannot create that key type".to_string())
    } else if text.contains("CTAP2_ERR_NO_CREDENTIALS") {
        FidoError::Other("this is not the security key the SSH key was created on".to_string())
    } else {
        FidoError::Other(text.to_string())
    }
}

impl Authenticator for HidAuthenticator {
    fn make_credential(
        &self,
        kind: SkKind,
        application: &str,
        user: &str,
        pin: Option<&str>,
    ) -> Result<NewCredential, FidoError> {
        let device = open()?;
        // The challenge only matters for attestation, which SSH does not use.
        let challenge: [u8; 32] = rand::random();
        let key_type = match kind {
            SkKind::Ed25519 => CredentialSupportedKeyType::Ed25519,
            SkKind::EcdsaP256 => CredentialSupportedKeyType::Ecdsa256,
        };
        // OpenSSH uses an all-zero user id for keys that are not resident.
        let user = PublicKeyCredentialUserEntity::new(Some(&[0u8; 32]), Some(user), Some(user));
        let builder = MakeCredentialArgsBuilder::new(application, &challenge)
            .key_type(key_type)
            .user_entity(&user);
        let args = match pin {
            Some(pin) => builder.pin(pin).build(),
            None => builder.without_pin_and_uv().build(),
        };
        let attestation = device
            .make_credential_with_args(&args)
            .map_err(|err| classify(&err.to_string()))?;

        let matches_kind = matches!(
            (kind, &attestation.credential_publickey.key_type),
            (SkKind::Ed25519, PublicKeyType::Ed25519)
                | (SkKind::EcdsaP256, PublicKeyType::Ecdsa256)
        );
        if !matches_kind {
            return Err(FidoError::Unsupported(
                "the security key created a different key type than was asked for".to_string(),
            ));
        }
        Ok(NewCredential {
            kind,
            public_key: attestation.credential_publickey.der,
            key_handle: attestation.credential_descriptor.id,
        })
    }

    fn get_assertion(
        &self,
        _kind: SkKind,
        application: &str,
        key_handle: &[u8],
        message: &[u8],
        pin: Option<&str>,
    ) -> Result<Assertion, FidoError> {
        let device = open()?;
        // The library hashes the "challenge" with SHA-256 to make the client
        // data hash, so it is given the whole message.
        let builder = GetAssertionArgsBuilder::new(application, message).credential_id(key_handle);
        let args = match pin {
            Some(pin) => builder.pin(pin).build(),
            None => builder.without_pin_and_uv().build(),
        };
        let assertion = device
            .get_assertion_with_args(&args)
            .map_err(|err| classify(&err.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                FidoError::Other("the security key returned no signature".to_string())
            })?;
        let (flags, counter) = super::parse_authenticator_data(&assertion.auth_data)
            .ok_or_else(|| FidoError::Other("the security key returned bad data".to_string()))?;
        Ok(Assertion {
            signature: assertion.signature,
            flags,
            counter,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctap_status_text_is_classified() {
        let pin =
            classify("0x36 CTAP2_ERR_PIN_REQUIRED  PIN is required for the selected operation.");
        assert_eq!(pin, FidoError::PinRequired);
        assert_eq!(
            classify("0x31 CTAP2_ERR_PIN_INVALID   PIN Invalid."),
            FidoError::PinInvalid
        );
        assert_eq!(
            classify("0x2F CTAP2_ERR_USER_ACTION_TIMEOUT"),
            FidoError::NotConfirmed
        );
        assert!(matches!(
            classify("0x26 CTAP2_ERR_UNSUPPORTED_ALGORITHM"),
            FidoError::Unsupported(_)
        ));
        assert!(matches!(
            classify("0x32 CTAP2_ERR_PIN_BLOCKED PIN Blocked."),
            FidoError::Other(_)
        ));
        assert_eq!(
            classify("something else"),
            FidoError::Other("something else".to_string())
        );
    }

    #[test]
    fn no_security_key_plugged_in_is_reported_as_such() {
        // CI machines and this test environment have no security key.
        if ctap_hid_fido2::get_fidokey_devices().is_empty() {
            let result = HidAuthenticator.get_assertion(SkKind::Ed25519, "ssh:", b"h", b"m", None);
            assert_eq!(result, Err(FidoError::NoDevice));
        }
    }
}
