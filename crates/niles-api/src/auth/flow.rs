//! What stands in for a signature on the way back from GitHub.
//!
//! GitHub's OAuth app flow has no discovery document, no ID token and
//! no PKCE. The callback arrives as a plain `GET` carrying a `code`,
//! and there is nothing on it to verify. So two things together
//! authenticate it, and both must hold:
//!
//! 1. a **`state`** we minted, which is single-use and expires — so a
//!    callback cannot be replayed, or forged by somebody who never
//!    started a sign-in; and
//! 2. a **cookie** written when the flow began, binding it to that one
//!    browser — so a `state` observed in a log or a referrer is no use
//!    anywhere else.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Long enough to sign in to GitHub, short enough that an abandoned
/// attempt is not still usable when you get back from lunch.
const TTL_SECS: u64 = 10 * 60;

pub const BINDING_COOKIE: &str = "niles_sso";

/// An attempt in progress.
pub struct Pending {
    created_at: u64,
    /// Where to go afterwards. Same-origin, already checked.
    pub next: String,
    /// SHA-256 of the binding cookie's value. Hashed rather than kept,
    /// so this map is not a list of usable cookies.
    binding: [u8; 32],
}

#[derive(Default)]
pub struct Attempts {
    pending: Mutex<HashMap<String, Pending>>,
}

impl Attempts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a `state` and the cookie value that binds it to this browser.
    pub fn begin(&self, next: String) -> (String, String) {
        let state = random_token();
        let binding = random_token();
        let now = now();
        let mut guard = self.lock();
        // Expired attempts are dropped here rather than on a timer: the
        // map only grows when somebody is signing in, so the moment
        // somebody does is the moment worth tidying.
        guard.retain(|_, p| now.saturating_sub(p.created_at) < TTL_SECS);
        guard.insert(
            state.clone(),
            Pending {
                created_at: now,
                next,
                binding: hash(&binding),
            },
        );
        (state, binding)
    }

    /// Spend a `state`, if it is one of ours, still fresh, and presented
    /// by the browser that started it.
    ///
    /// Removed whether or not the binding matches: a `state` somebody
    /// else is holding has already leaked, and leaving it live would let
    /// them keep trying.
    pub fn finish(&self, state: &str, binding: Option<&str>) -> Option<Pending> {
        let pending = self.lock().remove(state)?;
        if now().saturating_sub(pending.created_at) >= TTL_SECS {
            return None;
        }
        let given = hash(binding?);
        (given == pending.binding).then_some(pending)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Where to send the browser once it is signed in.
///
/// **A same-origin path, or nothing.** An absolute URL here would make
/// the sign-in page a phishing hop: a link that authenticates against
/// the real Niles and then lands somewhere else entirely, with the
/// person having watched a genuine GitHub consent screen on the way.
pub fn safe_next(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or("/").trim();

    // Refusing only a leading `//` is not enough, because a browser's
    // parser is more forgiving than a prefix check:
    //
    //   * a backslash is equivalent to a forward slash in the relative
    //     states of a special scheme, so `/\evil.example` parses as
    //     `//evil.example`;
    //   * tab, CR and LF are stripped *before* parsing, so
    //     `/<TAB>/evil.example` collapses to the same thing.
    //
    // Both satisfy "starts with one slash, not two". Reject the
    // characters that make them possible instead of out-guessing the
    // parser.
    if candidate
        .bytes()
        .any(|b| b == b'\\' || b < 0x21 || b == 0x7f)
    {
        return "/".to_string();
    }
    if candidate.starts_with('/') && !candidate.starts_with("//") {
        candidate.to_string()
    } else {
        "/".to_string()
    }
}

/// The binding cookie, scoped as narrowly as it can be: it is only ever
/// read by the callback.
pub fn binding_cookie(value: &str, secure: bool) -> String {
    format!(
        "{BINDING_COOKIE}={value}; Path=/auth; HttpOnly; SameSite=Lax; Max-Age={TTL_SECS}{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_binding_cookie(secure: bool) -> String {
    format!(
        "{BINDING_COOKIE}=; Path=/auth; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

fn random_token() -> String {
    let bytes: [u8; 32] = std::array::from_fn(|_| rand::random());
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_is_spent_once() {
        // Otherwise a callback in a browser history, a log or a referrer
        // header is a reusable key.
        let attempts = Attempts::new();
        let (state, binding) = attempts.begin("/".into());
        assert!(attempts.finish(&state, Some(&binding)).is_some());
        assert!(attempts.finish(&state, Some(&binding)).is_none());
    }

    #[test]
    fn a_state_we_never_minted_is_refused() {
        let attempts = Attempts::new();
        assert!(attempts.finish("made-up", Some("anything")).is_none());
    }

    #[test]
    fn another_browser_cannot_spend_it() {
        // This is what makes a leaked `state` useless: it only works
        // where the sign-in began.
        let attempts = Attempts::new();
        let (state, _binding) = attempts.begin("/".into());
        assert!(
            attempts
                .finish(&state, Some("somebody-elses-cookie"))
                .is_none()
        );
    }

    #[test]
    fn a_callback_with_no_binding_cookie_is_refused() {
        let attempts = Attempts::new();
        let (state, _) = attempts.begin("/".into());
        assert!(attempts.finish(&state, None).is_none());
    }

    #[test]
    fn a_state_someone_else_tried_is_not_left_live() {
        // Once a wrong binding has been presented, the state has leaked;
        // the right browser coming back later must not still work.
        let attempts = Attempts::new();
        let (state, binding) = attempts.begin("/".into());
        assert!(attempts.finish(&state, Some("wrong")).is_none());
        assert!(attempts.finish(&state, Some(&binding)).is_none());
    }

    #[test]
    fn two_attempts_do_not_collide() {
        let attempts = Attempts::new();
        let (first, first_binding) = attempts.begin("/one".into());
        let (second, second_binding) = attempts.begin("/two".into());
        assert_ne!(first, second);
        assert_eq!(
            attempts
                .finish(&second, Some(&second_binding))
                .unwrap()
                .next,
            "/two"
        );
        assert_eq!(
            attempts.finish(&first, Some(&first_binding)).unwrap().next,
            "/one"
        );
    }

    #[test]
    fn the_destination_is_a_path_here_or_nowhere() {
        assert_eq!(safe_next(Some("/rooms")), "/rooms");
        assert_eq!(safe_next(None), "/");
        for hostile in [
            "https://evil.example",
            "//evil.example",
            r"/\evil.example",
            "/\t/evil.example",
            "/\n//evil.example",
            "javascript:alert(1)",
        ] {
            assert_eq!(safe_next(Some(hostile)), "/", "{hostile:?}");
        }
    }

    #[test]
    fn the_binding_cookie_is_not_readable_by_script_and_is_scoped_narrowly() {
        let header = binding_cookie("value", true);
        assert!(header.contains("HttpOnly"), "{header}");
        assert!(header.contains("Path=/auth"), "{header}");
        assert!(header.contains("Secure"), "{header}");
    }

    #[test]
    fn tokens_are_not_guessable_by_being_the_same() {
        let attempts = Attempts::new();
        let (a, _) = attempts.begin("/".into());
        let (b, _) = attempts.begin("/".into());
        assert_ne!(a, b);
        assert_eq!(a.len(), 64, "32 bytes, hex");
    }
}
