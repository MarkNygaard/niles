//! Each person's own MCP servers, connected and kept in step with My
//! profile. See ARCHITECTURE.md's "Each person's own tool servers".
//!
//! A server's tools go into the shared [`DynamicTools`] as
//! `my-<slug>__<tool>`, under an owner of the person's own, and are
//! offered only to that person, typing in the app's chat: never on a
//! satellite, in a spoken turn or in a job nobody is in front of. A tool
//! the server says only reads runs; any other asks on a card first,
//! until the person sets it otherwise. Every connection, and every
//! sign-in, is held to the public internet.

use crate::approvals::{Approvals, AskFirst, ToolCard};
use crate::tool_servers::{action, cap, describe, result_value};
use async_trait::async_trait;
use chrono::Utc;
use niles_api::personal_servers::{PersonalServerView, PersonalToolServers, PersonalToolView};
use niles_api::tool_servers::ServerStatus;
use niles_mcp::{
    Auth, BrowserSignIn, Credential, McpClient, Mode, NewServer, PersonalServer, PersonalServers,
    Pkce, Reach, Registration, RemoteTool, ServerEdit, SignInClient, SignInKind,
};
use niles_tools::{DynamicTools, Tool, ToolDescriptor};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long one request to a person's server may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// How long a browser sign-in may take from start to return.
const SIGN_IN_TAKES: Duration = Duration::from_secs(10 * 60);
/// How long before its access token runs out a sign-in is refreshed.
const REFRESH_BEFORE: i64 = 2;

/// Whether the turn running now may use `person`'s own tools: theirs,
/// typed in the app's chat, not a voice.
pub(crate) fn theirs_now(person: &str) -> bool {
    crate::approvals::asker()
        .is_some_and(|a| a.person == person && a.in_chat && a.satellite.is_none())
}

fn owner(person: &str, id: i64) -> String {
    format!("{person}:mcp:{id}")
}

/// One server's connection, as far as it got.
struct Conn {
    status: ServerStatus,
    message: Option<String>,
    tools: Vec<RemoteTool>,
    /// Bumped by each attempt, so a slow one that finishes after a newer
    /// one has started is thrown away.
    attempt: u64,
}

/// A browser sign-in begun and not yet back.
struct Pending {
    person: String,
    server: i64,
    auth: niles_mcp::AuthServer,
    client: Registration,
    verifier: String,
    redirect_uri: String,
    began: Instant,
}

pub(crate) struct PersonalTools {
    store: Arc<dyn PersonalServers>,
    dynamic: DynamicTools,
    approvals: Arc<Approvals>,
    conns: Mutex<HashMap<i64, Conn>>,
    pending: Mutex<HashMap<String, Pending>>,
    reach: Reach,
}

impl PersonalTools {
    /// Start connecting every person's servers.
    pub(crate) fn start(
        store: Arc<dyn PersonalServers>,
        dynamic: DynamicTools,
        approvals: Arc<Approvals>,
    ) -> Arc<Self> {
        let tools = Self::built(store, dynamic, approvals, Reach::PublicInternet);
        let starting = tools.clone();
        tokio::spawn(async move {
            match starting.store.all().await {
                Ok(all) => {
                    for (person, server) in all {
                        starting.clone().connect(person, server);
                    }
                }
                Err(e) => tracing::warn!("[my servers] couldn't be read: {e}"),
            }
        });
        tools
    }

    fn built(
        store: Arc<dyn PersonalServers>,
        dynamic: DynamicTools,
        approvals: Arc<Approvals>,
        reach: Reach,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            dynamic,
            approvals,
            conns: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            reach,
        })
    }

    fn conns(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Conn>> {
        self.conns.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn mark(&self, id: i64, attempt: u64, status: ServerStatus, message: Option<String>) {
        let mut conns = self.conns();
        let conn = conns.entry(id).or_insert_with(|| Conn {
            status: ServerStatus::Connecting,
            message: None,
            tools: Vec::new(),
            attempt,
        });
        if conn.attempt == attempt {
            conn.status = status;
            conn.message = message;
        }
    }

    /// Connect `server` again and offer its tools, in the background.
    fn connect(self: Arc<Self>, person: String, server: PersonalServer) {
        let attempt = {
            let mut conns = self.conns();
            let conn = conns.entry(server.id).or_insert_with(|| Conn {
                status: ServerStatus::Connecting,
                message: None,
                tools: Vec::new(),
                attempt: 0,
            });
            conn.attempt += 1;
            conn.status = ServerStatus::Connecting;
            conn.message = None;
            conn.attempt
        };
        tokio::spawn(async move {
            match self.open(&person, &server).await {
                Ok(Some((client, tools, secrets))) => {
                    self.offer(&person, &server, &client, &tools, secrets);
                    let mut conns = self.conns();
                    if let Some(conn) = conns.get_mut(&server.id)
                        && conn.attempt == attempt
                    {
                        conn.status = ServerStatus::Working;
                        conn.message = None;
                        conn.tools = tools;
                    }
                }
                Ok(None) => {
                    self.dynamic.remove(&owner(&person, server.id));
                    let status = match server.sign_in {
                        SignInKind::Key => ServerStatus::NeedsKey,
                        SignInKind::Browser => ServerStatus::NeedsSignIn,
                    };
                    self.mark(server.id, attempt, status, None);
                }
                Err(e) => {
                    self.dynamic.remove(&owner(&person, server.id));
                    let status = if matches!(e, niles_mcp::Error::Unauthorized { .. })
                        && server.sign_in == SignInKind::Browser
                    {
                        ServerStatus::NeedsSignIn
                    } else {
                        ServerStatus::Failed
                    };
                    tracing::info!("[my servers] {person}'s server {}: {e}", server.id);
                    self.mark(server.id, attempt, status, Some(e.to_string()));
                }
            }
        });
    }

    /// The server's session and its tools, or `None` with nothing to
    /// sign in with. A browser sign-in about to run out is refreshed
    /// first, and the new tokens kept.
    async fn open(
        &self,
        person: &str,
        server: &PersonalServer,
    ) -> niles_mcp::Result<Option<(Arc<McpClient>, Vec<RemoteTool>, Vec<String>)>> {
        let Some(credential) = self.store.credential(person, server.id).await? else {
            return Ok(None);
        };
        let (auth, secrets) = match credential {
            Credential::Key(key) => {
                let value = key.expose().to_string();
                let auth = match &server.header {
                    Some(name) => Auth::Header {
                        name: name.clone(),
                        value: value.clone(),
                    },
                    None => Auth::Bearer(value.clone()),
                };
                (auth, vec![value])
            }
            Credential::Browser(mut sign_in) => {
                if sign_in
                    .tokens
                    .expiring(Utc::now(), chrono::Duration::minutes(REFRESH_BEFORE))
                {
                    *sign_in = self.refresh(person, server.id, *sign_in).await?;
                }
                let mut secrets = vec![sign_in.tokens.access.clone()];
                secrets.extend(sign_in.tokens.refresh.clone());
                (Auth::Bearer(sign_in.tokens.access.clone()), secrets)
            }
        };
        let (client, _) = McpClient::connect_with(&server.url, auth, TIMEOUT, self.reach).await?;
        let tools = client.list_tools().await?;
        Ok(Some((Arc::new(client), tools, secrets)))
    }

    async fn refresh(
        &self,
        person: &str,
        id: i64,
        mut sign_in: BrowserSignIn,
    ) -> niles_mcp::Result<BrowserSignIn> {
        let Some(refresh) = sign_in.tokens.refresh.clone() else {
            return Err(niles_mcp::Error::SignIn(
                "the sign-in ran out; sign in again".into(),
            ));
        };
        sign_in.tokens = SignInClient::new(self.reach)?
            .refresh(&sign_in.auth, &sign_in.client, &refresh)
            .await?;
        self.store.signed_in(person, id, &sign_in).await?;
        Ok(sign_in)
    }

    /// Put a server's tools that aren't off into the registry, as the
    /// person's own.
    fn offer(
        &self,
        person: &str,
        server: &PersonalServer,
        client: &Arc<McpClient>,
        tools: &[RemoteTool],
        secrets: Vec<String>,
    ) {
        // A household server's id can't hold a dash, so these names
        // never meet a household tool's.
        let prefix = format!("my-{}", server.slug);
        let secrets = Arc::new(secrets);
        let offered: Vec<Arc<dyn Tool>> = tools
            .iter()
            .filter(|tool| server.mode(tool) != Mode::Off)
            .filter_map(|tool| {
                let descriptor = describe(&prefix, tool)?;
                let own = Arc::new(OwnTool {
                    person: person.to_string(),
                    client: client.clone(),
                    name: tool.name.clone(),
                    descriptor,
                    secrets: secrets.clone(),
                }) as Arc<dyn Tool>;
                if server.mode(tool) == Mode::Ask {
                    let card = ToolCard {
                        title: format!("Let {} {}?", server.name, action(tool)),
                        subtitle: server.name.clone(),
                        icon: None,
                        schema: tool.input_schema.clone(),
                    };
                    return Some(
                        Arc::new(AskFirst::new(own, self.approvals.clone(), card)) as Arc<dyn Tool>
                    );
                }
                Some(own)
            })
            .collect();
        let keywords = vec![server.slug.clone(), server.name.to_lowercase()];
        self.dynamic
            .set(&owner(person, server.id), keywords, offered);
    }

    fn view(&self, server: PersonalServer) -> PersonalServerView {
        let conns = self.conns();
        let conn = conns.get(&server.id);
        let tools = conn
            .map(|c| {
                c.tools
                    .iter()
                    .map(|t| PersonalToolView {
                        name: t.name.clone(),
                        title: t.title.clone(),
                        description: t.description.clone(),
                        read_only: t.read_only,
                        destructive: t.destructive,
                        mode: server.mode(t),
                    })
                    .collect()
            })
            .unwrap_or_default();
        PersonalServerView {
            status: conn.map_or(ServerStatus::Connecting, |c| c.status.clone()),
            message: conn.and_then(|c| c.message.clone()),
            tools,
            server,
        }
    }

    fn forget(&self, person: &str, id: i64) {
        self.dynamic.remove(&owner(person, id));
        self.conns().remove(&id);
        self.pending().retain(|_, p| p.server != id);
    }
}

/// One of a person's tools: offered to them alone, and called only for
/// them. What comes back has the server's key and tokens taken out.
struct OwnTool {
    person: String,
    client: Arc<McpClient>,
    /// The server's own name, which is what `tools/call` takes.
    name: String,
    descriptor: ToolDescriptor,
    secrets: Arc<Vec<String>>,
}

impl OwnTool {
    fn scrubbed(&self, text: String) -> String {
        self.secrets
            .iter()
            .filter(|s| s.len() >= 8)
            .fold(text, |text, secret| text.replace(secret.as_str(), "•••"))
    }
}

#[async_trait]
impl Tool for OwnTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    fn offered(&self) -> bool {
        theirs_now(&self.person)
    }

    async fn execute(&self, args: Value) -> niles_tools::Result<Value> {
        // The registry finds only offered tools; this is the second wall.
        if !theirs_now(&self.person) {
            return Err(niles_tools::Error::Remote(
                "that tool belongs to someone else".into(),
            ));
        }
        match self.client.call_tool(&self.name, args).await {
            Ok(result) if !result.is_error => Ok(result_value(self.scrubbed(result.text))),
            Ok(result) if result.text.trim().is_empty() => {
                Err(niles_tools::Error::Remote(format!("{} failed", self.name)))
            }
            Ok(result) => Err(niles_tools::Error::Remote(cap(self.scrubbed(result.text)))),
            Err(e) => Err(niles_tools::Error::Remote(format!(
                "{} could not be reached: {}",
                self.descriptor.name,
                self.scrubbed(e.to_string())
            ))),
        }
    }
}

/// What the app is told and asks for.
pub(crate) struct AppPersonalServers(pub(crate) Arc<PersonalTools>);

#[async_trait]
impl PersonalToolServers for AppPersonalServers {
    async fn list(&self, person: &str) -> niles_mcp::Result<Vec<PersonalServerView>> {
        let servers = self.0.store.list(person).await?;
        Ok(servers.into_iter().map(|s| self.0.view(s)).collect())
    }

    async fn add(&self, person: &str, server: NewServer) -> niles_mcp::Result<PersonalServerView> {
        let kept = self.0.store.add(person, &server).await?;
        self.0.clone().connect(person.to_string(), kept.clone());
        Ok(self.0.view(kept))
    }

    async fn update(
        &self,
        person: &str,
        id: i64,
        edit: ServerEdit,
    ) -> niles_mcp::Result<Option<PersonalServerView>> {
        let Some(kept) = self.0.store.update(person, id, &edit).await? else {
            return Ok(None);
        };
        self.0.clone().connect(person.to_string(), kept.clone());
        Ok(Some(self.0.view(kept)))
    }

    async fn remove(&self, person: &str, id: i64) -> niles_mcp::Result<bool> {
        let removed = self.0.store.remove(person, id).await?;
        if removed {
            self.0.forget(person, id);
        }
        Ok(removed)
    }

    async fn set_mode(
        &self,
        person: &str,
        id: i64,
        tool: &str,
        mode: Mode,
    ) -> niles_mcp::Result<bool> {
        if !self.0.store.set_mode(person, id, tool, mode).await? {
            return Ok(false);
        }
        if let Some(server) = self.0.store.get(person, id).await? {
            self.0.clone().connect(person.to_string(), server);
        }
        Ok(true)
    }

    async fn begin_sign_in(
        &self,
        person: &str,
        id: i64,
        redirect_uri: &str,
    ) -> niles_mcp::Result<Option<String>> {
        let Some(server) = self.0.store.get(person, id).await? else {
            return Ok(None);
        };
        if server.sign_in != SignInKind::Browser {
            return Err(niles_mcp::Error::SignIn(
                "this server signs in with a key".into(),
            ));
        }
        // The server's 401 may say where its sign-in is described.
        let hint =
            match McpClient::connect_with(&server.url, Auth::None, TIMEOUT, self.0.reach).await {
                Err(niles_mcp::Error::Unauthorized {
                    resource_metadata, ..
                }) => resource_metadata,
                _ => None,
            };
        let signing = SignInClient::new(self.0.reach)?;
        let auth = signing.discover(&server.url, hint.as_deref()).await?;
        let client = signing.register(&auth, redirect_uri).await?;
        let pkce = Pkce::new();
        let state = niles_mcp::oauth::state();
        let link = niles_mcp::authorize_url(&auth, &client, redirect_uri, &state, &pkce.challenge)?;
        let mut pending = self.0.pending();
        pending.retain(|_, p| p.began.elapsed() < SIGN_IN_TAKES);
        pending.insert(
            state,
            Pending {
                person: person.to_string(),
                server: id,
                auth,
                client,
                verifier: pkce.verifier,
                redirect_uri: redirect_uri.to_string(),
                began: Instant::now(),
            },
        );
        Ok(Some(link))
    }

    async fn finish_sign_in(
        &self,
        person: &str,
        state: &str,
        code: &str,
    ) -> niles_mcp::Result<i64> {
        let pending = {
            let mut pending = self.0.pending();
            // Taken whoever asks, so a state is used once; answered only
            // for the person who began it.
            pending.remove(state)
        };
        let Some(pending) =
            pending.filter(|p| p.person == person && p.began.elapsed() < SIGN_IN_TAKES)
        else {
            return Err(niles_mcp::Error::SignIn(
                "no sign-in of yours is waiting; start it again".into(),
            ));
        };
        let tokens = SignInClient::new(self.0.reach)?
            .exchange(
                &pending.auth,
                &pending.client,
                code,
                &pending.verifier,
                &pending.redirect_uri,
            )
            .await?;
        let sign_in = BrowserSignIn {
            auth: pending.auth,
            client: pending.client,
            tokens,
        };
        if !self
            .0
            .store
            .signed_in(person, pending.server, &sign_in)
            .await?
        {
            return Err(niles_mcp::Error::SignIn("the server is gone".into()));
        }
        if let Some(server) = self.0.store.get(person, pending.server).await? {
            self.0.clone().connect(person.to_string(), server);
        }
        tracing::info!(
            "[my servers] {person} signed in to server {}",
            pending.server
        );
        Ok(pending.server)
    }

    async fn reconnect(&self, person: &str, id: i64) -> niles_mcp::Result<bool> {
        let Some(server) = self.0.store.get(person, id).await? else {
            return Ok(false);
        };
        self.0.clone().connect(person.to_string(), server);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{ASKER, Asker};
    use niles_mcp::{Key, MemoryServers};
    use niles_tools::ToolRegistry;
    use serde_json::json;
    use std::net::SocketAddr;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const MAJSE: &str = "person:majse";
    const MARK: &str = "person:mark";
    const KEY: &str = "notion-secret-key-123";

    /// An MCP server with a tool that reads and one that writes, whose
    /// answer echoes the key it was sent, as a careless server might.
    async fn notion() -> MockServer {
        let server = MockServer::start().await;
        let answer = |result: Value| {
            move |req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].clone();
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
            }
        };
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "initialize" })))
            .respond_with(answer(
                json!({ "protocolVersion": "2025-06-18", "capabilities": {} }),
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({ "method": "notifications/initialized" }),
            ))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .respond_with(answer(json!({ "tools": [
                { "name": "search", "description": "Search pages.",
                  "annotations": { "readOnlyHint": true } },
                { "name": "create_page", "description": "Create a page." },
            ] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/call" })))
            .respond_with(answer(json!({ "content": [
                { "type": "text", "text": format!("found 2 pages; your key is {KEY}") }
            ] })))
            .mount(&server)
            .await;
        server
    }

    fn asker(person: &str, in_chat: bool, satellite: Option<SocketAddr>) -> Asker {
        Asker {
            person: person.into(),
            slug: None,
            satellite,
            conversation: Some(1),
            in_chat,
        }
    }

    struct Harness {
        tools: Arc<PersonalTools>,
        store: Arc<MemoryServers>,
        registry: ToolRegistry,
        server: PersonalServer,
        _mcp: MockServer,
    }

    /// Majse's "Notion", on a stand-in reached as the household's would
    /// be: the stand-in is on loopback, which a person's real server
    /// never may be.
    async fn harness() -> Harness {
        let mcp = notion().await;
        let store = Arc::new(MemoryServers::new());
        let new = NewServer {
            name: "Notion".into(),
            url: mcp.uri(),
            sign_in: SignInKind::Key,
            header: None,
            key: Some(Key::new(KEY)),
        };
        let server = store.add(MAJSE, &new).await.unwrap();
        let dynamic = DynamicTools::new();
        let approvals = Arc::new(Approvals::new(niles_core::EventBus::default(), None));
        let tools =
            PersonalTools::built(store.clone(), dynamic.clone(), approvals, Reach::Anywhere);
        let mut registry = ToolRegistry::new();
        registry.attach_dynamic(dynamic);
        tools.clone().connect(MAJSE.into(), server.clone());
        until(|| {
            tools
                .conns()
                .get(&server.id)
                .is_some_and(|c| c.status == ServerStatus::Working)
        })
        .await;
        Harness {
            tools,
            store,
            registry,
            server,
            _mcp: mcp,
        }
    }

    async fn until(mut done: impl FnMut() -> bool) {
        for _ in 0..300 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out");
    }

    fn offered(registry: &ToolRegistry) -> Vec<String> {
        registry.llm_tools().into_iter().map(|t| t.name).collect()
    }

    fn call(name: &str) -> niles_llm::ToolCall {
        niles_llm::ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: json!({}),
        }
    }

    #[tokio::test]
    async fn the_tools_are_offered_to_their_person_typing_in_the_app_alone() {
        let h = harness().await;
        let hers = ASKER
            .scope(asker(MAJSE, true, None), async { offered(&h.registry) })
            .await;
        assert!(hers.contains(&"my-notion__search".to_string()), "{hers:?}");
        assert!(
            hers.contains(&"my-notion__create_page".to_string()),
            "{hers:?}"
        );

        let satellite: SocketAddr = "192.168.1.50:5000".parse().unwrap();
        for (who, case) in [
            (asker(MARK, true, None), "another person"),
            (
                asker(MAJSE, true, Some(satellite)),
                "her voice on a satellite",
            ),
            (
                asker(MAJSE, false, None),
                "a job for her nobody is in front of",
            ),
        ] {
            let seen = ASKER.scope(who, async { offered(&h.registry) }).await;
            assert!(
                !seen.iter().any(|n| n.starts_with("my-")),
                "{case}: {seen:?}"
            );
        }
        assert!(
            !offered(&h.registry).iter().any(|n| n.starts_with("my-")),
            "no turn at all"
        );
    }

    #[tokio::test]
    async fn another_person_can_t_call_them_by_name() {
        let h = harness().await;
        let refused = ASKER
            .scope(
                asker(MARK, true, None),
                h.registry.execute(&call("my-notion__search")),
            )
            .await;
        assert!(refused.is_err());
    }

    #[tokio::test]
    async fn a_read_runs_its_answer_without_the_key() {
        let h = harness().await;
        let answer = ASKER
            .scope(
                asker(MAJSE, true, None),
                h.registry.execute(&call("my-notion__search")),
            )
            .await
            .expect("a read runs");
        let text = answer.to_string();
        assert!(text.contains("found 2 pages"), "{text}");
        assert!(!text.contains(KEY), "the key is scrubbed: {text}");
    }

    #[tokio::test]
    async fn a_write_waits_on_its_card_and_off_takes_it_away() {
        let h = harness().await;
        let held = ASKER
            .scope(
                asker(MAJSE, true, None),
                h.registry.execute(&call("my-notion__create_page")),
            )
            .await
            .expect("held, not refused");
        assert!(
            held.to_string().to_lowercase().contains("card")
                || held.to_string().to_lowercase().contains("wait"),
            "{held}"
        );

        let app = AppPersonalServers(h.tools.clone());
        assert!(
            app.set_mode(MAJSE, h.server.id, "create_page", Mode::Off)
                .await
                .unwrap()
        );
        until(|| {
            h.tools
                .dynamic
                .names()
                .iter()
                .all(|(_, name)| name != "my-notion__create_page")
        })
        .await;
        let hers = ASKER
            .scope(asker(MAJSE, true, None), async { offered(&h.registry) })
            .await;
        assert!(hers.contains(&"my-notion__search".to_string()));
        assert!(!hers.contains(&"my-notion__create_page".to_string()));
    }

    #[tokio::test]
    async fn the_app_sees_status_and_settings_and_never_the_key() {
        let h = harness().await;
        let app = AppPersonalServers(h.tools.clone());
        let listed = app.list(MAJSE).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, ServerStatus::Working);
        let modes: Vec<(String, Mode)> = listed[0]
            .tools
            .iter()
            .map(|t| (t.name.clone(), t.mode))
            .collect();
        assert!(modes.contains(&("search".into(), Mode::On)));
        assert!(modes.contains(&("create_page".into(), Mode::Ask)));
        assert!(!serde_json::to_string(&listed).unwrap().contains(KEY));
        assert!(app.list(MARK).await.unwrap().is_empty());
        assert!(!app.remove(MARK, h.server.id).await.unwrap());
    }

    #[tokio::test]
    async fn removing_a_server_takes_its_tools_away() {
        let h = harness().await;
        let app = AppPersonalServers(h.tools.clone());
        assert!(app.remove(MAJSE, h.server.id).await.unwrap());
        assert!(h.store.list(MAJSE).await.unwrap().is_empty());
        let hers = ASKER
            .scope(asker(MAJSE, true, None), async { offered(&h.registry) })
            .await;
        assert!(!hers.iter().any(|n| n.starts_with("my-")), "{hers:?}");
    }

    #[tokio::test]
    async fn a_sign_in_is_finished_only_by_who_began_it() {
        let h = harness().await;
        let app = AppPersonalServers(h.tools.clone());
        // A key server has no browser sign-in.
        assert!(
            app.begin_sign_in(MAJSE, h.server.id, "https://niles.example/cb")
                .await
                .is_err()
        );
        // An unknown, or someone else's, state finishes nothing.
        h.tools.pending().insert(
            "s1".into(),
            Pending {
                person: MAJSE.into(),
                server: h.server.id,
                auth: niles_mcp::AuthServer {
                    authorization_endpoint: "https://auth.example/a".into(),
                    token_endpoint: "https://auth.example/t".into(),
                    registration_endpoint: None,
                    resource: "https://mcp.example/mcp".into(),
                    scope: String::new(),
                },
                client: Registration {
                    client_id: "c".into(),
                    client_secret: None,
                },
                verifier: "v".into(),
                redirect_uri: "https://niles.example/cb".into(),
                began: Instant::now(),
            },
        );
        assert!(app.finish_sign_in(MARK, "s1", "code").await.is_err());
        assert!(
            h.tools.pending().is_empty(),
            "a state is used once, whoever brings it"
        );
        assert!(app.finish_sign_in(MAJSE, "nope", "code").await.is_err());
    }
}
