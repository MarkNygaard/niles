//! The cookie that says who you are.
//!
//! It carries the **address**, signed, rather than an opaque id into a
//! session table. That is what makes removing somebody from the
//! allowlist actually remove them: the address is checked against the
//! list in force *now*, on every request, so a cookie signed months ago
//! has no more standing than the list gives it today.
//!
//! The cost of that choice is that a cookie cannot be revoked
//! individually — only by taking its owner off the list, or by changing
//! the signing key, which signs everybody out. For a household of two
//! that is the right trade; for anything larger it would not be.

use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

/// How long a cookie stands before its holder has to sign in again.
///
/// Long, because this is a light switch in somebody's home and being
/// logged out on the way past a dark hallway is its own kind of
/// failure. Revocation does not depend on it: taking somebody off the
/// allowlist is immediate.
const MAX_AGE_SECS: u64 = 400 * 24 * 60 * 60;

pub const COOKIE: &str = "niles_session";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub email: String,
    pub issued_at: u64,
    /// GitHub's numeric account id, which is what the avatar is
    /// addressed by. Optional because a cookie signed before this
    /// existed is still perfectly good — it just draws initials.
    pub github_id: Option<u64>,
}

impl Session {
    pub fn new(email: impl Into<String>, github_id: Option<u64>) -> Self {
        Self {
            email: email.into(),
            issued_at: now(),
            github_id,
        }
    }
}

/// `<base64url(email)>|<issued_at>|<github id>.<base64url(hmac)>`
///
/// Hand-rolled rather than a JWT: there is one issuer, one audience and
/// one algorithm, so a format that can negotiate those is a larger
/// surface than the problem. Notably there is no `alg` field to confuse.
pub fn sign(secret: &str, session: &Session) -> String {
    let payload = format!(
        "{}|{}|{}",
        encode(session.email.as_bytes()),
        session.issued_at,
        session
            .github_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
    );
    let signature = mac(secret, payload.as_bytes());
    format!("{payload}.{}", encode(&signature))
}

/// The session a cookie proves, or `None`.
///
/// `None` covers every way it can fail — tampered, truncated, signed
/// with a different key, or simply old — because none of them is worth
/// telling the holder apart from the others.
pub fn verify(secret: &str, token: &str) -> Option<Session> {
    let (payload, signature) = token.rsplit_once('.')?;
    let expected = mac(secret, payload.as_bytes());
    let given = decode(signature)?;
    // Constant time: a byte-by-byte comparison that returns early leaks
    // how much of a forged signature was right.
    if !constant_time_eq(&expected, &given) {
        return None;
    }

    // Two fields or three: a cookie signed before the id was carried
    // still verifies, and simply has no avatar to draw.
    let mut parts = payload.split('|');
    let email = String::from_utf8(decode(parts.next()?)?).ok()?;
    let issued_at: u64 = parts.next()?.parse().ok()?;
    let github_id = parts.next().and_then(|id| id.parse().ok());
    if now().saturating_sub(issued_at) > MAX_AGE_SECS {
        return None;
    }
    Some(Session {
        email,
        issued_at,
        github_id,
    })
}

/// `Set-Cookie` for a fresh session.
///
/// `SameSite=Lax` is what makes every mutation here safe from another
/// site: they are all `POST`, `PATCH` or `DELETE`, and Lax withholds
/// the cookie from cross-site requests that are not top-level
/// navigations.
pub fn set_cookie(token: &str, secure: bool) -> String {
    format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={MAX_AGE_SECS}{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_cookie(secure: bool) -> String {
    format!(
        "{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

/// Pull one cookie out of a `Cookie:` header.
pub fn from_header(header: &str, name: &str) -> Option<String> {
    header
        .split(';')
        .map(str::trim)
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn mac(secret: &str, message: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts a key of any length");
    Mac::update(&mut mac, message);
    Mac::finalize(mac).into_bytes().to_vec()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── base64url, unpadded ──────────────────────────────────────────────────────
// Small enough to write; a dependency for 30 lines is a dependency to
// audit, update and explain.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        let indices = [n >> 18, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, index) in indices.iter().enumerate() {
            if i <= chunk.len() {
                out.push(ALPHABET[*index as usize] as char);
            }
        }
    }
    out
}

fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for chunk in text.as_bytes().chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let mut n = 0u32;
        for (i, byte) in chunk.iter().enumerate() {
            let value = ALPHABET.iter().position(|c| c == byte)? as u32;
            n |= value << (18 - 6 * i);
        }
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "a-signing-key-that-outlives-a-restart";

    #[test]
    fn a_cookie_we_signed_names_the_person() {
        let token = sign(SECRET, &Session::new("mark@example.com", Some(47065655)));
        let session = verify(SECRET, &token).unwrap();
        assert_eq!(session.email, "mark@example.com");
        assert_eq!(session.github_id, Some(47065655));
    }

    #[test]
    fn a_cookie_signed_with_another_key_is_nobody() {
        let token = sign(SECRET, &Session::new("mark@example.com", None));
        assert!(verify("a-different-key", &token).is_none());
    }

    #[test]
    fn changing_the_address_invalidates_it() {
        // The whole point of signing it: the address is the claim.
        let token = sign(SECRET, &Session::new("mark@example.com", None));
        let forged = token.replace(&encode(b"mark@example.com"), &encode(b"evil@example.com"));
        assert_ne!(forged, token, "the test must actually change it");
        assert!(verify(SECRET, &forged).is_none());
    }

    #[test]
    fn a_stale_cookie_is_nobody() {
        let old = Session {
            email: "mark@example.com".into(),
            issued_at: now() - MAX_AGE_SECS - 1,
            github_id: None,
        };
        assert!(verify(SECRET, &sign(SECRET, &old)).is_none());
    }

    #[test]
    fn rubbish_is_nobody_rather_than_a_panic() {
        for token in ["", ".", "no-dot", "a.b", "....", "%%%.%%%"] {
            assert!(verify(SECRET, token).is_none(), "{token:?}");
        }
    }

    #[test]
    fn base64url_round_trips_every_length() {
        // The padding cases are where a hand-rolled codec goes wrong.
        for n in 0..40 {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 7 + 3) as u8).collect();
            assert_eq!(decode(&encode(&bytes)).unwrap(), bytes, "length {n}");
        }
    }

    #[test]
    fn base64url_avoids_the_characters_a_cookie_cannot_carry() {
        let encoded = encode(&(0u8..=255).collect::<Vec<u8>>());
        assert!(
            !encoded.contains('+') && !encoded.contains('/') && !encoded.contains('='),
            "{encoded}"
        );
    }

    #[test]
    fn the_cookie_is_locked_down() {
        let header = set_cookie("token", true);
        assert!(header.contains("HttpOnly"), "{header}");
        assert!(header.contains("SameSite=Lax"), "{header}");
        assert!(header.contains("Secure"), "{header}");
        assert!(header.contains("Path=/"), "{header}");
    }

    #[test]
    fn a_plain_http_deployment_gets_a_cookie_a_browser_will_store() {
        // `Secure` over plain HTTP is a cookie the browser drops, which
        // presents as signing in doing nothing at all.
        assert!(!set_cookie("token", false).contains("Secure"));
    }

    #[test]
    fn the_session_is_read_out_of_a_crowded_cookie_header() {
        let header = format!("theme=dark; {COOKIE}=abc123; other=1");
        assert_eq!(from_header(&header, COOKIE).as_deref(), Some("abc123"));
        assert_eq!(from_header("nothing=here", COOKIE), None);
        assert_eq!(from_header(&format!("{COOKIE}="), COOKIE), None);
    }

    #[test]
    fn a_cookie_signed_before_the_id_existed_still_works() {
        // Adding a field must not sign the household out.
        let payload = format!("{}|{}", encode(b"mark@example.com"), now());
        let token = format!("{payload}.{}", encode(&mac(SECRET, payload.as_bytes())));
        let session = verify(SECRET, &token).expect("two fields is still a session");
        assert_eq!(session.email, "mark@example.com");
        assert_eq!(session.github_id, None, "no avatar, and that is fine");
    }
}
