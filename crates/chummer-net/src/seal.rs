//! End-to-end sealing of mailbox blobs.
//!
//! A blob is sealed to the recipient's [`EndpointId`] and signed by the
//! sender, so the relay operator can read neither the content nor the
//! author, and the recipient can check who wrote it.
//!
//! Construction (sign, then seal):
//! 1. The sender signs `DOMAIN || recipient_id || payload` with its iroh
//!    ed25519 node key. Binding the recipient into the signature stops a
//!    recipient from re-sealing a signed message to someone else and passing
//!    it off as sent to them.
//! 2. The envelope `(sender_id, signature, payload)` is encrypted with a
//!    libsodium-compatible sealed box (`crypto_box_seal`: ephemeral X25519 +
//!    XSalsa20-Poly1305) to the recipient's X25519 key.
//!
//! The X25519 keys are the standard birational conversion of the ed25519
//! node keys (as libsodium's `crypto_sign_ed25519_pk_to_curve25519` /
//! `_sk_to_curve25519`), so no extra key has to be published.
//!
//! Crates: `crypto_box` 0.9 (RustCrypto, `seal` feature) for the sealed box,
//! `ed25519-dalek` (through iroh's key types) for signatures and the key
//! conversion. No home-made primitives.
//!
//! Sealing does not stop replays: the relay could deliver the same blob
//! twice. Payloads must carry their own ids so the sync layer can drop
//! duplicates.

use iroh::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};

const DOMAIN: &[u8] = b"chummer-rs/mailbox-seal/1\0";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealError {
    #[error("could not encrypt the message")]
    Encrypt,
    #[error("could not decrypt the message (not for this key, or damaged)")]
    Decrypt,
    #[error("the message is damaged")]
    Malformed,
    #[error("the sender's signature does not match")]
    BadSignature,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    sender: [u8; 32],
    #[serde(with = "sig_bytes")]
    signature: [u8; 64],
    payload: Vec<u8>,
}

/// A blob that was opened and whose signature checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// The author, proven by the signature.
    pub sender: PublicKey,
    pub payload: Vec<u8>,
}

fn signed_message(recipient: &PublicKey, payload: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(DOMAIN.len() + 32 + payload.len());
    m.extend_from_slice(DOMAIN);
    m.extend_from_slice(recipient.as_bytes());
    m.extend_from_slice(payload);
    m
}

fn x25519_public(key: &PublicKey) -> crypto_box::PublicKey {
    crypto_box::PublicKey::from(key.as_verifying_key().to_montgomery().to_bytes())
}

fn x25519_secret(key: &SecretKey) -> crypto_box::SecretKey {
    crypto_box::SecretKey::from(key.as_signing_key().to_scalar_bytes())
}

/// Signs `payload` with `sender` and seals it to `recipient`.
pub fn seal(
    sender: &SecretKey,
    recipient: &PublicKey,
    payload: &[u8],
) -> Result<Vec<u8>, SealError> {
    let signature = sender.sign(&signed_message(recipient, payload));
    let envelope = Envelope {
        sender: *sender.public().as_bytes(),
        signature: signature.to_bytes(),
        payload: payload.to_vec(),
    };
    let plain = postcard::to_stdvec(&envelope).map_err(|_| SealError::Encrypt)?;
    x25519_public(recipient)
        .seal(&mut crypto_box::aead::OsRng, &plain)
        .map_err(|_| SealError::Encrypt)
}

/// Opens a blob sealed to `recipient` and checks the sender's signature.
pub fn open(recipient: &SecretKey, blob: &[u8]) -> Result<Opened, SealError> {
    let plain = x25519_secret(recipient)
        .unseal(blob)
        .map_err(|_| SealError::Decrypt)?;
    let envelope: Envelope = postcard::from_bytes(&plain).map_err(|_| SealError::Malformed)?;
    let sender = PublicKey::from_bytes(&envelope.sender).map_err(|_| SealError::Malformed)?;
    let signature = Signature::from_bytes(&envelope.signature);
    sender
        .verify(
            &signed_message(&recipient.public(), &envelope.payload),
            &signature,
        )
        .map_err(|_| SealError::BadSignature)?;
    Ok(Opened {
        sender,
        payload: envelope.payload,
    })
}

mod sig_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let v: &[u8] = Deserialize::deserialize(d)?;
        v.try_into()
            .map_err(|_| serde::de::Error::invalid_length(v.len(), &"64 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip() {
        let alice = SecretKey::generate();
        let bob = SecretKey::generate();
        let blob = seal(&alice, &bob.public(), b"karma +5").unwrap();
        assert!(!blob.windows(8).any(|w| w == b"karma +5"));
        let opened = open(&bob, &blob).unwrap();
        assert_eq!(opened.sender, alice.public());
        assert_eq!(opened.payload, b"karma +5");
    }

    #[test]
    fn wrong_recipient_cannot_open() {
        let alice = SecretKey::generate();
        let bob = SecretKey::generate();
        let eve = SecretKey::generate();
        let blob = seal(&alice, &bob.public(), b"secret").unwrap();
        assert_eq!(open(&eve, &blob), Err(SealError::Decrypt));
    }

    #[test]
    fn tampering_is_detected() {
        let alice = SecretKey::generate();
        let bob = SecretKey::generate();
        let mut blob = seal(&alice, &bob.public(), b"secret").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert_eq!(open(&bob, &blob), Err(SealError::Decrypt));
    }

    #[test]
    fn forged_or_resealed_signature_is_rejected() {
        let alice = SecretKey::generate();
        let bob = SecretKey::generate();
        let eve = SecretKey::generate();
        // Eve claims Alice wrote this: the signature cannot match.
        let mut envelope = Envelope {
            sender: *alice.public().as_bytes(),
            signature: eve
                .sign(&signed_message(&bob.public(), b"forged"))
                .to_bytes(),
            payload: b"forged".to_vec(),
        };
        let blob = x25519_public(&bob.public())
            .seal(
                &mut crypto_box::aead::OsRng,
                &postcard::to_stdvec(&envelope).unwrap(),
            )
            .unwrap();
        assert_eq!(open(&bob, &blob), Err(SealError::BadSignature));

        // Eve got a message Alice sealed to Eve and re-seals it to Bob.
        let to_eve = open(&eve, &seal(&alice, &eve.public(), b"hi eve").unwrap()).unwrap();
        envelope.payload = to_eve.payload;
        envelope.signature = alice
            .sign(&signed_message(&eve.public(), b"hi eve"))
            .to_bytes();
        let blob = x25519_public(&bob.public())
            .seal(
                &mut crypto_box::aead::OsRng,
                &postcard::to_stdvec(&envelope).unwrap(),
            )
            .unwrap();
        assert_eq!(open(&bob, &blob), Err(SealError::BadSignature));
    }
}
