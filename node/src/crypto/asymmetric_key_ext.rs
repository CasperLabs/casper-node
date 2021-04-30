//! Additional operations an asymmetric key

use std::path::Path;

use derp::{Der, Tag};
use once_cell::sync::Lazy;
use pem::Pem;
#[cfg(test)]
use rand::{Rng, RngCore};
use untrusted::Input;

use casper_types::{AsymmetricKeyTag, AsymmetricType, PublicKey, SecretKey};

#[cfg(test)]
use crate::testing::TestRng;
use crate::{crypto::Error, utils};

// See https://tools.ietf.org/html/rfc8410#section-10.3
const ED25519_OBJECT_IDENTIFIER: [u8; 3] = [43, 101, 112];
const ED25519_PEM_SECRET_KEY_TAG: &str = "PRIVATE KEY";
const ED25519_PEM_PUBLIC_KEY_TAG: &str = "PUBLIC KEY";

// Ref?
const SECP256K1_OBJECT_IDENTIFIER: [u8; 5] = [43, 129, 4, 0, 10];
const SECP256K1_PEM_SECRET_KEY_TAG: &str = "EC PRIVATE KEY";
const SECP256K1_PEM_PUBLIC_KEY_TAG: &str = "PUBLIC KEY";

// See https://www.secg.org/sec1-v2.pdf#subsection.C.4
const EC_PUBLIC_KEY_OBJECT_IDENTIFIER: [u8; 7] = [42, 134, 72, 206, 61, 2, 1];

static ED25519_SECRET_KEY: Lazy<SecretKey> = Lazy::new(|| {
    let bytes = [15u8; SecretKey::ED25519_LENGTH];
    SecretKey::ed25519_from_bytes(bytes).unwrap()
});

static ED25519_PUBLIC_KEY: Lazy<PublicKey> = Lazy::new(|| {
    let bytes = [15u8; SecretKey::ED25519_LENGTH];
    let secret_key = SecretKey::ed25519_from_bytes(bytes).unwrap();
    PublicKey::from(&secret_key)
});

/// Additional operations an asymmetric key
pub trait AsymmetricKeyExt: Sized {
    /// Constructs a new ed25519 variant using the operating system's cryptographically secure
    /// random number generator.
    fn generate_ed25519() -> Result<Self, Error>;

    /// Constructs a new secp256k1 variant using the operating system's cryptographically secure
    /// random number generator.
    fn generate_secp256k1() -> Result<Self, Error>;

    /// Attempts to write the key bytes to the configured file path.
    fn to_file<P: AsRef<Path>>(&self, file: P) -> Result<(), Error>;

    /// Attempts to read the key bytes from configured file path.
    fn from_file<P: AsRef<Path>>(file: P) -> Result<Self, Error>;

    /// DER encodes a key.
    fn to_der(&self) -> Result<Vec<u8>, Error>;

    /// Decodes a key from a DER-encoded slice.
    fn from_der<T: AsRef<[u8]>>(input: T) -> Result<Self, Error>;

    /// PEM encodes a key.
    fn to_pem(&self) -> Result<String, Error>;

    /// Decodes a secret key from a PEM-encoded slice.
    fn from_pem<T: AsRef<[u8]>>(input: T) -> Result<Self, Error>;

    /// Generates a random instance using a `TestRng`.
    #[cfg(test)]
    fn random(rng: &mut TestRng) -> Self;

    /// Generates a random ed25519 instance using a `TestRng`.
    #[cfg(test)]
    fn random_ed25519(rng: &mut TestRng) -> Self;

    /// Generates a random secp256k1 instance using a `TestRng`.
    #[cfg(test)]
    fn random_secp256k1(rng: &mut TestRng) -> Self;

    /// Returns an example value for documentation purposes.
    fn doc_example() -> &'static Self;
}

impl AsymmetricKeyExt for SecretKey {
    fn generate_ed25519() -> Result<Self, Error> {
        let mut bytes = [0u8; Self::ED25519_LENGTH];
        getrandom::getrandom(&mut bytes[..])?;
        Ok(SecretKey::ed25519_from_bytes(bytes)?)
    }

    fn generate_secp256k1() -> Result<Self, Error> {
        let mut bytes = [0u8; Self::SECP256K1_LENGTH];
        getrandom::getrandom(&mut bytes[..])?;
        Ok(SecretKey::secp256k1_from_bytes(bytes)?)
    }

    fn to_file<P: AsRef<Path>>(&self, file: P) -> Result<(), Error> {
        utils::write_private_file(file, self.to_pem()?).map_err(Error::SecretKeySave)
    }

    fn from_file<P: AsRef<Path>>(file: P) -> Result<Self, Error> {
        let data = utils::read_file(file).map_err(Error::SecretKeyLoad)?;
        Self::from_pem(data)
    }

    fn to_der(&self) -> Result<Vec<u8>, Error> {
        match self {
            SecretKey::System => Err(Error::System(String::from("to_der"))),
            SecretKey::Ed25519(secret_key) => {
                // See https://tools.ietf.org/html/rfc8410#section-10.3
                let mut key_bytes = vec![];
                let mut der = Der::new(&mut key_bytes);
                der.octet_string(secret_key.as_ref())?;

                let mut encoded = vec![];
                der = Der::new(&mut encoded);
                der.sequence(|der| {
                    der.integer(&[0])?;
                    der.sequence(|der| der.oid(&ED25519_OBJECT_IDENTIFIER))?;
                    der.octet_string(&key_bytes)
                })?;
                Ok(encoded)
            }
            SecretKey::Secp256k1(secret_key) => {
                // See https://www.secg.org/sec1-v2.pdf#subsection.C.4
                let mut oid_bytes = vec![];
                let mut der = Der::new(&mut oid_bytes);
                der.oid(&SECP256K1_OBJECT_IDENTIFIER)?;

                let mut encoded = vec![];
                der = Der::new(&mut encoded);
                der.sequence(|der| {
                    der.integer(&[1])?;
                    der.octet_string(secret_key.to_bytes().as_slice())?;
                    der.element(Tag::ContextSpecificConstructed0, &oid_bytes)
                })?;
                Ok(encoded)
            }
        }
    }

    fn from_der<T: AsRef<[u8]>>(input: T) -> Result<Self, Error> {
        let input = Input::from(input.as_ref());

        let (key_type_tag, raw_bytes) = input.read_all(derp::Error::Read, |input| {
            derp::nested(input, Tag::Sequence, |input| {
                // Safe to ignore the first value which should be an integer.
                let version_slice =
                    derp::expect_tag_and_get_value(input, Tag::Integer)?.as_slice_less_safe();
                if version_slice.len() != 1 {
                    return Err(derp::Error::NonZeroUnusedBits);
                }
                let version = version_slice[0];

                // Read the next value.
                let (tag, value) = derp::read_tag_and_get_value(input)?;
                if tag == Tag::Sequence as u8 {
                    // Expecting an Ed25519 key.
                    if version != 0 {
                        return Err(derp::Error::WrongValue);
                    }

                    // The sequence should have one element: an object identifier defining Ed25519.
                    let object_identifier = value.read_all(derp::Error::Read, |input| {
                        derp::expect_tag_and_get_value(input, Tag::Oid)
                    })?;
                    if object_identifier.as_slice_less_safe() != ED25519_OBJECT_IDENTIFIER {
                        return Err(derp::Error::WrongValue);
                    }

                    // The third and final value should be the raw bytes of the secret key as an
                    // octet string in an octet string.
                    let raw_bytes = derp::nested(input, Tag::OctetString, |input| {
                        derp::expect_tag_and_get_value(input, Tag::OctetString)
                    })?
                    .as_slice_less_safe();

                    return Ok((AsymmetricKeyTag::Ed25519, raw_bytes));
                } else if tag == Tag::OctetString as u8 {
                    // Expecting a secp256k1 key.
                    if version != 1 {
                        return Err(derp::Error::WrongValue);
                    }

                    // The octet string is the secret key.
                    let raw_bytes = value.as_slice_less_safe();

                    // The object identifier is next.
                    let parameter0 =
                        derp::expect_tag_and_get_value(input, Tag::ContextSpecificConstructed0)?;
                    let object_identifier = parameter0.read_all(derp::Error::Read, |input| {
                        derp::expect_tag_and_get_value(input, Tag::Oid)
                    })?;
                    if object_identifier.as_slice_less_safe() != SECP256K1_OBJECT_IDENTIFIER {
                        return Err(derp::Error::WrongValue);
                    }

                    // There might be an optional public key as the final value, but we're not
                    // interested in parsing that.  Read it to ensure `input.read_all` doesn't fail
                    // with unused bytes error.
                    let _ = derp::read_tag_and_get_value(input);

                    return Ok((AsymmetricKeyTag::Secp256k1, raw_bytes));
                }

                Err(derp::Error::WrongValue)
            })
        })?;

        match key_type_tag {
            AsymmetricKeyTag::System => {
                Err(Error::AsymmetricKey("cannot construct variant".to_string()))
            }
            AsymmetricKeyTag::Ed25519 => {
                SecretKey::ed25519_from_bytes(raw_bytes).map_err(Into::into)
            }
            AsymmetricKeyTag::Secp256k1 => {
                SecretKey::secp256k1_from_bytes(raw_bytes).map_err(Into::into)
            }
        }
    }

    fn to_pem(&self) -> Result<String, Error> {
        let tag = match self {
            SecretKey::System => return Err(Error::System(String::from("to_pem"))),
            SecretKey::Ed25519(_) => ED25519_PEM_SECRET_KEY_TAG.to_string(),
            SecretKey::Secp256k1(_) => SECP256K1_PEM_SECRET_KEY_TAG.to_string(),
        };
        let contents = self.to_der()?;
        let pem = Pem { tag, contents };
        Ok(pem::encode(&pem))
    }

    fn from_pem<T: AsRef<[u8]>>(input: T) -> Result<Self, Error> {
        let pem = pem::parse(input)?;

        let secret_key = Self::from_der(&pem.contents)?;

        let bad_tag = |expected_tag: &str| {
            Error::FromPem(format!(
                "invalid tag: expected {}, got {}",
                expected_tag, pem.tag
            ))
        };

        match secret_key {
            SecretKey::System => return Err(Error::System(String::from("from_pem"))),
            SecretKey::Ed25519(_) => {
                if pem.tag != ED25519_PEM_SECRET_KEY_TAG {
                    return Err(bad_tag(ED25519_PEM_SECRET_KEY_TAG));
                }
            }
            SecretKey::Secp256k1(_) => {
                if pem.tag != SECP256K1_PEM_SECRET_KEY_TAG {
                    return Err(bad_tag(SECP256K1_PEM_SECRET_KEY_TAG));
                }
            }
        }

        Ok(secret_key)
    }

    #[cfg(test)]
    fn random(rng: &mut TestRng) -> Self {
        if rng.gen() {
            Self::random_ed25519(rng)
        } else {
            Self::random_secp256k1(rng)
        }
    }

    #[cfg(test)]
    fn random_ed25519(rng: &mut TestRng) -> Self {
        let mut bytes = [0u8; Self::ED25519_LENGTH];
        rng.fill_bytes(&mut bytes[..]);
        SecretKey::ed25519_from_bytes(bytes).unwrap()
    }

    #[cfg(test)]
    fn random_secp256k1(rng: &mut TestRng) -> Self {
        let mut bytes = [0u8; Self::SECP256K1_LENGTH];
        rng.fill_bytes(&mut bytes[..]);
        SecretKey::secp256k1_from_bytes(bytes).unwrap()
    }

    fn doc_example() -> &'static Self {
        &*ED25519_SECRET_KEY
    }
}

impl AsymmetricKeyExt for PublicKey {
    fn generate_ed25519() -> Result<Self, Error> {
        let mut bytes = [0u8; Self::ED25519_LENGTH];
        getrandom::getrandom(&mut bytes[..]).expect("RNG failure!");
        PublicKey::ed25519_from_bytes(bytes).map_err(Into::into)
    }

    fn generate_secp256k1() -> Result<Self, Error> {
        let mut bytes = [0u8; Self::SECP256K1_LENGTH];
        getrandom::getrandom(&mut bytes[..]).expect("RNG failure!");
        PublicKey::secp256k1_from_bytes(bytes).map_err(Into::into)
    }

    fn to_file<P: AsRef<Path>>(&self, file: P) -> Result<(), Error> {
        utils::write_file(file, self.to_pem()?).map_err(Error::PublicKeySave)
    }

    fn from_file<P: AsRef<Path>>(file: P) -> Result<Self, Error> {
        let data = utils::read_file(file).map_err(Error::PublicKeyLoad)?;
        Self::from_pem(data)
    }

    fn to_der(&self) -> Result<Vec<u8>, Error> {
        match self {
            PublicKey::System => Err(Error::System(String::from("to_der"))),
            PublicKey::Ed25519(public_key) => {
                // See https://tools.ietf.org/html/rfc8410#section-10.1
                let mut encoded = vec![];
                let mut der = Der::new(&mut encoded);
                der.sequence(|der| {
                    der.sequence(|der| der.oid(&ED25519_OBJECT_IDENTIFIER))?;
                    der.bit_string(0, public_key.as_ref())
                })?;
                Ok(encoded)
            }
            PublicKey::Secp256k1(public_key) => {
                // See https://www.secg.org/sec1-v2.pdf#subsection.C.3
                let mut encoded = vec![];
                let mut der = Der::new(&mut encoded);
                der.sequence(|der| {
                    der.sequence(|der| {
                        der.oid(&EC_PUBLIC_KEY_OBJECT_IDENTIFIER)?;
                        der.oid(&SECP256K1_OBJECT_IDENTIFIER)
                    })?;
                    der.bit_string(0, &public_key.to_bytes())
                })?;
                Ok(encoded)
            }
        }
    }

    fn from_der<T: AsRef<[u8]>>(input: T) -> Result<Self, Error> {
        let input = Input::from(input.as_ref());

        let (key_type_tag, raw_bytes) = input.read_all(derp::Error::Read, |input| {
            derp::nested(input, Tag::Sequence, |input| {
                let tag = derp::nested(input, Tag::Sequence, |input| {
                    // Read the first value.
                    let object_identifier =
                        derp::expect_tag_and_get_value(input, Tag::Oid)?.as_slice_less_safe();
                    if object_identifier == ED25519_OBJECT_IDENTIFIER {
                        Ok(AsymmetricKeyTag::Ed25519)
                    } else if object_identifier == EC_PUBLIC_KEY_OBJECT_IDENTIFIER {
                        // Assert the next object identifier is the secp256k1 ID.
                        let next_object_identifier =
                            derp::expect_tag_and_get_value(input, Tag::Oid)?.as_slice_less_safe();
                        if next_object_identifier != SECP256K1_OBJECT_IDENTIFIER {
                            return Err(derp::Error::WrongValue);
                        }

                        Ok(AsymmetricKeyTag::Secp256k1)
                    } else {
                        Err(derp::Error::WrongValue)
                    }
                })?;
                Ok((
                    tag,
                    derp::bit_string_with_no_unused_bits(input)?.as_slice_less_safe(),
                ))
            })
        })?;

        match key_type_tag {
            AsymmetricKeyTag::Ed25519 => {
                PublicKey::ed25519_from_bytes(raw_bytes).map_err(Into::into)
            }
            AsymmetricKeyTag::Secp256k1 => {
                PublicKey::secp256k1_from_bytes(raw_bytes).map_err(Into::into)
            }
            _ => unreachable!(),
        }
    }

    fn to_pem(&self) -> Result<String, Error> {
        let tag = match self {
            PublicKey::System => return Err(Error::System(String::from("to_pem"))),
            PublicKey::Ed25519(_) => ED25519_PEM_PUBLIC_KEY_TAG.to_string(),
            PublicKey::Secp256k1(_) => SECP256K1_PEM_PUBLIC_KEY_TAG.to_string(),
        };
        let contents = self.to_der()?;
        let pem = Pem { tag, contents };
        Ok(pem::encode(&pem))
    }

    fn from_pem<T: AsRef<[u8]>>(input: T) -> Result<Self, Error> {
        let pem = pem::parse(input)?;
        let public_key = Self::from_der(&pem.contents)?;
        let bad_tag = |expected_tag: &str| {
            Error::FromPem(format!(
                "invalid tag: expected {}, got {}",
                expected_tag, pem.tag
            ))
        };
        match public_key {
            PublicKey::System => return Err(Error::System(String::from("from_pem"))),
            PublicKey::Ed25519(_) => {
                if pem.tag != ED25519_PEM_PUBLIC_KEY_TAG {
                    return Err(bad_tag(ED25519_PEM_PUBLIC_KEY_TAG));
                }
            }
            PublicKey::Secp256k1(_) => {
                if pem.tag != SECP256K1_PEM_PUBLIC_KEY_TAG {
                    return Err(bad_tag(SECP256K1_PEM_PUBLIC_KEY_TAG));
                }
            }
        }
        Ok(public_key)
    }

    #[cfg(test)]
    fn random(rng: &mut TestRng) -> Self {
        let secret_key = SecretKey::random(rng);
        PublicKey::from(&secret_key)
    }

    #[cfg(test)]
    fn random_ed25519(rng: &mut TestRng) -> Self {
        let secret_key = SecretKey::random_ed25519(rng);
        PublicKey::from(&secret_key)
    }

    #[cfg(test)]
    fn random_secp256k1(rng: &mut TestRng) -> Self {
        let secret_key = SecretKey::random_secp256k1(rng);
        PublicKey::from(&secret_key)
    }

    fn doc_example() -> &'static Self {
        &*ED25519_PUBLIC_KEY
    }
}
