//! The release signature: the public key embedded in the binary, and the
//! check of the Ed25519 signature the release workflow puts on `SHA256SUMS`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signature, VerifyingKey};
use thiserror::Error;

/// Public key of the release signature (PEM, `SubjectPublicKeyInfo`).
const EMBEDDED_KEY: &str = include_str!("../../assets/release-signing.pub");

/// DER prefix of an Ed25519 `SubjectPublicKeyInfo`, before the 32 key bytes.
const SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

const PEM_BEGIN: &str = "-----BEGIN PUBLIC KEY-----";
const PEM_END: &str = "-----END PUBLIC KEY-----";

/// Why a key or a signature is refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignatureError {
    /// The public key is not a PEM Ed25519 key.
    #[error("the release public key is not a valid Ed25519 key")]
    InvalidKey,
    /// The signature is not 64 bytes.
    #[error("the signature is malformed")]
    MalformedSignature,
    /// The signature does not match the signed bytes.
    #[error("the signature does not match")]
    Mismatch,
}

/// The key the releases are signed with.
pub struct ReleaseKey(VerifyingKey);

impl ReleaseKey {
    /// The key embedded in this binary.
    ///
    /// # Errors
    ///
    /// [`SignatureError::InvalidKey`] when the embedded file is damaged.
    pub fn embedded() -> Result<Self, SignatureError> {
        Self::from_pem(EMBEDDED_KEY)
    }

    /// A key read from a PEM `PUBLIC KEY` block.
    ///
    /// # Errors
    ///
    /// [`SignatureError::InvalidKey`] when it is not an Ed25519 key.
    pub fn from_pem(pem: &str) -> Result<Self, SignatureError> {
        let body: String = pem
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && *line != PEM_BEGIN && *line != PEM_END)
            .collect();
        let der = BASE64
            .decode(body)
            .map_err(|_| SignatureError::InvalidKey)?;
        let raw: [u8; 32] = der
            .strip_prefix(&SPKI_PREFIX)
            .and_then(|key| key.try_into().ok())
            .ok_or(SignatureError::InvalidKey)?;
        VerifyingKey::from_bytes(&raw)
            .map(Self)
            .map_err(|_| SignatureError::InvalidKey)
    }

    /// Checks that `signature` signs exactly `message`.
    ///
    /// # Errors
    ///
    /// [`SignatureError::MalformedSignature`] or [`SignatureError::Mismatch`].
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), SignatureError> {
        let signature =
            Signature::from_slice(signature).map_err(|_| SignatureError::MalformedSignature)?;
        self.0
            .verify_strict(message, &signature)
            .map_err(|_| SignatureError::Mismatch)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

    /// A throw-away signing key and its PEM public key, for the tests.
    pub(crate) fn test_keys(seed: u8) -> (SigningKey, String) {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let mut der = SPKI_PREFIX.to_vec();
        der.extend_from_slice(signing.verifying_key().as_bytes());
        let pem = format!("{PEM_BEGIN}\n{}\n{PEM_END}\n", BASE64.encode(der));
        (signing, pem)
    }

    /// A signature made by the command the release workflow runs (`openssl
    /// pkeyutl -sign -rawin`, OpenSSL 3.0) with a throw-away key: the format
    /// the helper must accept.
    #[test]
    fn a_signature_made_by_openssl_verifies() {
        const PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\n\
            MCowBQYDK2VwAyEAW6KpncmHmpQyUdsFUw3mwxVM5UUtw8OWHIgO1bgpcAA=\n\
            -----END PUBLIC KEY-----\n";
        const MESSAGE: &[u8] = b"abc123  hubergamof_1.0.0_amd64.deb\n";
        const SIGNATURE_HEX: &str = "0db4cdb23f6d93470416fa358a6f2bc728561ded753eb25728514dc878cbefb6\
            e238682249207915797cfb598ee0af395eb0d930b78b008183e19b77b2604007";
        let signature: Vec<u8> = (0..SIGNATURE_HEX.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&SIGNATURE_HEX[index..index + 2], 16).unwrap())
            .collect();
        let key = ReleaseKey::from_pem(PUBLIC_KEY).unwrap();
        assert_eq!(key.verify(MESSAGE, &signature), Ok(()));
        assert_eq!(
            key.verify(b"abc124  hubergamof_1.0.0_amd64.deb\n", &signature),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn the_embedded_key_is_a_valid_ed25519_key() {
        assert!(ReleaseKey::embedded().is_ok());
    }

    #[test]
    fn a_signature_verifies_only_the_signed_bytes() {
        let (signing, pem) = test_keys(7);
        let key = ReleaseKey::from_pem(&pem).unwrap();
        let signature = signing.sign(b"checksums").to_bytes();
        assert_eq!(key.verify(b"checksums", &signature), Ok(()));
        assert_eq!(
            key.verify(b"checksums\n", &signature),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn another_key_does_not_verify() {
        let (signing, _) = test_keys(7);
        let (_, other_pem) = test_keys(8);
        let other = ReleaseKey::from_pem(&other_pem).unwrap();
        let signature = signing.sign(b"checksums").to_bytes();
        assert_eq!(
            other.verify(b"checksums", &signature),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn a_malformed_signature_is_refused() {
        let (_, pem) = test_keys(7);
        let key = ReleaseKey::from_pem(&pem).unwrap();
        for signature in [&[][..], &[0; 63], &[0; 65]] {
            assert_eq!(
                key.verify(b"checksums", signature),
                Err(SignatureError::MalformedSignature)
            );
        }
        assert_eq!(
            key.verify(b"checksums", &[0; 64]),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn keys_that_are_not_ed25519_are_refused() {
        for pem in [
            "",
            "not a key",
            "-----BEGIN PUBLIC KEY-----\n!!!\n-----END PUBLIC KEY-----",
            // An RSA-sized body: valid base64, wrong structure.
            "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----",
        ] {
            assert_eq!(
                ReleaseKey::from_pem(pem).err(),
                Some(SignatureError::InvalidKey),
                "{pem:?}"
            );
        }
    }
}
