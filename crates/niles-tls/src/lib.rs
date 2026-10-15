//! Trusting a device that signed its own certificate.
//!
//! An LG TV, a Bambu Lab printer: each answers TLS with a certificate
//! nobody issued, so there is no chain to check — only that whoever
//! answers holds the key in it. That much has to work whatever the
//! certificate looks like, and these devices often present X.509 v1,
//! which webpki will not read at all. So the key is taken straight from
//! the certificate's DER, and the handshake signature is checked against
//! the key alone.

use rustls::DigitallySignedStruct;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, SubjectPublicKeyInfoDer};
use sha2::{Digest, Sha256};

/// A certificate's `subjectPublicKeyInfo`, whole, whatever its version.
pub fn public_key_info(certificate: &[u8]) -> Result<&[u8], String> {
    let unreadable = || "the certificate cannot be read".to_string();
    let (tag, cert, _) = der(certificate).ok_or_else(unreadable)?;
    let (tbs_tag, tbs, _) = der(cert).ok_or_else(unreadable)?;
    if tag != 0x30 || tbs_tag != 0x30 {
        return Err(unreadable());
    }
    let mut rest = tbs;
    // `[0] version`, which a v1 certificate leaves out.
    if rest.first() == Some(&0xA0) {
        rest = der(rest).ok_or_else(unreadable)?.2;
    }
    // Serial number, signature algorithm, issuer, validity, subject.
    for _ in 0..5 {
        rest = der(rest).ok_or_else(unreadable)?.2;
    }
    let (key_tag, _, after) = der(rest).ok_or_else(unreadable)?;
    if key_tag != 0x30 {
        return Err(unreadable());
    }
    Ok(&rest[..rest.len() - after.len()])
}

/// The SHA-256 of a certificate's public key, as hex pairs: what is
/// pinned when a device is trusted on first use.
pub fn fingerprint(certificate: &CertificateDer<'_>) -> Result<String, String> {
    let digest = Sha256::digest(public_key_info(certificate)?);
    Ok(digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":"))
}

/// One DER element at the front of `input`: its tag, its contents, and
/// what follows it.
pub fn der(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |len, byte| (len << 8) | usize::from(*byte));
        (len, &rest[n..])
    };
    (rest.len() >= len).then(|| (tag, &rest[..len], &rest[len..]))
}

/// Which TLS version a handshake signature belongs to: they are checked
/// by slightly different rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tls {
    V12,
    V13,
}

/// The handshake signature, checked against the key in `cert` — the
/// proof that whoever answered holds it.
pub fn signed_by_key(
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
    algorithms: &WebPkiSupportedAlgorithms,
    version: Tls,
) -> Result<HandshakeSignatureValid, rustls::Error> {
    let spki = SubjectPublicKeyInfoDer::from(
        public_key_info(cert.as_ref()).map_err(rustls::Error::General)?,
    );
    if version == Tls::V13 {
        return rustls::crypto::verify_tls13_signature_with_raw_key(
            message, &spki, dss, algorithms,
        );
    }
    let key = webpki::RawPublicKeyEntity::try_from(&spki)
        .map_err(|e| rustls::Error::General(format!("the device's key cannot be used: {e}")))?;
    let candidates = algorithms
        .mapping
        .iter()
        .find(|(scheme, _)| *scheme == dss.scheme)
        .map(|(_, candidates)| *candidates)
        .ok_or(rustls::Error::PeerMisbehaved(
            rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme,
        ))?;
    let mut last = None;
    for algorithm in candidates {
        match key.verify_signature(*algorithm, message, dss.signature()) {
            Ok(()) => return Ok(HandshakeSignatureValid::assertion()),
            Err(e @ webpki::Error::UnsupportedSignatureAlgorithmForPublicKeyContext(_)) => {
                last = Some(e);
            }
            Err(e) => {
                return Err(rustls::Error::General(format!(
                    "the device's handshake signature is wrong: {e}"
                )));
            }
        }
    }
    Err(rustls::Error::General(format!(
        "the device's key cannot make that signature{}",
        last.map(|e| format!(": {e}")).unwrap_or_default()
    )))
}

/// The same certificate as X.509 v1: no `[0] version`, no extensions —
/// what these devices present. Its signature no longer matches, which
/// trusting a device never checks; the key is what matters, and a
/// handshake still proves the server holds it.
#[cfg(any(test, feature = "test-support"))]
pub fn as_version_1(certificate: &[u8]) -> Vec<u8> {
    fn tlv(tag: u8, contents: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        let len = contents.len();
        if len < 0x80 {
            out.push(len as u8);
        } else {
            let bytes: Vec<u8> = len
                .to_be_bytes()
                .into_iter()
                .skip_while(|b| *b == 0)
                .collect();
            out.push(0x80 | bytes.len() as u8);
            out.extend(bytes);
        }
        out.extend_from_slice(contents);
        out
    }
    let (_, cert, _) = der(certificate).expect("a certificate");
    let (_, tbs, after_tbs) = der(cert).expect("a certificate");
    let mut fields = Vec::new();
    let mut rest = tbs;
    while !rest.is_empty() {
        let (tag, _, next) = der(rest).expect("a certificate");
        if tag != 0xA0 && tag != 0xA3 {
            fields.extend_from_slice(&rest[..rest.len() - next.len()]);
        }
        rest = next;
    }
    let mut body = tlv(0x30, &fields);
    body.extend_from_slice(after_tbs);
    tlv(0x30, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_1_certificate_has_the_same_key_as_its_version_3_self() {
        let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
        let v1 = as_version_1(made.cert.der());
        assert_ne!(v1.as_slice(), made.cert.der().as_ref());
        assert_eq!(
            fingerprint(&v1.into()).unwrap(),
            fingerprint(made.cert.der()).unwrap()
        );
    }

    #[test]
    fn reads_one_element_and_says_what_follows() {
        assert_eq!(
            der(&[0x02, 0x01, 0x05, 0xff]),
            Some((0x02, &[0x05][..], &[0xff][..]))
        );
        let long = [&[0x04, 0x81, 0x80][..], &[0u8; 0x80]].concat();
        assert_eq!(der(&long).unwrap().1.len(), 0x80);
    }

    #[test]
    fn a_short_or_lying_length_is_refused() {
        assert_eq!(der(&[0x02, 0x05, 0x01]), None);
        assert_eq!(der(&[0x02]), None);
        assert_eq!(der(&[0x02, 0x80]), None);
    }

    #[test]
    fn something_that_is_not_a_certificate_is_said_plainly() {
        assert_eq!(
            fingerprint(&vec![0x30, 0x03, 0x02, 0x01, 0x00].into()).unwrap_err(),
            "the certificate cannot be read"
        );
    }
}
