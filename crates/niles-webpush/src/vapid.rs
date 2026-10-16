//! Which server is sending (VAPID, RFC 8292).
//!
//! A browser subscribes with this server's public key, and from then
//! on its push service takes messages for that subscription only with
//! a token signed by the matching private key. One key pair serves the
//! whole house, made the first time anybody turns notifications on and
//! kept (sealed) in the database: a new key would orphan every device
//! already subscribed.

use crate::error::{Error, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::elliptic_curve::Generate;

/// How long a token is good for. RFC 8292 allows a day; Apple refuses
/// one that reaches further, so this stays well inside it.
const VALID_FOR: Duration = Duration::hours(12);

pub struct VapidKey {
    key: SigningKey,
}

impl std::fmt::Debug for VapidKey {
    /// The public half only. A `Debug` that printed the private key
    /// would put it in a log line sooner or later.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VapidKey")
            .field("public", &self.public_key())
            .finish_non_exhaustive()
    }
}

impl VapidKey {
    pub fn generate() -> Self {
        Self {
            key: SigningKey::generate(),
        }
    }

    /// The private key as [`VapidKey::to_base64url`] wrote it.
    pub fn from_base64url(private: &str) -> Result<Self> {
        let raw = b64url_decode(private).ok_or_else(|| invalid("it is not base64url"))?;
        let key = SigningKey::from_slice(&raw).map_err(|_| invalid("it is not a P-256 key"))?;
        Ok(Self { key })
    }

    /// The private key, 32 bytes as base64url, for the database to seal.
    pub fn to_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.key.to_bytes())
    }

    /// The public key as a browser takes it for `applicationServerKey`:
    /// the uncompressed point, base64url.
    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.key.verifying_key().to_sec1_point(false).as_bytes())
    }

    /// The `Authorization` header for a push to `endpoint`.
    ///
    /// `sub` is how the push service's operator could reach whoever
    /// runs this server. RFC 8292 makes it optional; Apple does not,
    /// and answers a token without a usable one with 403 BadJwtToken.
    pub fn authorization(&self, endpoint: &str, sub: &str, now: DateTime<Utc>) -> Result<String> {
        let claims = serde_json::json!({
            "aud": origin_of(endpoint)?,
            "exp": (now + VALID_FOR).timestamp(),
            "sub": sub,
        });
        let signing_input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let signature: Signature = self.key.sign(signing_input.as_bytes());
        Ok(format!(
            "vapid t={signing_input}.{}, k={}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes()),
            self.public_key()
        ))
    }
}

/// `https://host[:port]` of a URL: the audience a push service checks
/// a token against.
pub fn origin_of(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url).map_err(|e| Error::Invalid {
        kind: "push endpoint",
        reason: e.to_string(),
    })?;
    match parsed.origin() {
        origin @ url::Origin::Tuple(..) => Ok(origin.ascii_serialization()),
        url::Origin::Opaque(_) => Err(Error::Invalid {
            kind: "push endpoint",
            reason: format!("{url} has no origin"),
        }),
    }
}

/// Base64url as browsers write it: no padding, though padding is
/// forgiven.
pub(crate) fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim().trim_end_matches('=')).ok()
}

fn invalid(reason: &str) -> Error {
    Error::Invalid {
        kind: "VAPID key",
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{VerifyingKey, signature::Verifier};
    use serde_json::Value;

    fn parts(header: &str) -> (String, String) {
        let rest = header.strip_prefix("vapid t=").expect("vapid scheme");
        let (token, key) = rest.split_once(", k=").expect("t and k");
        (token.to_string(), key.to_string())
    }

    #[test]
    fn the_token_is_signed_by_the_key_it_names() {
        let key = VapidKey::generate();
        let header = key
            .authorization(
                "https://web.push.apple.com/abc",
                "https://niles.example",
                Utc::now(),
            )
            .unwrap();
        let (token, k) = parts(&header);
        assert_eq!(k, key.public_key());

        let (input, signature) = token.rsplit_once('.').unwrap();
        let verifying =
            VerifyingKey::from_sec1_bytes(&b64url_decode(&k).unwrap()).expect("public key");
        let signature = Signature::from_slice(&b64url_decode(signature).unwrap()).unwrap();
        verifying
            .verify(input.as_bytes(), &signature)
            .expect("verifies");
    }

    #[test]
    fn the_claims_name_the_push_service_a_contact_and_an_expiry_within_a_day() {
        let key = VapidKey::generate();
        let now = Utc::now();
        let header = key
            .authorization(
                "https://fcm.googleapis.com:443/fcm/send/xyz",
                "https://niles.example",
                now,
            )
            .unwrap();
        let (token, _) = parts(&header);
        let mut pieces = token.split('.');
        let head: Value =
            serde_json::from_slice(&b64url_decode(pieces.next().unwrap()).unwrap()).unwrap();
        let claims: Value =
            serde_json::from_slice(&b64url_decode(pieces.next().unwrap()).unwrap()).unwrap();
        assert_eq!(head["alg"], "ES256");
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["sub"], "https://niles.example");
        let exp = claims["exp"].as_i64().unwrap();
        assert!(exp > now.timestamp() && exp <= (now + Duration::hours(24)).timestamp());
    }

    #[test]
    fn a_kept_key_comes_back_the_same() {
        let key = VapidKey::generate();
        let again = VapidKey::from_base64url(&key.to_base64url()).unwrap();
        assert_eq!(again.public_key(), key.public_key());
    }

    #[test]
    fn the_public_key_is_an_uncompressed_point() {
        let raw = b64url_decode(&VapidKey::generate().public_key()).unwrap();
        assert_eq!(raw.len(), 65);
        assert_eq!(raw[0], 4);
    }

    #[test]
    fn a_damaged_key_is_refused() {
        assert!(VapidKey::from_base64url("not a key!").is_err());
        assert!(VapidKey::from_base64url("AAAA").is_err());
    }

    #[test]
    fn debug_never_shows_the_private_key() {
        let key = VapidKey::generate();
        let shown = format!("{key:?}");
        assert!(!shown.contains(&key.to_base64url()), "{shown}");
    }

    #[test]
    fn the_origin_keeps_a_port_that_is_not_the_default() {
        assert_eq!(
            origin_of("https://push.example:8443/x?y").unwrap(),
            "https://push.example:8443"
        );
        assert_eq!(
            origin_of("https://updates.push.services.mozilla.com/wpush/v2/abc").unwrap(),
            "https://updates.push.services.mozilla.com"
        );
        assert!(origin_of("not a url").is_err());
    }

    #[test]
    fn padding_is_forgiven() {
        assert_eq!(b64url_decode("AQ=="), Some(vec![1]));
        assert_eq!(b64url_decode("AQ"), Some(vec![1]));
    }
}
