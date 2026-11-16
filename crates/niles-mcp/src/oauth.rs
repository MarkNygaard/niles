//! Signing in to a server the way MCP's authorization asks: OAuth 2.1
//! with PKCE, for a person's own server that wants a browser sign-in
//! rather than a key.
//!
//! Found from the server: its protected-resource metadata (RFC 9728),
//! named by its `401` or at the well-known address, says which
//! authorization server signs it in; that server's metadata (RFC 8414,
//! or OpenID's) gives the endpoints. Niles registers itself as a client
//! (RFC 7591) where the server offers that, sends the person's browser
//! to sign in with a PKCE challenge, and exchanges the code it comes
//! back with for tokens, which it refreshes before they run out.
//!
//! Every address on the way, the server's own included, is held to the
//! public internet when the client is (`Reach::PublicInternet`).

use crate::error::{Error, Result};
use crate::public::{Reach, check};
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;
use url::Url;

/// How long one sign-in request may take.
const TIMEOUT: Duration = Duration::from_secs(20);

/// What signs a server in: the authorization server's endpoints, the
/// resource the tokens are for, and the scope asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthServer {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    /// The MCP server's own address, sent as RFC 8707's `resource`.
    pub resource: String,
    /// Space-separated, as the server lists them; empty asks for none.
    pub scope: String,
}

/// Niles as the authorization server knows it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for Registration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registration")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "…"))
            .finish()
    }
}

/// What a sign-in gives: never printed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl Tokens {
    /// Whether the access token runs out within `margin` of `now`.
    pub fn expiring(&self, now: DateTime<Utc>, margin: chrono::Duration) -> bool {
        self.expires_at.is_some_and(|at| at - margin <= now)
    }
}

/// A PKCE pair: the verifier kept by Niles, the challenge sent along.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Pkce {
        let mut raw = [0u8; 32];
        rand::rng().fill_bytes(&mut raw);
        let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        Pkce {
            verifier,
            challenge,
        }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

/// A random value for `state`, which ties the browser's return to the
/// sign-in that sent it.
pub fn state() -> String {
    let mut raw = [0u8; 24];
    rand::rng().fill_bytes(&mut raw);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

fn sign_in(reason: impl Into<String>) -> Error {
    Error::SignIn(reason.into())
}

/// The sign-in side of one server.
pub struct SignInClient {
    http: reqwest::Client,
    reach: Reach,
}

impl SignInClient {
    pub fn new(reach: Reach) -> Result<SignInClient> {
        Ok(SignInClient {
            http: crate::public::client(reach, TIMEOUT)?,
            reach,
        })
    }

    async fn get_json(&self, url: &str) -> Result<Option<Value>> {
        check(url, self.reach)?;
        let response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        if !response.status().is_success() {
            return Ok(None);
        }
        Ok(response.json::<Value>().await.ok())
    }

    /// How `server` signs in. `hint` is the metadata address its `401`
    /// named, if it did.
    pub async fn discover(&self, server: &str, hint: Option<&str>) -> Result<AuthServer> {
        let server_url =
            Url::parse(server).map_err(|_| sign_in("the server's address doesn't read"))?;
        let mut candidates: Vec<String> = hint.map(str::to_string).into_iter().collect();
        candidates.extend(well_known(&server_url, "oauth-protected-resource"));
        let mut resource = None;
        for candidate in &candidates {
            if let Some(found) = self.get_json(candidate).await? {
                resource = Some(found);
                break;
            }
        }
        // An older server has no resource metadata: it is its own
        // authorization server.
        let (issuer, resource_scopes) = match &resource {
            Some(found) => (
                found
                    .get("authorization_servers")
                    .and_then(Value::as_array)
                    .and_then(|servers| servers.first())
                    .and_then(Value::as_str)
                    .ok_or_else(|| sign_in("the server names no authorization server"))?
                    .to_string(),
                strings(found.get("scopes_supported")),
            ),
            None => (origin(&server_url), Vec::new()),
        };
        let issuer_url = Url::parse(&issuer)
            .map_err(|_| sign_in("the authorization server's address doesn't read"))?;
        let mut metadata = None;
        for candidate in well_known(&issuer_url, "oauth-authorization-server")
            .into_iter()
            .chain(well_known(&issuer_url, "openid-configuration"))
        {
            if let Some(found) = self.get_json(&candidate).await? {
                metadata = Some(found);
                break;
            }
        }
        let metadata =
            metadata.ok_or_else(|| sign_in("the authorization server doesn't describe itself"))?;
        let endpoint = |name: &str| -> Result<String> {
            let value = metadata
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| sign_in(format!("the authorization server has no {name}")))?;
            check(value, self.reach)?;
            Ok(value.to_string())
        };
        let methods = strings(metadata.get("code_challenge_methods_supported"));
        if !methods.is_empty() && !methods.iter().any(|m| m == "S256") {
            return Err(sign_in("the authorization server doesn't take PKCE's S256"));
        }
        let registration_endpoint = match metadata
            .get("registration_endpoint")
            .and_then(Value::as_str)
        {
            Some(value) => {
                check(value, self.reach)?;
                Some(value.to_string())
            }
            None => None,
        };
        let scopes = if resource_scopes.is_empty() {
            strings(metadata.get("scopes_supported"))
        } else {
            resource_scopes
        };
        Ok(AuthServer {
            authorization_endpoint: endpoint("authorization_endpoint")?,
            token_endpoint: endpoint("token_endpoint")?,
            registration_endpoint,
            resource: server.to_string(),
            scope: scopes.join(" "),
        })
    }

    /// Registers Niles as a public client that comes back to
    /// `redirect_uri`.
    pub async fn register(&self, auth: &AuthServer, redirect_uri: &str) -> Result<Registration> {
        let Some(endpoint) = &auth.registration_endpoint else {
            return Err(sign_in(
                "the server doesn't let Niles register itself; add it with a key instead",
            ));
        };
        check(endpoint, self.reach)?;
        let response = self
            .http
            .post(endpoint)
            .json(&serde_json::json!({
                "client_name": "Niles",
                "redirect_uris": [redirect_uri],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            }))
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        if !response.status().is_success() {
            return Err(sign_in(format!(
                "registering Niles was refused ({})",
                response.status().as_u16()
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| sign_in("the registration's answer doesn't read"))?;
        Ok(Registration {
            client_id: body
                .get("client_id")
                .and_then(Value::as_str)
                .ok_or_else(|| sign_in("the registration gave no client id"))?
                .to_string(),
            client_secret: body
                .get("client_secret")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    async fn token(
        &self,
        auth: &AuthServer,
        client: &Registration,
        form: Vec<(&str, String)>,
    ) -> Result<Tokens> {
        check(&auth.token_endpoint, self.reach)?;
        let mut form = form;
        form.push(("client_id", client.client_id.clone()));
        form.push(("resource", auth.resource.clone()));
        if let Some(secret) = &client.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let response = self
            .http
            .post(&auth.token_endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&form)
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            // The error code only: a description may quote what was sent.
            let code = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("refused");
            return Err(sign_in(format!("the sign-in was refused ({code})")));
        }
        let access = body
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| sign_in("the sign-in gave no access token"))?
            .to_string();
        Ok(Tokens {
            access,
            refresh: body
                .get("refresh_token")
                .and_then(Value::as_str)
                .map(str::to_string),
            expires_at: body
                .get("expires_in")
                .and_then(Value::as_i64)
                .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds)),
        })
    }

    /// The tokens for the code the browser came back with.
    pub async fn exchange(
        &self,
        auth: &AuthServer,
        client: &Registration,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<Tokens> {
        self.token(
            auth,
            client,
            vec![
                ("grant_type", "authorization_code".into()),
                ("code", code.to_string()),
                ("code_verifier", verifier.to_string()),
                ("redirect_uri", redirect_uri.to_string()),
            ],
        )
        .await
    }

    /// New tokens for `refresh`. A server that hands out no new refresh
    /// token keeps the old one working, so it is kept.
    pub async fn refresh(
        &self,
        auth: &AuthServer,
        client: &Registration,
        refresh: &str,
    ) -> Result<Tokens> {
        let mut tokens = self
            .token(
                auth,
                client,
                vec![
                    ("grant_type", "refresh_token".into()),
                    ("refresh_token", refresh.to_string()),
                ],
            )
            .await?;
        if tokens.refresh.is_none() {
            tokens.refresh = Some(refresh.to_string());
        }
        Ok(tokens)
    }
}

/// Where the person's browser goes to sign in.
pub fn authorize_url(
    auth: &AuthServer,
    client: &Registration,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<String> {
    let mut url = Url::parse(&auth.authorization_endpoint)
        .map_err(|_| sign_in("the authorization address doesn't read"))?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("resource", &auth.resource);
        if !auth.scope.is_empty() {
            query.append_pair("scope", &auth.scope);
        }
    }
    Ok(url.to_string())
}

fn origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

/// RFC 8414's well-known addresses for `url`: with its path inserted
/// after the well-known part, then at the root.
fn well_known(url: &Url, name: &str) -> Vec<String> {
    let origin = origin(url);
    let path = url.path().trim_end_matches('/');
    let mut out = Vec::new();
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{name}{path}"));
    }
    out.push(format!("{origin}/.well-known/{name}"));
    out
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn the_challenge_is_the_verifiers_sha256() {
        // RFC 7636's own example.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let pair = Pkce::new();
        assert!(pair.verifier.len() >= 43);
        assert_ne!(Pkce::new().verifier, pair.verifier);
    }

    #[test]
    fn the_well_known_address_keeps_the_path() {
        let url = Url::parse("https://mcp.example.com/v1/mcp").unwrap();
        assert_eq!(
            well_known(&url, "oauth-protected-resource"),
            [
                "https://mcp.example.com/.well-known/oauth-protected-resource/v1/mcp",
                "https://mcp.example.com/.well-known/oauth-protected-resource",
            ]
        );
    }

    #[tokio::test]
    async fn a_server_is_signed_in_from_its_metadata_to_its_tokens() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-protected-resource/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resource": format!("{base}/mcp"),
                "authorization_servers": [format!("{base}/auth")],
                "scopes_supported": ["read", "write"],
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-authorization-server/auth"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": format!("{base}/auth"),
                "authorization_endpoint": format!("{base}/auth/authorize"),
                "token_endpoint": format!("{base}/auth/token"),
                "registration_endpoint": format!("{base}/auth/register"),
                "code_challenge_methods_supported": ["S256"],
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth/register"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "client_id": "niles-1" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code_verifier="))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "a1", "refresh_token": "r1", "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let signing = SignInClient::new(Reach::Anywhere).unwrap();
        let auth = signing
            .discover(&format!("{base}/mcp"), None)
            .await
            .expect("found");
        assert_eq!(auth.token_endpoint, format!("{base}/auth/token"));
        assert_eq!(auth.scope, "read write");
        let client = signing
            .register(&auth, "https://niles.example/me/tool-servers/callback")
            .await
            .expect("registered");
        assert_eq!(client.client_id, "niles-1");

        let pkce = Pkce::new();
        let link = authorize_url(
            &auth,
            &client,
            "https://niles.example/cb",
            "s7",
            &pkce.challenge,
        )
        .unwrap();
        for part in [
            "code_challenge_method=S256",
            "state=s7",
            "client_id=niles-1",
            "scope=read+write",
        ] {
            assert!(link.contains(part), "{link}");
        }
        let tokens = signing
            .exchange(
                &auth,
                &client,
                "c0de",
                &pkce.verifier,
                "https://niles.example/cb",
            )
            .await
            .expect("tokens");
        assert_eq!(tokens.access, "a1");
        assert_eq!(tokens.refresh.as_deref(), Some("r1"));
        assert!(!format!("{tokens:?}").contains("a1"));
        assert!(!tokens.expiring(Utc::now(), chrono::Duration::minutes(1)));
    }

    #[tokio::test]
    async fn a_refresh_without_a_new_refresh_token_keeps_the_old_one() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "access_token": "a2" })))
            .mount(&server)
            .await;
        let auth = AuthServer {
            authorization_endpoint: format!("{}/authorize", server.uri()),
            token_endpoint: format!("{}/token", server.uri()),
            registration_endpoint: None,
            resource: "https://mcp.example.com/mcp".into(),
            scope: String::new(),
        };
        let client = Registration {
            client_id: "niles-1".into(),
            client_secret: None,
        };
        let tokens = SignInClient::new(Reach::Anywhere)
            .unwrap()
            .refresh(&auth, &client, "r1")
            .await
            .expect("refreshed");
        assert_eq!(tokens.access, "a2");
        assert_eq!(tokens.refresh.as_deref(), Some("r1"));
    }

    #[tokio::test]
    async fn a_person_s_sign_in_is_held_to_the_public_internet() {
        let signing = SignInClient::new(Reach::PublicInternet).unwrap();
        let refused = signing
            .discover("https://mcp.example.com/mcp", Some("https://10.0.0.1/meta"))
            .await;
        assert!(matches!(refused, Err(Error::NotPublic(_))), "{refused:?}");
    }
}
