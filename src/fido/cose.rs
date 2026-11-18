//! Reads the public key out of WebAuthn authenticator data.
//!
//! When a security key creates a credential it returns the new public key as
//! a COSE key (RFC 9052) inside the "attested credential data". Only the two
//! key types SSH uses are understood.

use super::SkKind;

/// The public key of a newly created credential, in the form
/// [`super::NewCredential::public_key`] wants: the 32-byte key for ED25519,
/// the SEC1 uncompressed point for ECDSA P-256.
///
/// `data` is `rpIdHash[32] flags[1] counter[4] aaguid[16] idLen[2] id[idLen]`
/// followed by the COSE key.
pub fn credential_public_key(data: &[u8], kind: SkKind) -> Option<Vec<u8>> {
    const ATTESTED_CREDENTIAL_DATA: u8 = 0x40;
    if data.get(32)? & ATTESTED_CREDENTIAL_DATA == 0 {
        return None;
    }
    let id_len = u16::from_be_bytes(data.get(53..55)?.try_into().ok()?) as usize;
    let key = CoseKey::parse(data.get(55 + id_len..)?)?;

    const OKP: i64 = 1;
    const EC2: i64 = 2;
    const CURVE_P256: i64 = 1;
    const CURVE_ED25519: i64 = 6;
    match kind {
        SkKind::Ed25519 => {
            let x = key.x.filter(|x| x.len() == 32)?;
            (key.key_type == Some(OKP) && key.curve == Some(CURVE_ED25519)).then(|| x.to_vec())
        }
        SkKind::EcdsaP256 => {
            let x = key.x.filter(|x| x.len() == 32)?;
            let y = key.y.filter(|y| y.len() == 32)?;
            (key.key_type == Some(EC2) && key.curve == Some(CURVE_P256))
                .then(|| [&[0x04][..], x, y].concat())
        }
    }
}

#[derive(Default)]
struct CoseKey<'a> {
    key_type: Option<i64>,
    curve: Option<i64>,
    x: Option<&'a [u8]>,
    y: Option<&'a [u8]>,
}

enum Value<'a> {
    Int(i64),
    Bytes(&'a [u8]),
}

impl<'a> CoseKey<'a> {
    /// A COSE key is a CBOR map from small integers to integers or byte
    /// strings. Anything else is rejected rather than skipped.
    fn parse(mut input: &'a [u8]) -> Option<Self> {
        const MAP: u8 = 5;
        let (major, entries) = head(&mut input)?;
        if major != MAP {
            return None;
        }
        let mut key = Self::default();
        for _ in 0..entries {
            let Value::Int(label) = value(&mut input)? else {
                return None;
            };
            match (label, value(&mut input)?) {
                (1, Value::Int(key_type)) => key.key_type = Some(key_type),
                (-1, Value::Int(curve)) => key.curve = Some(curve),
                (-2, Value::Bytes(x)) => key.x = Some(x),
                (-3, Value::Bytes(y)) => key.y = Some(y),
                _ => {}
            }
        }
        Some(key)
    }
}

/// Major type and argument of the next CBOR item.
fn head(input: &mut &[u8]) -> Option<(u8, u64)> {
    let (&first, rest) = input.split_first()?;
    *input = rest;
    let extra = match first & 0x1f {
        small @ 0..=23 => return Some((first >> 5, small.into())),
        24 => 1,
        25 => 2,
        26 => 4,
        27 => 8,
        _ => return None,
    };
    let bytes = input.get(..extra)?;
    *input = &input[extra..];
    let argument = bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
    Some((first >> 5, argument))
}

fn value<'a>(input: &mut &'a [u8]) -> Option<Value<'a>> {
    const UNSIGNED: u8 = 0;
    const NEGATIVE: u8 = 1;
    const BYTES: u8 = 2;
    let (major, argument) = head(input)?;
    match major {
        UNSIGNED => Some(Value::Int(i64::try_from(argument).ok()?)),
        NEGATIVE => Some(Value::Int(-1 - i64::try_from(argument).ok()?)),
        BYTES => {
            let len = usize::try_from(argument).ok()?;
            let bytes = input.get(..len)?;
            *input = &input[len..];
            Some(Value::Bytes(bytes))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authenticator_data(flags: u8, credential_id: &[u8], cose_key: &[u8]) -> Vec<u8> {
        let mut data = vec![0xAA; 32];
        data.push(flags);
        data.extend([0, 0, 0, 7]);
        data.extend([0xBB; 16]);
        data.extend((credential_id.len() as u16).to_be_bytes());
        data.extend(credential_id);
        data.extend(cose_key);
        data
    }

    /// `{1: 1, 3: -8, -1: 6, -2: x}`
    fn ed25519_key(x: &[u8]) -> Vec<u8> {
        let mut key = vec![0xA4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20];
        key.extend(x);
        key
    }

    /// `{1: 2, 3: -7, -1: 1, -2: x, -3: y}`
    fn p256_key(x: &[u8], y: &[u8]) -> Vec<u8> {
        let mut key = vec![0xA5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
        key.extend(x);
        key.extend([0x22, 0x58, 0x20]);
        key.extend(y);
        key
    }

    #[test]
    fn ed25519_key_is_read() {
        let data = authenticator_data(0x41, &[9; 64], &ed25519_key(&[5; 32]));
        assert_eq!(
            credential_public_key(&data, SkKind::Ed25519),
            Some(vec![5; 32])
        );
        assert_eq!(credential_public_key(&data, SkKind::EcdsaP256), None);
    }

    #[test]
    fn p256_key_becomes_an_uncompressed_point() {
        // A long credential id, as security keys really return.
        let data = authenticator_data(0x45, &[9; 300], &p256_key(&[1; 32], &[2; 32]));
        let point = credential_public_key(&data, SkKind::EcdsaP256).unwrap();
        assert_eq!(point.len(), 65);
        assert_eq!(point[0], 0x04);
        assert_eq!(&point[1..33], &[1; 32]);
        assert_eq!(&point[33..], &[2; 32]);
        assert_eq!(credential_public_key(&data, SkKind::Ed25519), None);
    }

    #[test]
    fn data_without_a_credential_is_rejected() {
        let key = ed25519_key(&[5; 32]);
        // The "attested credential data" flag is not set.
        let no_flag = authenticator_data(0x01, &[9; 16], &key);
        assert_eq!(credential_public_key(&no_flag, SkKind::Ed25519), None);
        // Cut off anywhere, it must fail cleanly rather than panic.
        let whole = authenticator_data(0x41, &[9; 16], &key);
        for len in 0..whole.len() {
            assert_eq!(credential_public_key(&whole[..len], SkKind::Ed25519), None);
        }
        // Wrong coordinate size.
        let short = authenticator_data(0x41, &[9; 16], &ed25519_key(&[5; 32])[..40]);
        assert_eq!(credential_public_key(&short, SkKind::Ed25519), None);
    }

    #[test]
    fn unexpected_cbor_is_rejected() {
        // An array instead of a map, and a map with a text-string value.
        for cose in [&[0x84, 0x01, 0x01][..], &[0xA1, 0x01, 0x61, 0x41][..]] {
            let data = authenticator_data(0x41, &[9; 16], cose);
            assert_eq!(credential_public_key(&data, SkKind::Ed25519), None);
        }
    }
}
