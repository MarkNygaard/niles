//! Each person's own MCP servers: what they add under My profile, kept
//! for them alone. Postgres keeps them in the house (niles-db); memory
//! in tests.
//!
//! Every method is scoped by the person's key, as Vault's are, so
//! another person's id answers like one never kept. A server's key or
//! sign-in belongs to its row and leaves with it.

use crate::client::RemoteTool;
use crate::error::{Error, Result};
use crate::oauth::{AuthServer, Registration, Tokens};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The longest name a person may give a server.
pub const MAX_NAME: usize = 40;
/// The longest key accepted.
pub const MAX_KEY: usize = 4000;

/// What the model may do with one of a person's tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    On,
    /// Called, and held for the person's yes before it runs.
    Ask,
    Off,
}

impl Mode {
    /// A tool's setting before the person chooses one: what the server
    /// says only reads runs, anything else asks.
    pub fn default_for(tool: &RemoteTool) -> Mode {
        if tool.read_only { Mode::On } else { Mode::Ask }
    }
}

/// How a server is signed in to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignInKind {
    /// A key or token the person pasted.
    Key,
    /// The server's own sign-in page, in the person's browser.
    Browser,
}

/// A key or token as it was pasted. Never printed.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Key(String);

impl Key {
    pub fn new(value: impl Into<String>) -> Key {
        Key(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("…")
    }
}

/// A person's server, as My profile lists it: never its key or tokens.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PersonalServer {
    pub id: i64,
    /// What the model's names for its tools start with: `my-<slug>__`.
    pub slug: String,
    pub name: String,
    pub url: String,
    pub sign_in: SignInKind,
    /// The header a key goes in, when not `Authorization: Bearer`.
    pub header: Option<String>,
    /// Each tool's setting, by the server's name for it.
    pub modes: BTreeMap<String, Mode>,
    /// A key is kept, or the browser sign-in finished.
    pub signed_in: bool,
}

impl PersonalServer {
    /// What `tool` is set to: the person's choice, or its default.
    pub fn mode(&self, tool: &RemoteTool) -> Mode {
        self.modes
            .get(&tool.name)
            .copied()
            .unwrap_or_else(|| Mode::default_for(tool))
    }
}

/// A browser sign-in that finished: where it signs in, Niles as its
/// client, and the tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSignIn {
    pub auth: AuthServer,
    pub client: Registration,
    pub tokens: Tokens,
}

/// What a server is signed in with, opened: for Niles's connection,
/// never a response or the model.
#[derive(Debug, Clone, PartialEq)]
pub enum Credential {
    Key(Key),
    /// Boxed: a sign-in is much larger than a key.
    Browser(Box<BrowserSignIn>),
}

fn invalid(kind: &'static str, reason: &str) -> Error {
    Error::Invalid {
        kind,
        reason: reason.to_string(),
    }
}

/// A name a person gave, checked.
pub fn name(given: &str) -> Result<String> {
    let given = given.trim();
    if given.is_empty() {
        return Err(invalid("name", "it is empty"));
    }
    if given.chars().count() > MAX_NAME {
        return Err(invalid("name", "it is longer than 40 characters"));
    }
    if given.chars().any(char::is_control) {
        return Err(invalid("name", "it has a control character in it"));
    }
    Ok(given.to_string())
}

/// A header name for a key, checked: a token of letters, digits and
/// dashes, and not one HTTP itself sets.
pub fn header(given: &str) -> Result<Option<String>> {
    let given = given.trim();
    if given.is_empty() || given.eq_ignore_ascii_case("authorization") {
        return Ok(None);
    }
    let token = given.len() <= 64
        && given
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let reserved = [
        "host",
        "content-type",
        "content-length",
        "accept",
        "cookie",
        "mcp-session-id",
        "mcp-protocol-version",
    ]
    .iter()
    .any(|r| given.eq_ignore_ascii_case(r));
    if !token || reserved {
        return Err(invalid(
            "header",
            "it must be a header name such as X-API-Key",
        ));
    }
    Ok(Some(given.to_string()))
}

/// A key a person pasted, checked: one line, not empty.
pub fn key(given: Key) -> Result<Key> {
    let trimmed = given.expose().trim();
    if trimmed.is_empty() {
        return Err(invalid("key", "it is empty"));
    }
    if trimmed.len() > MAX_KEY || trimmed.chars().any(char::is_control) {
        return Err(invalid(
            "key",
            "it isn't a key: too long, or more than one line",
        ));
    }
    Ok(Key::new(trimmed))
}

/// The slug a name gives: lower case letters, digits and underscores,
/// at most 20, never empty.
pub fn slug_of(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('_') && !slug.is_empty() {
            slug.push('_');
        }
        if slug.len() >= 20 {
            break;
        }
    }
    let slug = slug.trim_end_matches('_').to_string();
    if slug.is_empty() {
        "server".into()
    } else {
        slug
    }
}

/// A server as it is added, checked.
#[derive(Debug, Clone)]
pub struct NewServer {
    pub name: String,
    pub url: String,
    pub sign_in: SignInKind,
    pub header: Option<String>,
    /// For [`SignInKind::Key`]; none for a browser sign-in.
    pub key: Option<Key>,
}

impl NewServer {
    pub fn new(
        given_name: &str,
        address: &str,
        sign_in: SignInKind,
        given_header: &str,
        given_key: Option<Key>,
    ) -> Result<NewServer> {
        let url = crate::public::public_url(address)
            .map_err(|e| invalid("address", &e.to_string()))?
            .to_string();
        let key = match sign_in {
            SignInKind::Key => Some(key(given_key.unwrap_or_else(|| Key::new("")))?),
            SignInKind::Browser => None,
        };
        Ok(NewServer {
            name: name(given_name)?,
            url,
            sign_in,
            header: match sign_in {
                SignInKind::Key => header(given_header)?,
                SignInKind::Browser => None,
            },
            key,
        })
    }
}

/// A change from My profile. A `None` key keeps the kept one.
#[derive(Debug, Clone)]
pub struct ServerEdit {
    pub name: String,
    pub header: Option<String>,
    pub key: Option<Key>,
}

impl ServerEdit {
    pub fn new(given_name: &str, given_header: &str, given_key: Option<Key>) -> Result<ServerEdit> {
        Ok(ServerEdit {
            name: name(given_name)?,
            header: header(given_header)?,
            key: given_key
                .filter(|k| !k.expose().trim().is_empty())
                .map(key)
                .transpose()?,
        })
    }
}

/// Each person's own servers. Another person's id answers like a
/// missing one on every method, and changes nothing.
#[async_trait]
pub trait PersonalServers: Send + Sync {
    async fn list(&self, person: &str) -> Result<Vec<PersonalServer>>;
    async fn get(&self, person: &str, id: i64) -> Result<Option<PersonalServer>>;
    /// Kept under a slug of its name no other of the person's servers
    /// has.
    async fn add(&self, person: &str, server: &NewServer) -> Result<PersonalServer>;
    async fn update(
        &self,
        person: &str,
        id: i64,
        edit: &ServerEdit,
    ) -> Result<Option<PersonalServer>>;
    async fn remove(&self, person: &str, id: i64) -> Result<bool>;
    async fn set_mode(&self, person: &str, id: i64, tool: &str, mode: Mode) -> Result<bool>;
    /// The key or sign-in, opened, for Niles's connection only.
    async fn credential(&self, person: &str, id: i64) -> Result<Option<Credential>>;
    /// Keeps a finished browser sign-in, or newer tokens for one.
    async fn signed_in(&self, person: &str, id: i64, sign_in: &BrowserSignIn) -> Result<bool>;
    /// Every person's servers, for connecting them when Niles starts.
    async fn all(&self) -> Result<Vec<(String, PersonalServer)>>;
}

struct Row {
    person: String,
    server: PersonalServer,
    credential: Option<Credential>,
}

/// The in-memory store, for tests.
#[derive(Default)]
pub struct MemoryServers {
    kept: Mutex<(Vec<Row>, i64)>,
}

impl MemoryServers {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Vec<Row>, i64)> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A slug for `name` that none of `taken` is.
pub fn free_slug(name: &str, taken: &[String]) -> String {
    let base = slug_of(name);
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("an unbounded range always finds one")
}

#[async_trait]
impl PersonalServers for MemoryServers {
    async fn list(&self, person: &str) -> Result<Vec<PersonalServer>> {
        let kept = self.lock();
        let mut out: Vec<PersonalServer> = kept
            .0
            .iter()
            .filter(|r| r.person == person)
            .map(|r| r.server.clone())
            .collect();
        out.sort_by_key(|s| (s.name.to_lowercase(), s.id));
        Ok(out)
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<PersonalServer>> {
        Ok(self
            .lock()
            .0
            .iter()
            .find(|r| r.person == person && r.server.id == id)
            .map(|r| r.server.clone()))
    }

    async fn add(&self, person: &str, new: &NewServer) -> Result<PersonalServer> {
        let mut kept = self.lock();
        let taken: Vec<String> = kept
            .0
            .iter()
            .filter(|r| r.person == person)
            .map(|r| r.server.slug.clone())
            .collect();
        kept.1 += 1;
        let server = PersonalServer {
            id: kept.1,
            slug: free_slug(&new.name, &taken),
            name: new.name.clone(),
            url: new.url.clone(),
            sign_in: new.sign_in,
            header: new.header.clone(),
            modes: BTreeMap::new(),
            signed_in: new.key.is_some(),
        };
        kept.0.push(Row {
            person: person.to_string(),
            server: server.clone(),
            credential: new.key.clone().map(Credential::Key),
        });
        Ok(server)
    }

    async fn update(
        &self,
        person: &str,
        id: i64,
        edit: &ServerEdit,
    ) -> Result<Option<PersonalServer>> {
        let mut kept = self.lock();
        let Some(row) = kept
            .0
            .iter_mut()
            .find(|r| r.person == person && r.server.id == id)
        else {
            return Ok(None);
        };
        row.server.name = edit.name.clone();
        if row.server.sign_in == SignInKind::Key {
            row.server.header = edit.header.clone();
            if let Some(key) = &edit.key {
                row.credential = Some(Credential::Key(key.clone()));
                row.server.signed_in = true;
            }
        }
        Ok(Some(row.server.clone()))
    }

    async fn remove(&self, person: &str, id: i64) -> Result<bool> {
        let mut kept = self.lock();
        let before = kept.0.len();
        kept.0
            .retain(|r| !(r.person == person && r.server.id == id));
        Ok(kept.0.len() != before)
    }

    async fn set_mode(&self, person: &str, id: i64, tool: &str, mode: Mode) -> Result<bool> {
        let mut kept = self.lock();
        let Some(row) = kept
            .0
            .iter_mut()
            .find(|r| r.person == person && r.server.id == id)
        else {
            return Ok(false);
        };
        row.server.modes.insert(tool.to_string(), mode);
        Ok(true)
    }

    async fn credential(&self, person: &str, id: i64) -> Result<Option<Credential>> {
        Ok(self
            .lock()
            .0
            .iter()
            .find(|r| r.person == person && r.server.id == id)
            .and_then(|r| r.credential.clone()))
    }

    async fn signed_in(&self, person: &str, id: i64, sign_in: &BrowserSignIn) -> Result<bool> {
        let mut kept = self.lock();
        let Some(row) = kept.0.iter_mut().find(|r| {
            r.person == person && r.server.id == id && r.server.sign_in == SignInKind::Browser
        }) else {
            return Ok(false);
        };
        row.credential = Some(Credential::Browser(Box::new(sign_in.clone())));
        row.server.signed_in = true;
        Ok(true)
    }

    async fn all(&self) -> Result<Vec<(String, PersonalServer)>> {
        Ok(self
            .lock()
            .0
            .iter()
            .map(|r| (r.person.clone(), r.server.clone()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAJSE: &str = "person:majse";
    const MARK: &str = "person:mark";

    fn notion() -> NewServer {
        NewServer::new(
            "Notion",
            "https://mcp.notion.com/mcp",
            SignInKind::Key,
            "",
            Some(Key::new("secret-token")),
        )
        .unwrap()
    }

    #[test]
    fn a_new_server_is_checked_before_anything_is_kept() {
        assert!(
            NewServer::new(
                "Notion",
                "https://192.168.1.2/mcp",
                SignInKind::Key,
                "",
                Some(Key::new("k"))
            )
            .is_err()
        );
        assert!(
            NewServer::new(
                "Notion",
                "https://mcp.notion.com/mcp",
                SignInKind::Key,
                "",
                None
            )
            .is_err()
        );
        assert!(
            NewServer::new(
                "",
                "https://mcp.notion.com/mcp",
                SignInKind::Browser,
                "",
                None
            )
            .is_err()
        );
        assert!(
            NewServer::new(
                "Notion",
                "https://mcp.notion.com/mcp",
                SignInKind::Key,
                "Host",
                Some(Key::new("k"))
            )
            .is_err()
        );
        let custom = NewServer::new(
            "X",
            "https://mcp.example.com/",
            SignInKind::Key,
            "X-API-Key",
            Some(Key::new("k")),
        )
        .unwrap();
        assert_eq!(custom.header.as_deref(), Some("X-API-Key"));
        assert!(!format!("{custom:?}").contains("\"k\""));
    }

    #[test]
    fn slugs_are_short_plain_and_unique_per_person() {
        assert_eq!(slug_of("My Notion!"), "my_notion");
        assert_eq!(slug_of("???"), "server");
        assert_eq!(free_slug("Notion", &["notion".into()]), "notion_2");
    }

    #[tokio::test]
    async fn a_server_is_its_persons_alone() {
        let store = MemoryServers::new();
        let server = store.add(MAJSE, &notion()).await.unwrap();
        assert!(server.signed_in);
        assert!(store.list(MARK).await.unwrap().is_empty());
        assert!(store.get(MARK, server.id).await.unwrap().is_none());
        assert!(store.credential(MARK, server.id).await.unwrap().is_none());
        assert!(
            !store
                .set_mode(MARK, server.id, "search", Mode::On)
                .await
                .unwrap()
        );
        assert!(!store.remove(MARK, server.id).await.unwrap());
        // Two people may both have a "Notion": each slug is per person.
        let his = store.add(MARK, &notion()).await.unwrap();
        assert_eq!(his.slug, server.slug);
        assert_eq!(store.add(MAJSE, &notion()).await.unwrap().slug, "notion_2");
    }

    #[tokio::test]
    async fn an_empty_key_in_an_edit_keeps_the_kept_one() {
        let store = MemoryServers::new();
        let server = store.add(MAJSE, &notion()).await.unwrap();
        let edit = ServerEdit::new("Work Notion", "", Some(Key::new(""))).unwrap();
        store.update(MAJSE, server.id, &edit).await.unwrap();
        assert_eq!(
            store.credential(MAJSE, server.id).await.unwrap(),
            Some(Credential::Key(Key::new("secret-token")))
        );
    }

    #[test]
    fn read_only_tools_run_and_the_rest_ask_until_chosen() {
        let tool = |read_only| RemoteTool {
            name: "t".into(),
            title: None,
            description: String::new(),
            input_schema: serde_json::json!({}),
            read_only,
            destructive: false,
        };
        assert_eq!(Mode::default_for(&tool(true)), Mode::On);
        assert_eq!(Mode::default_for(&tool(false)), Mode::Ask);
    }
}
