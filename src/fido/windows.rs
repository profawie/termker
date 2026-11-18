//! Security keys through the Windows WebAuthn API (`webauthn.dll`).
//!
//! Windows does not let ordinary programs talk to a security key over USB
//! directly; that needs administrator rights. Programs go through Windows
//! instead, which shows its own "Windows Security" dialogs for the PIN and
//! the touch. This is the same route OpenSSH for Windows takes.

use std::slice;

use windows::{
    Win32::{
        Foundation::{
            NTE_DEVICE_NOT_FOUND, NTE_INVALID_PARAMETER, NTE_NOT_FOUND, NTE_NOT_SUPPORTED,
            NTE_USER_CANCELLED,
        },
        Networking::WindowsWebServices::{
            WEBAUTHN_ASSERTION, WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
            WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
            WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS,
            WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_1,
            WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS,
            WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_1, WEBAUTHN_CLIENT_DATA,
            WEBAUTHN_CLIENT_DATA_CURRENT_VERSION, WEBAUTHN_COSE_ALGORITHM_ECDSA_P256_WITH_SHA256,
            WEBAUTHN_COSE_CREDENTIAL_PARAMETER, WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
            WEBAUTHN_COSE_CREDENTIAL_PARAMETERS, WEBAUTHN_CREDENTIAL,
            WEBAUTHN_CREDENTIAL_ATTESTATION, WEBAUTHN_CREDENTIAL_CURRENT_VERSION,
            WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY, WEBAUTHN_CREDENTIALS,
            WEBAUTHN_HASH_ALGORITHM_SHA_256, WEBAUTHN_RP_ENTITY_INFORMATION,
            WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION, WEBAUTHN_USER_ENTITY_INFORMATION,
            WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
            WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED, WebAuthNAuthenticatorGetAssertion,
            WebAuthNAuthenticatorMakeCredential, WebAuthNFreeAssertion,
            WebAuthNFreeCredentialAttestation,
        },
        UI::WindowsAndMessaging::GetForegroundWindow,
    },
    core::{HRESULT, HSTRING, PCWSTR},
};

use super::{Assertion, Authenticator, FidoError, NewCredential, SkKind, cose};

/// COSE algorithm number for EdDSA; the `windows` crate has no constant for it.
const COSE_ALGORITHM_EDDSA: i32 = -8;

/// How long Windows waits for the user before giving up.
const TIMEOUT_MS: u32 = 60_000;

pub struct WebAuthnAuthenticator;

fn classify(error: &windows::core::Error) -> FidoError {
    /// `HRESULT_FROM_WIN32(ERROR_CANCELLED)` and `(ERROR_TIMEOUT)`.
    const CANCELLED: HRESULT = HRESULT(0x800704C7_u32 as i32);
    const TIMED_OUT: HRESULT = HRESULT(0x800705B4_u32 as i32);
    let code = error.code();
    if code == NTE_USER_CANCELLED || code == CANCELLED || code == TIMED_OUT {
        FidoError::NotConfirmed
    } else if code == NTE_DEVICE_NOT_FOUND {
        FidoError::NoDevice
    } else if code == NTE_NOT_FOUND {
        FidoError::Other("this is not the security key the SSH key was created on".to_string())
    } else if code == NTE_NOT_SUPPORTED || code == NTE_INVALID_PARAMETER {
        FidoError::Unsupported(
            "Windows or this security key cannot do that with this key type".to_string(),
        )
    } else {
        FidoError::Other(error.message())
    }
}

/// The message Windows hashes with SHA-256 to make the client data hash.
/// For SSH this is the data being signed, not WebAuthn's usual JSON.
fn client_data(message: &[u8]) -> WEBAUTHN_CLIENT_DATA {
    WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: message.len() as u32,
        pbClientDataJSON: message.as_ptr().cast_mut(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    }
}

/// Copy `len` bytes Windows returned at `data`.
///
/// # Safety
/// `data` must be null or point to at least `len` readable bytes.
unsafe fn copy(data: *const u8, len: u32) -> Vec<u8> {
    if data.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: checked non-null above; the caller vouches for the length.
    unsafe { slice::from_raw_parts(data, len as usize) }.to_vec()
}

/// Frees an attestation Windows allocated, on every way out of the function.
struct Attestation(*mut WEBAUTHN_CREDENTIAL_ATTESTATION);

impl Drop for Attestation {
    fn drop(&mut self) {
        // SAFETY: the pointer came from WebAuthNAuthenticatorMakeCredential
        // and is freed exactly once.
        unsafe { WebAuthNFreeCredentialAttestation(Some(self.0)) }
    }
}

struct AssertionResult(*mut WEBAUTHN_ASSERTION);

impl Drop for AssertionResult {
    fn drop(&mut self) {
        // SAFETY: the pointer came from WebAuthNAuthenticatorGetAssertion
        // and is freed exactly once.
        unsafe { WebAuthNFreeAssertion(self.0) }
    }
}

impl Authenticator for WebAuthnAuthenticator {
    fn make_credential(
        &self,
        kind: SkKind,
        application: &str,
        user: &str,
        _pin: Option<&str>,
    ) -> Result<NewCredential, FidoError> {
        let application = HSTRING::from(application);
        let user_name = HSTRING::from(user);
        // OpenSSH uses an all-zero user id for keys that are not resident.
        let user_id = [0u8; 32];
        // The challenge only matters for attestation, which SSH does not use.
        let challenge: [u8; 32] = rand::random();

        let relying_party = WEBAUTHN_RP_ENTITY_INFORMATION {
            dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
            pwszId: PCWSTR(application.as_ptr()),
            pwszName: PCWSTR(application.as_ptr()),
            pwszIcon: PCWSTR::null(),
        };
        let user = WEBAUTHN_USER_ENTITY_INFORMATION {
            dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
            cbId: user_id.len() as u32,
            pbId: user_id.as_ptr().cast_mut(),
            pwszName: PCWSTR(user_name.as_ptr()),
            pwszIcon: PCWSTR::null(),
            pwszDisplayName: PCWSTR(user_name.as_ptr()),
        };
        let mut parameter = WEBAUTHN_COSE_CREDENTIAL_PARAMETER {
            dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            lAlg: match kind {
                SkKind::Ed25519 => COSE_ALGORITHM_EDDSA,
                SkKind::EcdsaP256 => WEBAUTHN_COSE_ALGORITHM_ECDSA_P256_WITH_SHA256,
            },
        };
        let parameters = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS {
            cCredentialParameters: 1,
            pCredentialParameters: &mut parameter,
        };
        let client_data = client_data(&challenge);
        // SAFETY: the options struct is plain data for which all-zero is the
        // documented "not set" value of every field.
        let mut options: WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS =
            unsafe { std::mem::zeroed() };
        // Version 1 covers every field set here; Windows ignores the rest.
        options.dwVersion = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_1;
        options.dwTimeoutMilliseconds = TIMEOUT_MS;
        // A security key, not the fingerprint reader or face camera.
        options.dwAuthenticatorAttachment = WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM;
        options.dwUserVerificationRequirement = WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED;
        options.dwAttestationConveyancePreference = WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE;

        // SAFETY: every pointer passed lives until the call returns, and the
        // structs carry the version numbers their layouts match.
        let attestation = unsafe {
            WebAuthNAuthenticatorMakeCredential(
                GetForegroundWindow(),
                &relying_party,
                &user,
                &parameters,
                &client_data,
                Some(&options),
            )
        }
        .map_err(|err| classify(&err))?;
        if attestation.is_null() {
            return Err(FidoError::Other(
                "Windows returned no credential".to_string(),
            ));
        }
        let attestation = Attestation(attestation);
        // SAFETY: non-null and valid until `attestation` is dropped; the
        // lengths come from Windows alongside the pointers.
        let (authenticator_data, key_handle) = unsafe {
            let result = &*attestation.0;
            (
                copy(result.pbAuthenticatorData, result.cbAuthenticatorData),
                copy(result.pbCredentialId, result.cbCredentialId),
            )
        };

        let public_key =
            cose::credential_public_key(&authenticator_data, kind).ok_or_else(|| {
                FidoError::Unsupported(
                    "the security key created a different key type than was asked for".to_string(),
                )
            })?;
        if key_handle.is_empty() {
            return Err(FidoError::Other(
                "Windows returned no key handle".to_string(),
            ));
        }
        Ok(NewCredential {
            kind,
            public_key,
            key_handle,
        })
    }

    fn get_assertion(
        &self,
        _kind: SkKind,
        application: &str,
        key_handle: &[u8],
        message: &[u8],
        _pin: Option<&str>,
    ) -> Result<Assertion, FidoError> {
        let application = HSTRING::from(application);
        let mut credential = WEBAUTHN_CREDENTIAL {
            dwVersion: WEBAUTHN_CREDENTIAL_CURRENT_VERSION,
            cbId: key_handle.len() as u32,
            pbId: key_handle.as_ptr().cast_mut(),
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
        };
        let client_data = client_data(message);
        // SAFETY: plain data for which all-zero means "not set".
        let mut options: WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS =
            unsafe { std::mem::zeroed() };
        options.dwVersion = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_1;
        options.dwTimeoutMilliseconds = TIMEOUT_MS;
        options.CredentialList = WEBAUTHN_CREDENTIALS {
            cCredentials: 1,
            pCredentials: &mut credential,
        };
        options.dwAuthenticatorAttachment = WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM;
        options.dwUserVerificationRequirement = WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED;

        // SAFETY: every pointer passed lives until the call returns.
        let assertion = unsafe {
            WebAuthNAuthenticatorGetAssertion(
                GetForegroundWindow(),
                PCWSTR(application.as_ptr()),
                &client_data,
                Some(&options),
            )
        }
        .map_err(|err| classify(&err))?;
        if assertion.is_null() {
            return Err(FidoError::Other(
                "Windows returned no signature".to_string(),
            ));
        }
        let assertion = AssertionResult(assertion);
        // SAFETY: non-null and valid until `assertion` is dropped.
        let (authenticator_data, signature) = unsafe {
            let result = &*assertion.0;
            (
                copy(result.pbAuthenticatorData, result.cbAuthenticatorData),
                copy(result.pbSignature, result.cbSignature),
            )
        };
        let (flags, counter) = super::parse_authenticator_data(&authenticator_data)
            .ok_or_else(|| FidoError::Other("the security key returned bad data".to_string()))?;
        Ok(Assertion {
            signature,
            flags,
            counter,
        })
    }

    fn has_own_dialogs(&self) -> bool {
        true
    }
}
