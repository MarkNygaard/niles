//! Signing in to ChatGPT, for the assistant that runs on a household's
//! ChatGPT subscription.
//!
//! The OAuth client the Codex CLI uses, with PKCE. There is no callback
//! server: the redirect points at `localhost:1455`, where the Codex CLI
//! would be listening and nothing is. The person signs in in their own
//! browser, the redirect fails to load, and they paste its address back
//! into Niles — which works against a server in a cluster and from a
//! phone. The device-code flow would be tidier, and OpenAI puts it
//! behind a workspace admin setting that a household cannot change.
//!
//! Only the HTTP and the token parsing. Where the tokens are kept, and
//! who may refresh them, belongs to the callers.

use crate::error::{Error, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// Where the Codex CLI would be listening. OpenAI accepts no other
/// redirect for this client.
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";

/// The access token's claim that carries the ChatGPT account id. The
/// token response does not say it anywhere else.
const AUTH_CLAIM: &str = "https://api.openai.com/auth";
const PROFILE_CLAIM: &str = "https://api.openai.com/profile";

/// A sign-in started: the page to send the person to, and what the
/// caller keeps until they come back. The verifier never leaves the
/// server.
pub struct Login {
    pub authorize_url: String,
    pub state: String,
    pub verifier: String,
}

pub fn start_login() -> Login {
    let bytes: [u8; 96] = std::array::from_fn(|_| rand::random());
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let state = format!("{:032x}", rand::random::<u128>());
    Login {
        authorize_url: authorize_url(&state, &challenge(&verifier)),
        state,
        verifier,
    }
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn authorize_url(state: &str, challenge: &str) -> String {
    reqwest::Url::parse_with_params(
        AUTHORIZE_URL,
        [
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPE),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            // What the Codex CLI sends. Without them the consent page is
            // the generic one and the token carries no ChatGPT account.
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "codex_cli_rs"),
        ],
    )
    .expect("the authorize URL is a constant that parses")
    .to_string()
}

/// The query of whatever was pasted: a whole address, or just its query.
fn pasted_params(pasted: &str) -> Vec<(String, String)> {
    let text = pasted.trim();
    let query = match text.split_once('?') {
        Some((_, query)) => query,
        None if text.contains('=') => text,
        None => return Vec::new(),
    };
    let query = query.split('#').next().unwrap_or_default();
    reqwest::Url::parse(&format!("http://pasted/?{query}"))
        .map(|url| url.query_pairs().into_owned().collect())
        .unwrap_or_default()
}

fn param(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(k, v)| k == key && !v.is_empty())
        .map(|(_, v)| v.clone())
}

/// The `state` in a pasted address, when there is one: which sign-in it
/// finishes. A bare code carries none.
pub fn state_in(pasted: &str) -> Option<String> {
    param(&pasted_params(pasted), "state")
}

/// The authorization code out of what was pasted.
///
/// Tolerant, because it comes out of an address bar: the whole address,
/// its query alone, or the bare code for a browser that makes the
/// address awkward to copy. A state that is not this sign-in's is
/// refused; a bare code has none to compare.
pub fn code_from_paste(pasted: &str, expected_state: &str) -> Result<String> {
    let refused = |reason: &str| Error::SignIn {
        reason: reason.to_string(),
    };
    let params = pasted_params(pasted);
    if let Some(code) = param(&params, "code") {
        if param(&params, "state").is_some_and(|state| state != expected_state) {
            return Err(refused(
                "That address is from a different sign-in. Press Sign in with ChatGPT again and paste the address it leads to.",
            ));
        }
        return Ok(code);
    }
    if let Some(error) = param(&params, "error") {
        let detail = param(&params, "error_description").unwrap_or(error);
        return Err(Error::SignIn {
            reason: format!("OpenAI did not sign you in: {detail}"),
        });
    }
    let bare = pasted.trim();
    if bare.is_empty() || !params.is_empty() || bare.contains(char::is_whitespace) {
        return Err(refused(
            "There is no sign-in code in that. Paste the whole address of the page that failed to load.",
        ));
    }
    Ok(bare.to_string())
}

/// What OpenAI hands back for a code or a refresh.
#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    /// Always on a sign-in. On a refresh only when OpenAI rotated it, and
    /// the old one is then spent.
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    /// Seconds. An hour when OpenAI does not say.
    pub expires_in: u64,
}

/// Swap the code for tokens. Form-encoded, as OAuth has it.
pub async fn exchange_code(
    http: &reqwest::Client,
    token_url: &str,
    code: &str,
    verifier: &str,
) -> Result<Tokens> {
    let response = http
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", REDIRECT_URI),
        ])
        .send()
        .await?;
    tokens_from(response).await
}

/// A new access token for a refresh token. JSON, unlike the exchange:
/// it is what the Codex CLI sends, and what OpenAI is known to accept.
pub async fn refresh(
    http: &reqwest::Client,
    token_url: &str,
    refresh_token: &str,
) -> Result<Tokens> {
    let response = http
        .post(token_url)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send()
        .await?;
    tokens_from(response).await
}

async fn tokens_from(response: reqwest::Response) -> Result<Tokens> {
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    let text = |key: &str| {
        body.get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if !status.is_success() {
        let detail = text("error_description")
            .or_else(|| body["error"]["message"].as_str().map(str::to_string))
            .or_else(|| text("error"))
            .unwrap_or_else(|| format!("status {}", status.as_u16()));
        return Err(Error::SignIn {
            reason: format!("OpenAI refused it: {detail}"),
        });
    }
    let access_token = text("access_token").ok_or_else(|| Error::SignIn {
        reason: "OpenAI answered without an access token".into(),
    })?;
    Ok(Tokens {
        access_token,
        refresh_token: text("refresh_token"),
        id_token: text("id_token"),
        expires_in: body["expires_in"].as_u64().unwrap_or(3600),
    })
}

/// The claims of a JWT, unverified: these are read to label and route a
/// token that came straight from OpenAI, not to trust anybody's.
fn claims(jwt: &str) -> Option<serde_json::Value> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The ChatGPT account an access token is for. Every request to the
/// subscription names it.
pub fn account_id(access_token: &str) -> Option<String> {
    claims(access_token)?[AUTH_CLAIM]["chatgpt_account_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Who signed in: the ID token says it plainly, the access token in its
/// profile claim.
pub fn email(jwt: &str) -> Option<String> {
    let claims = claims(jwt)?;
    claims["email"]
        .as_str()
        .or_else(|| claims[PROFILE_CLAIM]["email"].as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn jwt(claims: serde_json::Value) -> String {
        format!(
            "e30.{}.sig",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        )
    }

    fn query_of(url: &str) -> Vec<(String, String)> {
        reqwest::Url::parse(url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    #[test]
    fn the_sign_in_page_is_asked_for_what_the_codex_cli_asks_for() {
        let login = start_login();
        assert!(login.authorize_url.starts_with(AUTHORIZE_URL));
        let query = query_of(&login.authorize_url);
        let get = |key: &str| param(&query, key).unwrap_or_else(|| panic!("{key} missing"));
        assert_eq!(get("client_id"), CLIENT_ID);
        assert_eq!(get("redirect_uri"), REDIRECT_URI);
        assert_eq!(get("scope"), "openid profile email offline_access");
        assert_eq!(get("response_type"), "code");
        assert_eq!(get("code_challenge_method"), "S256");
        assert_eq!(get("code_challenge"), challenge(&login.verifier));
        assert_eq!(get("state"), login.state);
        assert_eq!(get("id_token_add_organizations"), "true");
        assert_eq!(get("codex_cli_simplified_flow"), "true");
        assert_eq!(get("originator"), "codex_cli_rs");
    }

    #[test]
    fn the_verifier_is_96_random_bytes_and_stays_out_of_the_url() {
        let login = start_login();
        assert_eq!(URL_SAFE_NO_PAD.decode(&login.verifier).unwrap().len(), 96);
        assert!(!login.authorize_url.contains(&login.verifier));
        assert_ne!(start_login().state, login.state);
    }

    #[test]
    fn the_challenge_is_the_s256_of_the_verifier() {
        // Worked out independently, with Python's hashlib and base64.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mJ0kzjFSiBMvJjFr4ZVz9oOmkdA4ZM"),
            "SAxkrjpG1ulhSEXtoKih9woEVnyZ_S6A9aAeYXQdUps"
        );
    }

    #[test]
    fn a_pasted_address_gives_its_code() {
        let pasted = "http://localhost:1455/auth/callback?code=ac_123&scope=openid&state=abc";
        assert_eq!(code_from_paste(pasted, "abc").unwrap(), "ac_123");
        assert_eq!(state_in(pasted).as_deref(), Some("abc"));
    }

    #[test]
    fn its_query_alone_will_do() {
        assert_eq!(
            code_from_paste("  code=ac_9&state=abc#frag ", "abc").unwrap(),
            "ac_9"
        );
    }

    #[test]
    fn a_bare_code_is_taken_as_it_is() {
        assert_eq!(
            code_from_paste(" ac_bare_code\n", "abc").unwrap(),
            "ac_bare_code"
        );
        assert_eq!(state_in("ac_bare_code"), None);
    }

    #[test]
    fn another_sign_ins_address_is_refused() {
        let err = code_from_paste(
            "http://localhost:1455/auth/callback?code=x&state=zzz",
            "abc",
        )
        .unwrap_err();
        assert!(err.to_string().contains("different sign-in"), "{err}");
    }

    #[test]
    fn a_refusal_from_openai_says_so() {
        let err = code_from_paste(
            "http://localhost:1455/auth/callback?error=access_denied&error_description=Nope&state=abc",
            "abc",
        )
        .unwrap_err();
        assert!(err.to_string().contains("Nope"), "{err}");
    }

    #[test]
    fn nothing_useful_pasted_is_an_error() {
        assert!(code_from_paste("", "abc").is_err());
        assert!(code_from_paste("http://localhost:1455/auth/callback?state=abc", "abc").is_err());
        assert!(code_from_paste("some words here", "abc").is_err());
    }

    #[test]
    fn the_account_is_read_from_the_access_token() {
        let token = jwt(serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct-1" },
            "https://api.openai.com/profile": { "email": "bro@example.com" },
        }));
        assert_eq!(account_id(&token).as_deref(), Some("acct-1"));
        assert_eq!(email(&token).as_deref(), Some("bro@example.com"));
    }

    #[test]
    fn the_id_tokens_email_is_its_own_claim() {
        let token = jwt(serde_json::json!({ "email": "bro@example.com" }));
        assert_eq!(email(&token).as_deref(), Some("bro@example.com"));
        assert_eq!(account_id(&token), None);
    }

    #[test]
    fn something_that_is_not_a_jwt_has_no_account() {
        assert_eq!(account_id("not-a-token"), None);
        assert_eq!(account_id("a.!!!.c"), None);
        assert_eq!(email(""), None);
    }

    #[tokio::test]
    async fn the_code_is_exchanged_form_encoded() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=ac_1"))
            .and(body_string_contains("code_verifier=v3r"))
            .and(body_string_contains(
                "redirect_uri=http%3A%2F%2Flocalhost%3A1455",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at", "refresh_token": "rt", "id_token": "it", "expires_in": 864000,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let tokens = exchange_code(&reqwest::Client::new(), &server.uri(), "ac_1", "v3r")
            .await
            .unwrap();
        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        assert_eq!(tokens.id_token.as_deref(), Some("it"));
        assert_eq!(tokens.expires_in, 864000);
    }

    #[tokio::test]
    async fn a_refresh_is_sent_as_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_json(serde_json::json!({
                "grant_type": "refresh_token",
                "client_id": CLIENT_ID,
                "refresh_token": "rt-old",
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "access_token": "at-new" })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let tokens = refresh(&reqwest::Client::new(), &server.uri(), "rt-old")
            .await
            .unwrap();
        assert_eq!(tokens.access_token, "at-new");
        assert_eq!(tokens.refresh_token, None);
        assert_eq!(tokens.expires_in, 3600);
    }

    #[tokio::test]
    async fn a_spent_refresh_token_says_why() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": { "message": "Your refresh token has already been used", "code": "refresh_token_reused" },
            })))
            .mount(&server)
            .await;
        let err = refresh(&reqwest::Client::new(), &server.uri(), "rt")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already been used"), "{err}");
    }
}
