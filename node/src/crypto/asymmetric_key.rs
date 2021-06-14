//! Asymmetric-key types and functions.

// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use ed25519_dalek::ExpandedSecretKey;
use k256::ecdsa::{
    signature::{Signer, Verifier},
    Signature as Secp256k1Signature, VerifyingKey as Secp256k1PublicKey,
};
#[cfg(test)]
use k256::elliptic_curve::sec1::ToEncodedPoint;

use casper_types::{PublicKey, SecretKey, Signature};

pub use super::{Error, Result};
use crate::crypto::AsymmetricKeyExt;

/// Generates an Ed25519 keypair using the operating system's cryptographically secure random number
/// generator.
pub fn generate_ed25519_keypair() -> (SecretKey, PublicKey) {
    let secret_key = SecretKey::generate_ed25519().unwrap();
    let public_key = PublicKey::from(&secret_key);
    (secret_key, public_key)
}

/// Signs the given message using the given key pair.
pub fn sign<T: AsRef<[u8]>>(
    message: T,
    secret_key: &SecretKey,
    public_key: &PublicKey,
) -> Signature {
    match (secret_key, public_key) {
        (SecretKey::System, PublicKey::System) => {
            panic!("cannot create signature with system keys",)
        }
        (SecretKey::Ed25519(secret_key), PublicKey::Ed25519(public_key)) => {
            let expanded_secret_key = ExpandedSecretKey::from(secret_key);
            let signature = expanded_secret_key.sign(message.as_ref(), public_key);
            Signature::Ed25519(signature)
        }
        (SecretKey::Secp256k1(secret_key), PublicKey::Secp256k1(_public_key)) => {
            let signer = secret_key;
            let signature: Secp256k1Signature = signer
                .try_sign(message.as_ref())
                .expect("should create signature");
            Signature::Secp256k1(signature)
        }
        _ => panic!("secret and public key types must match"),
    }
}

/// Verifies the signature of the given message against the given public key.
pub fn verify<T: AsRef<[u8]>>(
    message: T,
    signature: &Signature,
    public_key: &PublicKey,
) -> Result<()> {
    match (signature, public_key) {
        (Signature::System, _) => Err(Error::AsymmetricKey(String::from(
            "signatures based on the system key cannot be verified",
        ))),
        (Signature::Ed25519(signature), PublicKey::Ed25519(public_key)) => public_key
            .verify_strict(message.as_ref(), signature)
            .map_err(|_| Error::AsymmetricKey(String::from("failed to verify Ed25519 signature"))),
        (Signature::Secp256k1(signature), PublicKey::Secp256k1(public_key)) => {
            let verifier: &Secp256k1PublicKey = public_key;
            verifier
                .verify(message.as_ref(), signature)
                .map_err(|error| {
                    Error::AsymmetricKey(format!("failed to verify secp256k1 signature: {}", error))
                })
        }
        _ => Err(Error::AsymmetricKey(format!(
            "type mismatch between {} and {}",
            signature, public_key
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cmp::Ordering,
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
        iter,
    };

    use rand::RngCore;

    use openssl::pkey::{PKey, Private, Public};

    use casper_types::{bytesrepr, check_summed_hex, AsymmetricType, Tagged};

    use super::*;
    use crate::{crypto::AsymmetricKeyExt, testing::TestRng};

    type OpenSSLSecretKey = PKey<Private>;
    type OpenSSLPublicKey = PKey<Public>;

    // `SecretKey` does not implement `PartialEq`, so just compare derived `PublicKey`s.
    fn assert_secret_keys_equal(lhs: &SecretKey, rhs: &SecretKey) {
        assert_eq!(PublicKey::from(lhs), PublicKey::from(rhs));
    }

    fn secret_key_der_roundtrip(secret_key: SecretKey) {
        let der_encoded = secret_key.to_der().unwrap();
        let decoded = SecretKey::from_der(&der_encoded).unwrap();
        assert_secret_keys_equal(&secret_key, &decoded);
        assert_eq!(secret_key.tag(), decoded.tag());

        // Ensure malformed encoded version fails to decode.
        SecretKey::from_der(&der_encoded[1..]).unwrap_err();
    }

    fn secret_key_pem_roundtrip(secret_key: SecretKey) {
        let pem_encoded = secret_key.to_pem().unwrap();
        let decoded = SecretKey::from_pem(pem_encoded.as_bytes()).unwrap();
        assert_secret_keys_equal(&secret_key, &decoded);
        assert_eq!(secret_key.tag(), decoded.tag());

        // Check PEM-encoded can be decoded by openssl.
        let _ = OpenSSLSecretKey::private_key_from_pem(pem_encoded.as_bytes()).unwrap();

        // Ensure malformed encoded version fails to decode.
        SecretKey::from_pem(&pem_encoded[1..]).unwrap_err();
    }

    fn known_secret_key_to_pem(expected_key: &SecretKey, known_key_pem: &str, expected_tag: u8) {
        let decoded = SecretKey::from_pem(known_key_pem.as_bytes()).unwrap();
        assert_secret_keys_equal(expected_key, &decoded);
        assert_eq!(expected_tag, decoded.tag());
    }

    fn secret_key_file_roundtrip(secret_key: SecretKey) {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("test_secret_key.pem");

        secret_key.to_file(&path).unwrap();
        let decoded = SecretKey::from_file(&path).unwrap();
        assert_secret_keys_equal(&secret_key, &decoded);
        assert_eq!(secret_key.tag(), decoded.tag());
    }

    fn public_key_serialization_roundtrip(public_key: PublicKey) {
        // Try to/from bincode.
        let serialized = bincode::serialize(&public_key).unwrap();
        let deserialized = bincode::deserialize(&serialized).unwrap();
        assert_eq!(public_key, deserialized);
        assert_eq!(public_key.tag(), deserialized.tag());

        // Try to/from JSON.
        let serialized = serde_json::to_vec_pretty(&public_key).unwrap();
        let deserialized = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(public_key, deserialized);
        assert_eq!(public_key.tag(), deserialized.tag());

        // Using bytesrepr.
        bytesrepr::test_serialization_roundtrip(&public_key);
    }

    fn public_key_der_roundtrip(public_key: PublicKey) {
        let der_encoded = public_key.to_der().unwrap();
        let decoded = PublicKey::from_der(&der_encoded).unwrap();
        assert_eq!(public_key, decoded);

        // Check DER-encoded can be decoded by openssl.
        let _ = OpenSSLPublicKey::public_key_from_der(&der_encoded).unwrap();

        // Ensure malformed encoded version fails to decode.
        PublicKey::from_der(&der_encoded[1..]).unwrap_err();
    }

    fn public_key_pem_roundtrip(public_key: PublicKey) {
        let pem_encoded = public_key.to_pem().unwrap();
        let decoded = PublicKey::from_pem(pem_encoded.as_bytes()).unwrap();
        assert_eq!(public_key, decoded);
        assert_eq!(public_key.tag(), decoded.tag());

        // Check PEM-encoded can be decoded by openssl.
        let _ = OpenSSLPublicKey::public_key_from_pem(pem_encoded.as_bytes()).unwrap();

        // Ensure malformed encoded version fails to decode.
        PublicKey::from_pem(&pem_encoded[1..]).unwrap_err();
    }

    fn known_public_key_to_pem(known_key_hex: &str, known_key_pem: &str) {
        let key_bytes = check_summed_hex::decode(known_key_hex).unwrap();
        let decoded = PublicKey::from_pem(known_key_pem.as_bytes()).unwrap();
        assert_eq!(key_bytes, Into::<Vec<u8>>::into(decoded));
    }

    fn public_key_file_roundtrip(public_key: PublicKey) {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("test_public_key.pem");

        public_key.to_file(&path).unwrap();
        let decoded = PublicKey::from_file(&path).unwrap();
        assert_eq!(public_key, decoded);
    }

    fn public_key_hex_roundtrip(public_key: PublicKey) {
        let hex_encoded = public_key.to_hex();
        let decoded = PublicKey::from_hex(&hex_encoded).unwrap();
        assert_eq!(public_key, decoded);
        assert_eq!(public_key.tag(), decoded.tag());

        // Ensure malformed encoded version fails to decode.
        PublicKey::from_hex(&hex_encoded[..1]).unwrap_err();
        PublicKey::from_hex(&hex_encoded[1..]).unwrap_err();
    }

    fn signature_serialization_roundtrip(signature: Signature) {
        // Try to/from bincode.
        let serialized = bincode::serialize(&signature).unwrap();
        let deserialized: Signature = bincode::deserialize(&serialized).unwrap();
        assert_eq!(signature, deserialized);
        assert_eq!(signature.tag(), deserialized.tag());

        // Try to/from JSON.
        let serialized = serde_json::to_vec_pretty(&signature).unwrap();
        let deserialized = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(signature, deserialized);
        assert_eq!(signature.tag(), deserialized.tag());

        // Try to/from using bytesrepr.
        let serialized = bytesrepr::serialize(signature).unwrap();
        let deserialized = bytesrepr::deserialize(serialized).unwrap();
        assert_eq!(signature, deserialized);
        assert_eq!(signature.tag(), deserialized.tag())
    }

    fn signature_hex_roundtrip(signature: Signature) {
        let hex_encoded = signature.to_hex();
        let decoded = Signature::from_hex(hex_encoded.as_bytes()).unwrap();
        assert_eq!(signature, decoded);
        assert_eq!(signature.tag(), decoded.tag());

        // Ensure malformed encoded version fails to decode.
        Signature::from_hex(&hex_encoded[..1]).unwrap_err();
        Signature::from_hex(&hex_encoded[1..]).unwrap_err();
    }

    fn hash<T: Hash>(data: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        hasher.finish()
    }

    fn check_ord_and_hash<T: Clone + Ord + PartialOrd + Hash>(low: T, high: T) {
        let low_copy = low.clone();

        assert_eq!(hash(&low), hash(&low_copy));
        assert_ne!(hash(&low), hash(&high));

        assert_eq!(Ordering::Less, low.cmp(&high));
        assert_eq!(Some(Ordering::Less), low.partial_cmp(&high));

        assert_eq!(Ordering::Greater, high.cmp(&low));
        assert_eq!(Some(Ordering::Greater), high.partial_cmp(&low));

        assert_eq!(Ordering::Equal, low.cmp(&low_copy));
        assert_eq!(Some(Ordering::Equal), low.partial_cmp(&low_copy));
    }

    mod ed25519 {
        use rand::Rng;

        use casper_types::{check_summed_hex, ED25519_TAG};

        use super::*;
        use crate::crypto::AsymmetricKeyExt;

        const SECRET_KEY_LENGTH: usize = SecretKey::ED25519_LENGTH;
        const PUBLIC_KEY_LENGTH: usize = PublicKey::ED25519_LENGTH;
        const SIGNATURE_LENGTH: usize = Signature::ED25519_LENGTH;

        #[test]
        fn secret_key_from_bytes() {
            // Secret key should be `SecretKey::ED25519_LENGTH` bytes.
            let bytes = [0; SECRET_KEY_LENGTH + 1];
            assert!(SecretKey::ed25519_from_bytes(&bytes[..]).is_err());
            assert!(SecretKey::ed25519_from_bytes(&bytes[2..]).is_err());

            // Check the same bytes but of the right length succeeds.
            assert!(SecretKey::ed25519_from_bytes(&bytes[1..]).is_ok());
        }

        #[test]
        fn secret_key_to_and_from_der() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_ed25519(&mut rng);
            let der_encoded = secret_key.to_der().unwrap();
            secret_key_der_roundtrip(secret_key);

            // Check DER-encoded can be decoded by openssl.
            let _ = OpenSSLSecretKey::private_key_from_der(&der_encoded).unwrap();
        }

        #[test]
        fn secret_key_to_and_from_pem() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_ed25519(&mut rng);
            secret_key_pem_roundtrip(secret_key);
        }

        #[test]
        fn known_secret_key_to_pem() {
            // Example values taken from https://tools.ietf.org/html/rfc8410#section-10.3
            const KNOWN_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/nwoy/HU++CXqI9EdVhC
-----END PRIVATE KEY-----"#;
            let key_bytes = check_summed_hex::decode(
                "d4ee72dbf913584ad5b6d8f1f769f8ad3afe7c28cbf1d4fbe097a88f44755842",
            )
            .unwrap();
            let expected_key = SecretKey::ed25519_from_bytes(key_bytes).unwrap();
            super::known_secret_key_to_pem(&expected_key, KNOWN_KEY_PEM, ED25519_TAG);
        }

        #[test]
        fn secret_key_to_and_from_file() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_ed25519(&mut rng);
            secret_key_file_roundtrip(secret_key);
        }

        #[test]
        fn public_key_serialization_roundtrip() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_ed25519(&mut rng);
            super::public_key_serialization_roundtrip(public_key);
        }

        #[test]
        fn public_key_from_bytes() {
            // Public key should be `PublicKey::ED25519_LENGTH` bytes.  Create vec with an extra
            // byte.
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_ed25519(&mut rng);
            let bytes: Vec<u8> = iter::once(rng.gen())
                .chain(Into::<Vec<u8>>::into(public_key))
                .collect::<Vec<u8>>();

            assert!(PublicKey::ed25519_from_bytes(&bytes[..]).is_err());
            assert!(PublicKey::ed25519_from_bytes(&bytes[2..]).is_err());

            // Check the same bytes but of the right length succeeds.
            assert!(PublicKey::ed25519_from_bytes(&bytes[1..]).is_ok());
        }

        #[test]
        fn public_key_to_and_from_der() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_ed25519(&mut rng);
            public_key_der_roundtrip(public_key);
        }

        #[test]
        fn public_key_to_and_from_pem() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_ed25519(&mut rng);
            public_key_pem_roundtrip(public_key);
        }

        #[test]
        fn known_public_key_to_pem() {
            // Example values taken from https://tools.ietf.org/html/rfc8410#section-10.1
            const KNOWN_KEY_HEX: &str =
                "19bf44096984cdfe8541bac167dc3b96c85086aa30b6b6cb0c5c38ad703166e1";
            const KNOWN_KEY_PEM: &str = r#"-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=
-----END PUBLIC KEY-----"#;
            super::known_public_key_to_pem(KNOWN_KEY_HEX, KNOWN_KEY_PEM);
        }

        #[test]
        fn public_key_to_and_from_file() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_ed25519(&mut rng);
            public_key_file_roundtrip(public_key);
        }

        #[test]
        fn public_key_to_and_from_hex() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_ed25519(&mut rng);
            public_key_hex_roundtrip(public_key);
        }

        #[test]
        fn signature_serialization_roundtrip() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_ed25519(&mut rng);
            let public_key = PublicKey::from(&secret_key);
            let data = b"data";
            let signature = sign(data, &secret_key, &public_key);
            super::signature_serialization_roundtrip(signature);
        }

        #[test]
        fn signature_from_bytes() {
            // Signature should be < ~2^(252.5).
            let invalid_bytes = [255; SIGNATURE_LENGTH];
            assert!(Signature::ed25519_from_bytes(&invalid_bytes[..]).is_err());

            // Signature should be `Signature::ED25519_LENGTH` bytes.
            let bytes = [2; SIGNATURE_LENGTH + 1];
            assert!(Signature::ed25519_from_bytes(&bytes[..]).is_err());
            assert!(Signature::ed25519_from_bytes(&bytes[2..]).is_err());

            // Check the same bytes but of the right length succeeds.
            assert!(Signature::ed25519_from_bytes(&bytes[1..]).is_ok());
        }

        #[test]
        fn signature_key_to_and_from_hex() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_ed25519(&mut rng);
            let public_key = PublicKey::from(&secret_key);
            let data = b"data";
            let signature = sign(data, &secret_key, &public_key);
            signature_hex_roundtrip(signature);
        }

        #[test]
        fn public_key_traits() {
            let public_key_low = PublicKey::ed25519_from_bytes([1; PUBLIC_KEY_LENGTH]).unwrap();
            let public_key_high = PublicKey::ed25519_from_bytes([3; PUBLIC_KEY_LENGTH]).unwrap();
            check_ord_and_hash(public_key_low, public_key_high)
        }

        #[test]
        fn public_key_to_account_hash() {
            let public_key_high = PublicKey::ed25519_from_bytes([255; PUBLIC_KEY_LENGTH]).unwrap();
            assert_ne!(
                public_key_high.to_account_hash().as_ref(),
                Into::<Vec<u8>>::into(public_key_high)
            );
        }

        #[test]
        fn signature_traits() {
            let signature_low = Signature::ed25519([1; SIGNATURE_LENGTH]).unwrap();
            let signature_high = Signature::ed25519([3; SIGNATURE_LENGTH]).unwrap();
            check_ord_and_hash(signature_low, signature_high)
        }

        #[test]
        fn sign_and_verify() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_ed25519(&mut rng);

            let public_key = PublicKey::from(&secret_key);
            let other_public_key = PublicKey::random_ed25519(&mut rng);
            let wrong_type_public_key = PublicKey::random_secp256k1(&mut rng);

            let message = b"message";
            let signature = sign(message, &secret_key, &public_key);

            assert!(verify(message, &signature, &public_key).is_ok());
            assert!(verify(message, &signature, &other_public_key).is_err());
            assert!(verify(message, &signature, &wrong_type_public_key).is_err());
            assert!(verify(&message[1..], &signature, &public_key).is_err());
        }

        #[test]
        fn bytesrepr_roundtrip_signature() {
            let mut rng = TestRng::new();
            let ed25519_secret_key = SecretKey::random_ed25519(&mut rng);
            let public_key = PublicKey::from(&ed25519_secret_key);
            let data = b"data";
            let signature = sign(data, &ed25519_secret_key, &public_key);
            bytesrepr::test_serialization_roundtrip(&signature);
        }
    }

    mod secp256k1 {
        use rand::Rng;

        use casper_types::SECP256K1_TAG;

        use super::*;
        use crate::crypto::AsymmetricKeyExt;

        const SECRET_KEY_LENGTH: usize = SecretKey::SECP256K1_LENGTH;
        const SIGNATURE_LENGTH: usize = Signature::SECP256K1_LENGTH;

        #[test]
        fn secret_key_from_bytes() {
            // Secret key should be `SecretKey::SECP256K1_LENGTH` bytes.
            // The k256 library will ensure that a byte stream of a length not equal to
            // `SECP256K1_LENGTH` will fail due to an assertion internal to the library.
            // We can check that invalid byte streams e.g [0;32] does not generate a valid key.
            let bytes = [0; SECRET_KEY_LENGTH];
            assert!(SecretKey::secp256k1_from_bytes(&bytes[..]).is_err());

            // Check that a valid byte stream produces a valid key
            let bytes = [1; SECRET_KEY_LENGTH];
            assert!(SecretKey::secp256k1_from_bytes(&bytes[..]).is_ok());
        }

        #[test]
        fn secret_key_to_and_from_der() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_secp256k1(&mut rng);
            secret_key_der_roundtrip(secret_key);
        }

        #[test]
        fn secret_key_to_and_from_pem() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_secp256k1(&mut rng);
            secret_key_pem_roundtrip(secret_key);
        }

        #[test]
        fn known_secret_key_to_pem() {
            // Example values taken from Python client.
            const KNOWN_KEY_PEM: &str = r#"-----BEGIN EC PRIVATE KEY-----
MHQCAQEEIL3fqaMKAfXSK1D2PnVVbZlZ7jTv133nukq4+95s6kmcoAcGBSuBBAAK
oUQDQgAEQI6VJjFv0fje9IDdRbLMcv/XMnccnOtdkv+kBR5u4ISEAkuc2TFWQHX0
Yj9oTB9fx9+vvQdxJOhMtu46kGo0Uw==
-----END EC PRIVATE KEY-----"#;
            let key_bytes = check_summed_hex::decode(
                "bddfa9a30a01f5d22b50f63e75556d9959ee34efd77de7ba4ab8fbde6cea499c",
            )
            .unwrap();
            let expected_key = SecretKey::secp256k1_from_bytes(key_bytes).unwrap();
            super::known_secret_key_to_pem(&expected_key, KNOWN_KEY_PEM, SECP256K1_TAG);
        }

        #[test]
        fn secret_key_to_and_from_file() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_secp256k1(&mut rng);
            secret_key_file_roundtrip(secret_key);
        }

        #[test]
        fn public_key_serialization_roundtrip() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_secp256k1(&mut rng);
            super::public_key_serialization_roundtrip(public_key);
        }

        #[test]
        fn public_key_from_bytes() {
            // Public key should be `PublicKey::SECP256K1_LENGTH` bytes.  Create vec with an extra
            // byte.
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_secp256k1(&mut rng);
            let bytes: Vec<u8> = iter::once(rng.gen())
                .chain(Into::<Vec<u8>>::into(public_key))
                .collect::<Vec<u8>>();

            assert!(PublicKey::secp256k1_from_bytes(&bytes[..]).is_err());
            assert!(PublicKey::secp256k1_from_bytes(&bytes[2..]).is_err());

            // Check the same bytes but of the right length succeeds.
            assert!(PublicKey::secp256k1_from_bytes(&bytes[1..]).is_ok());
        }

        #[test]
        fn public_key_to_and_from_der() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_secp256k1(&mut rng);
            public_key_der_roundtrip(public_key);
        }

        #[test]
        fn public_key_to_and_from_pem() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_secp256k1(&mut rng);
            public_key_pem_roundtrip(public_key);
        }

        #[test]
        fn known_public_key_to_pem() {
            // Example values taken from Python client.
            const KNOWN_KEY_HEX: &str =
                "03408e9526316fd1f8def480dd45b2cc72ffd732771c9ceb5d92ffa4051e6ee084";
            const KNOWN_KEY_PEM: &str = r#"-----BEGIN PUBLIC KEY-----
MFYwEAYHKoZIzj0CAQYFK4EEAAoDQgAEQI6VJjFv0fje9IDdRbLMcv/XMnccnOtd
kv+kBR5u4ISEAkuc2TFWQHX0Yj9oTB9fx9+vvQdxJOhMtu46kGo0Uw==
-----END PUBLIC KEY-----"#;
            super::known_public_key_to_pem(KNOWN_KEY_HEX, KNOWN_KEY_PEM);
        }

        #[test]
        fn public_key_to_and_from_file() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_secp256k1(&mut rng);
            public_key_file_roundtrip(public_key);
        }

        #[test]
        fn public_key_to_and_from_hex() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_secp256k1(&mut rng);
            public_key_hex_roundtrip(public_key);
        }

        #[test]
        fn signature_serialization_roundtrip() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_secp256k1(&mut rng);
            let public_key = PublicKey::from(&secret_key);
            let data = b"data";
            let signature = sign(data, &secret_key, &public_key);
            super::signature_serialization_roundtrip(signature);
        }

        #[test]
        fn bytesrepr_roundtrip_signature() {
            let mut rng = TestRng::new();
            let secret_key = SecretKey::random_secp256k1(&mut rng);
            let public_key = PublicKey::from(&secret_key);
            let data = b"data";
            let signature = sign(data, &secret_key, &public_key);
            bytesrepr::test_serialization_roundtrip(&signature);
        }

        #[test]
        fn signature_from_bytes() {
            // Signature should be `Signature::SECP256K1_LENGTH` bytes.
            let bytes = [2; SIGNATURE_LENGTH + 1];
            assert!(Signature::secp256k1_from_bytes(&bytes[..]).is_err());
            assert!(Signature::secp256k1_from_bytes(&bytes[2..]).is_err());

            // Check the same bytes but of the right length succeeds.
            assert!(Signature::secp256k1_from_bytes(&bytes[1..]).is_ok());
        }

        #[test]
        fn signature_key_to_and_from_hex() {
            let mut rng = crate::new_rng();
            let secret_key = SecretKey::random_secp256k1(&mut rng);
            let public_key = PublicKey::from(&secret_key);
            let data = b"data";
            let signature = sign(data, &secret_key, &public_key);
            signature_hex_roundtrip(signature);
        }

        #[test]
        fn public_key_traits() {
            let mut rng = crate::new_rng();
            let public_key1 = PublicKey::random_secp256k1(&mut rng);
            let public_key2 = PublicKey::random_secp256k1(&mut rng);
            if Into::<Vec<u8>>::into(public_key1.clone())
                < Into::<Vec<u8>>::into(public_key2.clone())
            {
                check_ord_and_hash(public_key1, public_key2)
            } else {
                check_ord_and_hash(public_key2, public_key1)
            }
        }

        #[test]
        fn public_key_to_account_hash() {
            let mut rng = crate::new_rng();
            let public_key = PublicKey::random_secp256k1(&mut rng);
            assert_ne!(
                public_key.to_account_hash().as_ref(),
                Into::<Vec<u8>>::into(public_key)
            );
        }

        #[test]
        fn signature_traits() {
            let signature_low = Signature::secp256k1([1; SIGNATURE_LENGTH]).unwrap();
            let signature_high = Signature::secp256k1([3; SIGNATURE_LENGTH]).unwrap();
            check_ord_and_hash(signature_low, signature_high)
        }
    }

    #[test]
    fn public_key_traits() {
        let mut rng = crate::new_rng();
        let ed25519_public_key = PublicKey::random_ed25519(&mut rng);
        let secp256k1_public_key = PublicKey::random_secp256k1(&mut rng);
        check_ord_and_hash(ed25519_public_key, secp256k1_public_key);
    }

    #[test]
    fn signature_traits() {
        let signature_low = Signature::ed25519([3; Signature::ED25519_LENGTH]).unwrap();
        let signature_high = Signature::secp256k1([1; Signature::SECP256K1_LENGTH]).unwrap();
        check_ord_and_hash(signature_low, signature_high)
    }

    #[test]
    fn sign_and_verify() {
        let mut rng = crate::new_rng();
        let ed25519_secret_key = SecretKey::random_ed25519(&mut rng);
        let secp256k1_secret_key = SecretKey::random_secp256k1(&mut rng);

        let ed25519_public_key = PublicKey::from(&ed25519_secret_key);
        let secp256k1_public_key = PublicKey::from(&secp256k1_secret_key);

        let other_ed25519_public_key = PublicKey::random_ed25519(&mut rng);
        let other_secp256k1_public_key = PublicKey::random_secp256k1(&mut rng);

        let message = b"message";
        let ed25519_signature = sign(message, &ed25519_secret_key, &ed25519_public_key);
        let secp256k1_signature = sign(message, &secp256k1_secret_key, &secp256k1_public_key);

        assert!(verify(message, &ed25519_signature, &ed25519_public_key).is_ok());
        assert!(verify(message, &secp256k1_signature, &secp256k1_public_key).is_ok());

        assert!(verify(message, &ed25519_signature, &other_ed25519_public_key).is_err());
        assert!(verify(message, &secp256k1_signature, &other_secp256k1_public_key).is_err());

        assert!(verify(message, &ed25519_signature, &secp256k1_public_key).is_err());
        assert!(verify(message, &secp256k1_signature, &ed25519_public_key).is_err());

        assert!(verify(&message[1..], &ed25519_signature, &ed25519_public_key).is_err());
        assert!(verify(&message[1..], &secp256k1_signature, &secp256k1_public_key).is_err());
    }

    #[test]
    fn should_construct_secp256k1_from_uncompressed_bytes() {
        let mut rng = crate::new_rng();

        let mut secret_key_bytes = [0u8; SecretKey::SECP256K1_LENGTH];
        rng.fill_bytes(&mut secret_key_bytes[..]);

        // Construct a secp256k1 secret key and use that to construct a public key.
        let secp256k1_secret_key = k256::SecretKey::from_bytes(secret_key_bytes).unwrap();
        let secp256k1_public_key = secp256k1_secret_key.public_key();

        // Construct a CL secret key and public key from that (which will be a compressed key).
        let secret_key = SecretKey::secp256k1_from_bytes(secret_key_bytes).unwrap();
        let public_key = PublicKey::from(&secret_key);
        assert_eq!(
            Into::<Vec<u8>>::into(public_key.clone()).len(),
            PublicKey::SECP256K1_LENGTH
        );
        assert_ne!(
            secp256k1_public_key
                .to_encoded_point(false)
                .as_bytes()
                .len(),
            PublicKey::SECP256K1_LENGTH
        );

        // Construct a CL public key from uncompressed public key bytes and ensure it's compressed.
        let from_uncompressed_bytes = PublicKey::secp256k1_from_bytes(
            secp256k1_public_key.to_encoded_point(false).as_bytes(),
        )
        .unwrap();
        assert_eq!(public_key, from_uncompressed_bytes);

        // Construct a CL public key from the uncompressed one's hex representation and ensure it's
        // compressed.
        let uncompressed_hex = {
            let mut bytes = vec![0x02u8];
            bytes.extend_from_slice(secp256k1_public_key.to_encoded_point(false).as_bytes());
            check_summed_hex::encode(&bytes)
        };

        format!(
            "02{}",
            check_summed_hex::encode(secp256k1_public_key.to_encoded_point(false).as_bytes())
                .to_lowercase()
        );
        let from_uncompressed_hex = PublicKey::from_hex(&uncompressed_hex).unwrap();
        assert_eq!(public_key, from_uncompressed_hex);
    }

    #[test]
    fn generate_ed25519_should_generate_an_ed25519_key() {
        let secret_key = SecretKey::generate_ed25519().unwrap();
        assert!(matches!(secret_key, SecretKey::Ed25519(_)))
    }

    #[test]
    fn generate_secp256k1_should_generate_an_secp256k1_key() {
        let secret_key = SecretKey::generate_secp256k1().unwrap();
        assert!(matches!(secret_key, SecretKey::Secp256k1(_)))
    }
}
