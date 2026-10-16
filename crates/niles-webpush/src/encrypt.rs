//! Message encryption for Web Push (RFC 8291), the `aes128gcm` content
//! coding of RFC 8188 with one record.
//!
//! The push service carries the message but cannot read it: each one
//! is encrypted for the device's own key (`p256dh`) with a fresh key
//! pair and salt, and the device's `auth` secret goes into the key
//! derivation, so only the browser that subscribed can open it.

use crate::error::{Error, Result};
use aes_gcm::aead::Aead;
use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use p256::elliptic_curve::Generate;
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey};
use rand::Rng;
use sha2::Sha256;

/// One record holds the whole message, so the record size only has to
/// be at least the message; 4096 is what every push service accepts.
const RECORD_SIZE: u32 = 4096;

/// The header before the ciphertext: salt, record size, key id length
/// and the key id, which is the sender's public key.
const HEADER_LEN: usize = 16 + 4 + 1 + 65;

/// The most a message can be: a 4096-byte record, less its header, the
/// cipher's tag and the padding delimiter.
pub const MAX_PLAINTEXT: usize = RECORD_SIZE as usize - HEADER_LEN - 16 - 1;

/// `plaintext`, encrypted for the subscription whose public key is
/// `p256dh` (65 bytes, uncompressed) and whose secret is `auth` (16
/// bytes). The result is the request body, header included.
pub fn encrypt(p256dh: &[u8], auth: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    encrypt_with(&SecretKey::generate(), &salt, p256dh, auth, plaintext)
}

/// [`encrypt`] with the sender's key and the salt given, which only a
/// test vector has any business doing: reusing either across messages
/// gives the encryption away.
fn encrypt_with(
    sender: &SecretKey,
    salt: &[u8; 16],
    p256dh: &[u8],
    auth: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(invalid(format!(
            "it is {} bytes, and a push holds {MAX_PLAINTEXT}",
            plaintext.len()
        )));
    }
    if auth.len() != 16 {
        return Err(invalid(format!(
            "its auth secret is {} bytes, not 16",
            auth.len()
        )));
    }
    let device = PublicKey::from_sec1_bytes(p256dh)
        .map_err(|_| invalid("its p256dh key is not a P-256 point".into()))?;
    let device_public = device.to_sec1_point(false);
    let sender_public = sender.public_key().to_sec1_point(false);
    let shared = sender.diffie_hellman(&device);

    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(device_public.as_bytes());
    key_info.extend_from_slice(sender_public.as_bytes());
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|_| invalid("the key derivation failed".into()))?;

    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .and_then(|()| prk.expand(b"Content-Encoding: nonce\0", &mut nonce))
        .map_err(|_| invalid("the key derivation failed".into()))?;

    // 0x02 marks the last (and only) record; no padding after it.
    let mut record = Vec::with_capacity(plaintext.len() + 1);
    record.extend_from_slice(plaintext);
    record.push(2);
    let sealed = Aes128Gcm::new_from_slice(&cek)
        .map_err(|_| invalid("the content key was refused".into()))?
        .encrypt(&Nonce::from(nonce), record.as_slice())
        .map_err(|_| invalid("it could not be encrypted".into()))?;

    let mut body = Vec::with_capacity(HEADER_LEN + sealed.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(65);
    body.extend_from_slice(sender_public.as_bytes());
    body.extend_from_slice(&sealed);
    Ok(body)
}

fn invalid(reason: String) -> Error {
    Error::Invalid {
        kind: "push message",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vapid::b64url_decode;

    fn b(s: &str) -> Vec<u8> {
        b64url_decode(&s.replace(' ', "")).expect("base64url")
    }

    /// RFC 8291 Appendix A, every input as the RFC gives it.
    #[test]
    fn matches_the_rfc_8291_test_vector() {
        let sender =
            SecretKey::from_slice(&b("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let salt: [u8; 16] = b("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let p256dh = b("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcx \
             aOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
        let auth = b("BTBZMqHH6r4Tts7J_aSIgg");
        let plaintext = b("V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24");

        let body = encrypt_with(&sender, &salt, &p256dh, &auth, &plaintext).unwrap();

        let header = b(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z 9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml \
             mlMoZIIgDll6e3vCYLocInmYWAmS6Tlz AC8wEqKK6PBru3jl7A8",
        );
        let ciphertext = b(
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrd irN6GcG7sFz1y1sqLgVi1VhjVkHsUoEs \
             bI_0LpXMuGvnzQ",
        );
        assert_eq!(header.len(), HEADER_LEN);
        assert_eq!(&body[..HEADER_LEN], header.as_slice());
        assert_eq!(&body[HEADER_LEN..], ciphertext.as_slice());
    }

    fn device() -> (Vec<u8>, Vec<u8>) {
        let key = SecretKey::generate();
        (
            key.public_key().to_sec1_point(false).as_bytes().to_vec(),
            vec![7; 16],
        )
    }

    #[test]
    fn every_message_has_its_own_salt_and_key() {
        let (p256dh, auth) = device();
        let first = encrypt(&p256dh, &auth, b"same").unwrap();
        let second = encrypt(&p256dh, &auth, b"same").unwrap();
        assert_ne!(first[..16], second[..16], "salt");
        assert_ne!(first[21..HEADER_LEN], second[21..HEADER_LEN], "sender key");
        assert_ne!(first, second);
    }

    #[test]
    fn a_message_too_big_for_one_record_is_refused() {
        let (p256dh, auth) = device();
        assert!(encrypt(&p256dh, &auth, &vec![b'x'; MAX_PLAINTEXT]).is_ok());
        assert!(encrypt(&p256dh, &auth, &vec![b'x'; MAX_PLAINTEXT + 1]).is_err());
    }

    #[test]
    fn a_key_that_is_not_a_point_is_refused() {
        let (_, auth) = device();
        assert!(encrypt(&[4; 65], &auth, b"hi").is_err());
        assert!(encrypt(&[], &auth, b"hi").is_err());
    }

    #[test]
    fn an_auth_secret_of_the_wrong_size_is_refused() {
        let (p256dh, _) = device();
        assert!(encrypt(&p256dh, &[1; 15], b"hi").is_err());
    }
}
