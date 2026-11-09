//! The sign-in card and what Allow does: Niles binds the session to
//! the login in place, turns the guard on, restores the kept profile
//! and fills the form itself — only ever on the login's origins, each
//! value typed at most once a run. See ARCHITECTURE.md's "Signing in".

use std::sync::{Arc, Weak};

use chrono::Utc;
use niles_api::chat::{ApprovalCard, ApprovalStatus, CardField};
use niles_browser::{Asks, on_sites};
use niles_logins::{Credentials, Login};
use niles_tools::{Tool, ToolDescriptor};
use serde_json::{Value, json};
use url::Url;

use super::login::{NO_LOGINS, NOBODY, nobody};
use super::scrub::Scrub;
use super::step::{SESSION_ENDED, lapsed, page_of};
use super::tools::{CAPTCHA_NOTE, WAITING, in_the_chat, invalid, note, remote};
use super::{Bound, Browsing, Create, NOT_FOR_JOBS, Opened, State};
use crate::approvals::Asker;

const TOOL: &str = "browser_sign_in";

/// The card's note: what the model is told to do about it.
const SIGN_IN_NOTE: &str = "Nothing has happened yet. Under your reply the person has a card to sign in with their saved login, with a picture of the page, Allow once and Deny. End your turn now and say in one short sentence what is waiting for them. If they allow it, Niles fills in the login itself, never showing it to you, and starts your next turn; don't ask them to type yes or go on.";

/// Why a run lapses or stops, each a fixed sentence with no page text.
const LOGIN_GONE: &str =
    "The saved login was removed before the card was answered, so nothing was typed.";
const LEFT_ITS_SITES: &str =
    "The page left the login's sites before the card was answered, so nothing was typed.";
const LEFT_MID_SIGN_IN: &str =
    "The page left the login's sites while Niles signed in, so it typed nothing more there.";
const CAPTCHA_STOP: &str = "The site wants a person to show they aren't a robot (a CAPTCHA). Niles doesn't solve those, so it stopped there.";
const NO_FORM: &str = "Niles finds no sign-in form on this page to fill. Open the site's sign-in page first; a link or button on the page usually leads there.";

/// Sign in to the open site with one of the person's saved logins:
/// `browser_sign_in`. Always a card; Allow runs [`SignInStep`].
pub(super) struct SignIn(pub(super) Arc<Browsing>);

#[async_trait::async_trait]
impl Tool for SignIn {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: TOOL.into(),
            description: "Sign in to the open site with one of the person's saved logins, by the id request_login gave. Open the site's sign-in page first. Niles finds the username and password fields and fills them itself, only on the login's own sites; you never see either. It waits on a card: end your turn after calling it, and Niles starts your next turn once the person allows it. A signed-in session stays on the login's sites. When a page asks for a code the site sent, call this too: Niles asks the person for the code on a card you never see.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "login_id": {
                        "type": ["integer", "string"],
                        "title": "Login",
                        "description": "The saved login's id, as request_login gave it."
                    }
                },
                "required": ["login_id"],
            }),
        }
    }

    fn offered(&self) -> bool {
        in_the_chat().is_some_and(|asker| asker.person != nobody())
            && self.0.store().is_some()
            && matches!(self.0.state(), State::Ready(_))
    }

    async fn execute(&self, args: Value) -> niles_tools::Result<Value> {
        let Some(asker) = in_the_chat() else {
            return Err(invalid(TOOL, NOT_FOR_JOBS));
        };
        if let State::Off(why) = self.0.state() {
            return Ok(note(why));
        }
        let scrub = match self.0.scrub(&asker.person).await {
            Ok(scrub) => scrub,
            Err(why) => return Ok(note(why)),
        };
        scrub.result(hold_sign_in(&self.0, &asker, &args, &scrub).await)
    }
}

/// What the card waits on, and what runs when it is allowed.
pub(super) struct SignInStep {
    browsing: Weak<Browsing>,
    asker: Asker,
    serial: u64,
    login: i64,
}

#[async_trait::async_trait]
impl Tool for SignInStep {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: TOOL.into(),
            description: "Do the approved sign-in.".into(),
            parameters: json!({ "type": "object" }),
        }
    }

    async fn execute(&self, _args: Value) -> niles_tools::Result<Value> {
        let Some(browsing) = self.browsing.upgrade() else {
            return Err(lapsed(SESSION_ENDED));
        };
        let scrub = match browsing.scrub(&self.asker.person).await {
            Ok(scrub) => scrub,
            Err(why) => return Err(niles_tools::Error::Remote(why.into())),
        };
        scrub.result(self.run(&browsing).await)
    }
}

/// The call-time checks and the card. Each refusal names what the model
/// can fix; nothing is typed by this half.
async fn hold_sign_in(
    browsing: &Arc<Browsing>,
    asker: &Asker,
    args: &Value,
    scrub: &Scrub,
) -> niles_tools::Result<Value> {
    let Some(store) = browsing.store() else {
        return Ok(note(NO_LOGINS));
    };
    if asker.person == nobody() {
        return Ok(note(NOBODY));
    }
    let id = login_id(args)?;
    let login = match store.get(&asker.person, id).await {
        Ok(Some(login)) => login,
        Ok(None) => {
            return Err(invalid(
                TOOL,
                format!(
                    "No saved login {id} for this person. request_login answers with their saved logins for a site."
                ),
            ));
        }
        Err(e) => {
            return Err(niles_tools::Error::Remote(format!(
                "saved logins couldn't be read: {e}"
            )));
        }
    };
    let origins: Vec<String> = login
        .origins
        .iter()
        .map(|o| o.as_str().to_string())
        .collect();
    let opened = browsing
        .session(asker, Create::No)
        .await
        .map_err(|reason| invalid(TOOL, reason))?;
    let _guard = opened.lock.lock().await;
    session_checks(browsing, &asker.person, opened.serial, id, &login.label)?;
    let address = opened.tab.address().await.map_err(remote)?;
    if !on_sites(&address, &origins) {
        return Err(invalid(
            TOOL,
            format!(
                "The open page isn't on one of {}'s sites ({}). Open its sign-in page with browser_open first; Niles fills a login only on its own sites.",
                login.label,
                origins.join(", ")
            ),
        ));
    }
    match page_ask(
        &opened.tab,
        &origins,
        &login,
        id,
        opened.serial,
        browsing,
        &asker.person,
    )
    .await?
    {
        PageAsk::Captcha => return Ok(json!({ "captcha": true, "note": CAPTCHA_NOTE })),
        PageAsk::NoForm => return Err(invalid(TOOL, NO_FORM)),
        PageAsk::Ready => {}
    }
    let png = opened.tab.screenshot().await.map_err(remote)?;
    let picture = niles_browser::picture(&png, None).map_err(remote)?;
    let card = scrub.card(sign_in_card(&login, &address));
    let step = Arc::new(SignInStep {
        browsing: Arc::downgrade(browsing),
        asker: asker.clone(),
        serial: opened.serial,
        login: id,
    });
    let held = browsing.approvals.hold_step(
        asker,
        step,
        "Browser",
        card,
        niles_api::approvals::Picture {
            mime: "image/jpeg",
            bytes: picture,
        },
    );
    browsing.hold(&asker.person, opened.serial, held.id.clone());
    tracing::info!(
        "[browser] browser_sign_in is waiting for {}'s answer",
        asker.person
    );
    Ok(json!({
        "waiting_for_approval": true,
        "shown_to_the_person": held.title,
        "note": SIGN_IN_NOTE,
    }))
}

/// The call-time session checks: a waiting step comes first, and one
/// login to a session.
fn session_checks(
    browsing: &Arc<Browsing>,
    person: &str,
    serial: u64,
    id: i64,
    label: &str,
) -> niles_tools::Result<()> {
    if browsing.waiting_step(person) {
        return Err(invalid(TOOL, WAITING));
    }
    if let Some(bound) = browsing.bound(person, serial)
        && bound.login != id
    {
        return Err(invalid(
            TOOL,
            format!(
                "This session is signed in with another login, {}. Niles keeps one login to a session: to sign in with {label}, open its sign-in page with browser_open and new_session: true first.",
                bound.label
            ),
        ));
    }
    Ok(())
}

/// What the page asks, for the call-time check: a CAPTCHA stops it with
/// a note, no form and no fresh profile is the model's to fix, anything
/// else may hold its card.
enum PageAsk {
    Captcha,
    NoForm,
    Ready,
}

async fn page_ask(
    tab: &Arc<dyn niles_browser::Tab>,
    origins: &[String],
    login: &Login,
    id: i64,
    serial: u64,
    browsing: &Arc<Browsing>,
    person: &str,
) -> niles_tools::Result<PageAsk> {
    match tab.asks(origins).await {
        Ok(Asks::Captcha) => Ok(PageAsk::Captcha),
        Ok(Asks::Nothing) => {
            // A fresh profile may sign the page in without a form; a
            // session already bound to this login may be re-checking.
            let already = browsing
                .bound(person, serial)
                .is_some_and(|b| b.login == id);
            if !already && !login.signed_in {
                Ok(PageAsk::NoForm)
            } else {
                Ok(PageAsk::Ready)
            }
        }
        Ok(_) => Ok(PageAsk::Ready),
        Err(e) => Err(remote(e)),
    }
}

/// The saved login's id: an integer, or a numeric string.
fn login_id(args: &Value) -> niles_tools::Result<i64> {
    let id = args["login_id"].as_i64().or_else(|| {
        args["login_id"]
            .as_str()
            .and_then(|s| s.trim().parse().ok())
    });
    match id {
        Some(id) if id > 0 => Ok(id),
        _ => Err(invalid(
            TOOL,
            "login_id must be a saved login's id, such as 7, as request_login gave it.",
        )),
    }
}

/// The card the person answers: the site, the page, the login's label
/// and its sites — never the username.
fn sign_in_card(login: &Login, address: &str) -> ApprovalCard {
    let subtitle = Url::parse(address)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "this page".into());
    ApprovalCard {
        id: String::new(),
        title: format!("Sign in to {}?", login.label),
        subtitle,
        icon: Some("browser".into()),
        fields: vec![
            CardField {
                label: "Page".into(),
                value: page_of(address),
            },
            CardField {
                label: "Login".into(),
                value: login.label.clone(),
            },
            CardField {
                label: "Sites".into(),
                value: login
                    .origins
                    .iter()
                    .map(|o| o.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            },
        ],
        body: None,
        allow_conversation: false,
        picture: true,
        status: ApprovalStatus::Pending,
        outcome: None,
    }
}

impl SignInStep {
    /// What Allow runs: re-check against the login as it is now, guard,
    /// bind, restore, then the fill rounds. Each value is typed at most
    /// once; the result never carries one.
    async fn run(&self, browsing: &Arc<Browsing>) -> niles_tools::Result<Value> {
        let Some(opened) = browsing.session_of(&self.asker.person, self.serial) else {
            return Err(lapsed(SESSION_ENDED));
        };
        browsing.touch(&self.asker.person, opened.serial);
        let _guard = opened.lock.lock().await;
        let Some(store) = browsing.store() else {
            return Err(lapsed(LOGIN_GONE));
        };
        let login = match store.get(&self.asker.person, self.login).await {
            Ok(Some(login)) => login,
            Ok(None) => return Err(lapsed(LOGIN_GONE)),
            Err(e) => {
                return Err(niles_tools::Error::Remote(format!(
                    "saved logins couldn't be read: {e}"
                )));
            }
        };
        let label = login.label.clone();
        let origins: Vec<String> = login
            .origins
            .iter()
            .map(|o| o.as_str().to_string())
            .collect();
        if let Some(bound) = browsing.bound(&self.asker.person, opened.serial)
            && bound.login != self.login
        {
            return Err(lapsed(SESSION_ENDED));
        }
        let address = match opened.tab.address().await {
            Ok(address) => address,
            Err(niles_browser::Error::Closed) => return Err(lapsed(SESSION_ENDED)),
            Err(e) => return Err(remote(e)),
        };
        if !on_sites(&address, &origins) {
            return Err(lapsed(LEFT_ITS_SITES));
        }
        let Some(creds) = store
            .credentials(&self.asker.person, self.login)
            .await
            .map_err(|e| {
                niles_tools::Error::Remote(format!("saved logins couldn't be read: {e}"))
            })?
        else {
            return Err(lapsed(LOGIN_GONE));
        };
        let mut restored = false;
        if browsing.bound(&self.asker.person, opened.serial).is_none() {
            // No fill without the guard.
            opened.tab.stay_on(&origins).await.map_err(remote)?;
            browsing.bind(
                &self.asker.person,
                opened.serial,
                Bound {
                    login: self.login,
                    label: label.clone(),
                    origins: origins.clone(),
                    signed_in: false,
                },
            );
            match store.profile(&self.asker.person, self.login).await {
                Ok(Some(profile)) => match opened.tab.restore(&origins, profile.expose()).await {
                    Ok(()) => {
                        restored = true;
                        browsing.mark_signed_in(&self.asker.person, opened.serial);
                    }
                    Err(e) => {
                        tracing::warn!("[browser] a browser profile wasn't restored: {e}");
                    }
                },
                Ok(None) => {}
                Err(e) => tracing::warn!("[browser] a browser profile wasn't read: {e}"),
            }
        }
        if let Err(e) = store.used(&self.asker.person, self.login, Utc::now()).await {
            tracing::warn!("[browser] a login's use wasn't noted: {e}");
        }
        self.outcome(browsing, &opened, &label, &origins, &creds, restored)
            .await
    }

    /// The rounds' outcome in the model's words: signed in or not,
    /// never the values.
    async fn outcome(
        &self,
        browsing: &Arc<Browsing>,
        opened: &Opened,
        label: &str,
        origins: &[String],
        creds: &Credentials,
        restored: bool,
    ) -> niles_tools::Result<Value> {
        match rounds(&opened.tab, origins, creds).await? {
            Round::SignedIn => {
                browsing.mark_signed_in(&self.asker.person, opened.serial);
                if let Some(bound) = browsing.bound(&self.asker.person, opened.serial) {
                    browsing.save(&self.asker.person, &bound, &opened.tab).await;
                }
                Ok(Value::String(format!(
                    "Signed in to {label}, as far as Niles can tell: the page no longer asks for the login."
                )))
            }
            Round::NothingAsked if restored => Ok(Value::String(format!(
                "Signed in to {label} with the browser profile Niles kept from last time: the page asks for nothing to sign in."
            ))),
            Round::NothingAsked => Ok(Value::String(
                "Didn't sign in: Niles found no sign-in form it may fill on the page.".into(),
            )),
            Round::Code => {
                let site = origin_of(&opened.tab.address().await.unwrap_or_default());
                let bound = browsing
                    .bound(&self.asker.person, opened.serial)
                    .expect("bound above");
                browsing.ask_code(&self.asker.person, opened.serial, &bound, site);
                Ok(Value::String(format!(
                    "{label} sent the person a code to finish signing in."
                )))
            }
            Round::Captcha => Err(niles_tools::Error::Remote(CAPTCHA_STOP.into())),
            Round::StillAsks => Ok(Value::String(
                "Didn't sign in: the page still asks for the login, so the saved one may be wrong. The person can check it under My profile → Logins.".into(),
            )),
            Round::OffSite => Err(niles_tools::Error::Remote(LEFT_MID_SIGN_IN.into())),
            Round::Gone => Err(lapsed(SESSION_ENDED)),
        }
    }
}

/// What one round of filling found the page asking.
enum Round {
    SignedIn,
    NothingAsked,
    Code,
    Captcha,
    StillAsks,
    OffSite,
    Gone,
}

/// Fill the form over at most four rounds, checking the top origin
/// again after every redirect. Each value is typed at most once a run,
/// so a wrong password is never retried.
async fn rounds(
    tab: &Arc<dyn niles_browser::Tab>,
    origins: &[String],
    creds: &Credentials,
) -> niles_tools::Result<Round> {
    let mut typed_user = false;
    let mut typed_pass = false;
    for _ in 0..4 {
        let address = match tab.address().await {
            Ok(address) => address,
            Err(niles_browser::Error::Closed) => return Ok(Round::Gone),
            Err(e) => return Err(remote(e)),
        };
        if !on_sites(&address, origins) {
            return Ok(Round::OffSite);
        }
        let asks = match tab.asks(origins).await {
            Ok(asks) => asks,
            Err(niles_browser::Error::Closed) => return Ok(Round::Gone),
            Err(e) => return Err(remote(e)),
        };
        match asks {
            Asks::Captcha => return Ok(Round::Captcha),
            Asks::Code => return Ok(Round::Code),
            Asks::Nothing => {
                return Ok(if typed_user || typed_pass {
                    Round::SignedIn
                } else {
                    Round::NothingAsked
                });
            }
            Asks::Password { username } => {
                if typed_pass {
                    return Ok(Round::StillAsks);
                }
                let user = (username && !typed_user).then(|| creds.username.expose());
                let filled = fill(tab, origins, user, Some(creds.password.expose())).await?;
                if !filled.password {
                    return Ok(if typed_user || filled.username {
                        Round::StillAsks
                    } else {
                        Round::NothingAsked
                    });
                }
                typed_pass = true;
                typed_user |= filled.username;
            }
            Asks::Username => {
                if typed_user {
                    return Ok(Round::StillAsks);
                }
                let filled = fill(tab, origins, Some(creds.username.expose()), None).await?;
                if !filled.username {
                    return Ok(if typed_pass {
                        Round::StillAsks
                    } else {
                        Round::NothingAsked
                    });
                }
                typed_user = true;
            }
            // A kind of ask this Niles doesn't know: never guess.
            _ => return Ok(Round::StillAsks),
        }
    }
    Ok(Round::StillAsks)
}

/// One fill; a closed page ends the run.
async fn fill(
    tab: &Arc<dyn niles_browser::Tab>,
    origins: &[String],
    username: Option<&str>,
    password: Option<&str>,
) -> niles_tools::Result<niles_browser::Filled> {
    match tab.fill_login(origins, username, password).await {
        Ok(filled) => Ok(filled),
        Err(niles_browser::Error::Closed) => Err(lapsed(SESSION_ENDED)),
        Err(e) => Err(remote(e)),
    }
}

/// The address's ASCII origin, or "" when it doesn't parse.
pub(super) fn origin_of(address: &str) -> String {
    let Ok(url) = Url::parse(address) else {
        return String::new();
    };
    let origin = url.origin();
    if origin.is_tuple() {
        origin.ascii_serialization()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{self, Approvals, Via};
    use crate::browsing::fake::{self, FakeDriver, FakeTab};
    use crate::browsing::{Create, step, tools, with_store};
    use niles_api::chat::{ApprovalCard, ApprovalStatus, Decision};
    use niles_core::EventBus;
    use niles_logins::{LoginStore, MemoryLogins, NewLogin, Secret};
    use std::sync::Arc;

    const MARK: &str = "person:mark";
    const MAJSE: &str = "person:majse";
    const HER_USER: &str = "mark.scott+tellnoone@example.com";
    const HER_PASSWORD: &str = "correct horse battery staple 42";
    const SHOP: &str = "https://shop.example";
    const LOGIN: &str = "https://shop.example/login";

    struct Harness {
        browsing: Arc<Browsing>,
        store: Arc<MemoryLogins>,
        approvals: Arc<Approvals>,
        driver: Arc<FakeDriver>,
        tool: SignIn,
        login: niles_logins::Login,
    }

    async fn harness() -> Harness {
        let approvals = Arc::new(Approvals::new(EventBus::default(), None));
        let driver = Arc::new(FakeDriver::default());
        let store = Arc::new(MemoryLogins::new());
        let login = save_shop(&store, MARK).await;
        let browsing = with_store(approvals.clone(), driver.clone(), store.clone());
        let tool = SignIn(browsing.clone());
        Harness {
            browsing,
            store,
            approvals,
            driver,
            tool,
            login,
        }
    }

    async fn save_shop(store: &MemoryLogins, person: &str) -> niles_logins::Login {
        let new = NewLogin::new(
            "Shop",
            &[SHOP.to_string()],
            Secret::new(HER_USER),
            Secret::new(HER_PASSWORD),
        )
        .expect("a valid login");
        store.add(person, &new).await.expect("kept")
    }

    async fn save_mail(store: &MemoryLogins, person: &str) -> niles_logins::Login {
        let new = NewLogin::new(
            "Mail",
            &["https://mail.example".to_string()],
            Secret::new("mark@example.com"),
            Secret::new("another password"),
        )
        .expect("a valid login");
        store.add(person, &new).await.expect("kept")
    }

    fn asker() -> Asker {
        Asker {
            person: MARK.into(),
            slug: Some("mark".into()),
            satellite: None,
            conversation: Some(1),
            in_chat: true,
        }
    }

    async fn sign_in(tool: &SignIn, args: Value) -> niles_tools::Result<Value> {
        approvals::ASKER
            .scope(asker(), async { tool.execute(args).await })
            .await
    }

    impl Harness {
        /// A session with a page on it, asking `asks`.
        async fn page(&self, address: &str, asks: Asks) -> Arc<FakeTab> {
            self.browsing
                .session(&asker(), Create::Yes)
                .await
                .expect("a session");
            let tab = self.tab();
            fake::login_page(&tab, address, asks);
            tab
        }

        fn tab(&self) -> Arc<FakeTab> {
            self.driver
                .tabs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .expect("a session was made")
                .clone()
        }

        fn serial(&self) -> u64 {
            self.browsing
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(MARK)
                .expect("the session")
                .serial
        }

        async fn held(&self) -> niles_api::chat::ApprovalCard {
            let pending = self.approvals.pending(MARK);
            assert_eq!(pending.len(), 1, "one card waits");
            pending[0].card.clone()
        }

        /// Call the tool, expect the sign-in card, and answer it once.
        async fn run(&self, args: Value) -> (ApprovalCard, String) {
            let said = sign_in(&self.tool, args).await.expect("held");
            assert_eq!(said["waiting_for_approval"], true, "{said}");
            self.held().await;
            let pending = self.approvals.pending(MARK);
            let id = pending[0].card.id.clone();
            let held = self.approvals.take(&id, MARK).expect("waiting");
            self.approvals
                .settle(held, Decision::Once, Via::App, Some(1))
                .await
        }
    }

    #[tokio::test]
    async fn the_card_shows_the_site_and_the_label_never_the_username() {
        let h = harness().await;
        h.page(LOGIN, Asks::Password { username: true }).await;
        let said = sign_in(&h.tool, json!({ "login_id": h.login.id }))
            .await
            .expect("held");
        assert_eq!(said["shown_to_the_person"], "Sign in to Shop?");
        let card = h.held().await;
        assert_eq!(card.title, "Sign in to Shop?");
        assert_eq!(card.subtitle, "shop.example");
        let fields: Vec<String> = card.fields.iter().map(|f| f.label.clone()).collect();
        assert_eq!(fields, ["Page", "Login", "Sites"]);
        assert!(card.picture);
        let json = serde_json::to_string(&card).expect("serialises");
        assert!(!json.contains(HER_USER), "no username: {json}");
        assert!(!json.contains(HER_PASSWORD), "no password: {json}");
    }

    #[tokio::test]
    async fn another_persons_login_answers_like_one_that_doesnt_exist() {
        let h = harness().await;
        let hers = save_shop(&h.store, MAJSE).await;
        h.page(LOGIN, Asks::Password { username: true }).await;
        let missing = sign_in(&h.tool, json!({ "login_id": 999 }))
            .await
            .expect_err("a missing login")
            .to_string();
        let hers = sign_in(&h.tool, json!({ "login_id": hers.id }))
            .await
            .expect_err("another person's login")
            .to_string();
        // Rule 18: the same answer, the id the only difference.
        let missing = missing.replacen("999", "N", 1);
        let hers = hers.replacen('2', "N", 1);
        assert_eq!(missing, hers, "the same answer either way");
        assert!(hers.contains("No saved login"), "{hers}");
        assert!(h.approvals.pending(MARK).is_empty(), "no card either way");
    }

    #[tokio::test]
    async fn a_page_off_the_logins_sites_gets_no_card() {
        let h = harness().await;
        h.page(
            "https://evil.example/login",
            Asks::Password { username: true },
        )
        .await;
        let said = sign_in(&h.tool, json!({ "login_id": h.login.id }))
            .await
            .expect_err("off the sites");
        assert!(
            said.to_string().contains("isn't on one of Shop's sites"),
            "{said}"
        );
        assert!(h.approvals.pending(MARK).is_empty(), "no card");
    }

    #[tokio::test]
    async fn a_page_with_no_form_and_no_profile_gets_no_card() {
        let h = harness().await;
        h.page(LOGIN, Asks::Nothing).await;
        let said = sign_in(&h.tool, json!({ "login_id": h.login.id }))
            .await
            .expect_err("no form");
        assert!(said.to_string().contains("no sign-in form"), "{said}");
        assert!(h.approvals.pending(MARK).is_empty(), "no card");
    }

    #[tokio::test]
    async fn allowed_it_fills_both_fields_on_the_logins_origin_and_signs_in() {
        let h = harness().await;
        h.page(LOGIN, Asks::Password { username: true }).await;
        let (card, reply) = h.run(json!({ "login_id": h.login.id })).await;
        assert_eq!(card.status, ApprovalStatus::Allowed);
        assert!(reply.contains("Signed in to Shop"), "{reply}");
        {
            let tab = h.tab();
            let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
            assert_eq!(
                state.typed,
                [
                    (
                        LOGIN.to_string(),
                        "username".to_string(),
                        HER_USER.to_string()
                    ),
                    (
                        LOGIN.to_string(),
                        "password".to_string(),
                        HER_PASSWORD.to_string()
                    ),
                ],
                "both fields, once each: {:?}",
                state.typed
            );
            assert!(
                state.done.iter().any(|d| d.starts_with("stay_on")),
                "the guard went on first: {:?}",
                state.done
            );
            assert!(
                state.done.iter().any(|d| d.starts_with("profile")),
                "the profile was kept: {:?}",
                state.done
            );
        }
        assert!(
            h.store
                .profile(MARK, h.login.id)
                .await
                .expect("read")
                .is_some(),
            "the profile is kept"
        );
        assert!(
            h.store
                .get(MARK, h.login.id)
                .await
                .expect("read")
                .is_some_and(|l| l.last_used.is_some()),
            "the login is marked used"
        );
    }

    #[tokio::test]
    async fn a_redirect_before_the_answer_types_nothing() {
        let h = harness().await;
        h.page(LOGIN, Asks::Password { username: true }).await;
        let said = sign_in(&h.tool, json!({ "login_id": h.login.id }))
            .await
            .expect("held");
        assert_eq!(said["waiting_for_approval"], true);
        // The page moved to another site before the answer.
        h.tab()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .address = "https://evil.example/login".into();
        let pending = h.approvals.pending(MARK);
        let id = pending[0].card.id.clone();
        let held = h.approvals.take(&id, MARK).expect("waiting");
        let (card, reply) = h
            .approvals
            .settle(held, Decision::Once, Via::App, Some(1))
            .await;
        assert_eq!(card.status, ApprovalStatus::Lapsed);
        assert!(reply.contains("left the login's sites"), "{reply}");
        let tab = h.tab();
        let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.typed.is_empty(), "nothing typed: {:?}", state.typed);
        assert!(
            !state.done.iter().any(|d| d.starts_with("stay_on")),
            "the guard never went on: {:?}",
            state.done
        );
    }

    #[tokio::test]
    async fn a_redirect_after_the_username_types_no_password_there() {
        for leaky in [false, true] {
            let h = harness().await;
            let tab = h.page(LOGIN, Asks::Username).await;
            tab.state.lock().unwrap_or_else(|e| e.into_inner()).leaky = leaky;
            tab.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .then
                .push((
                    "https://evil.example/pw".to_string(),
                    Asks::Password { username: false },
                ));
            let (card, reply) = h.run(json!({ "login_id": h.login.id })).await;
            assert_eq!(card.status, ApprovalStatus::Failed, "{reply}");
            assert!(reply.contains("typed nothing more there"), "{reply}");
            let tab = h.tab();
            let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
            assert_eq!(
                state.typed,
                [(
                    LOGIN.to_string(),
                    "username".to_string(),
                    HER_USER.to_string()
                )],
                "only the username, at shop: {:?}",
                state.typed
            );
        }
    }

    #[tokio::test]
    async fn a_frame_off_the_origin_isnt_niless_to_fill() {
        let h = harness().await;
        h.page(LOGIN, Asks::Password { username: true }).await;
        h.run(json!({ "login_id": h.login.id })).await;
        let tab = h.tab();
        let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
        for done in &state.done {
            if let Some(rest) = done.strip_prefix("fill_login") {
                assert!(
                    rest.contains(&format!("{:?}", [SHOP.to_string()])),
                    "every fill names the login's origins: {done}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_wrong_password_is_never_typed_twice() {
        let h = harness().await;
        let tab = h.page(LOGIN, Asks::Password { username: true }).await;
        tab.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .then
            .push((
                "https://shop.example/login?again".to_string(),
                Asks::Password { username: true },
            ));
        let (card, reply) = h.run(json!({ "login_id": h.login.id })).await;
        assert_eq!(card.status, ApprovalStatus::Allowed, "{reply}");
        assert!(reply.contains("still asks for the login"), "{reply}");
        let tab = h.tab();
        let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            state
                .typed
                .iter()
                .filter(|(_, field, _)| field == "password")
                .count(),
            1,
            "the password went in once: {:?}",
            state.typed
        );
    }

    #[tokio::test]
    async fn a_code_page_gets_a_code_card() {
        let h = harness().await;
        h.page("https://shop.example/code", Asks::Code).await;
        let since = std::time::Instant::now();
        let (card, reply) = h.run(json!({ "login_id": h.login.id })).await;
        assert_eq!(card.status, ApprovalStatus::Allowed, "{reply}");
        assert!(reply.contains("sent the person a code"), "{reply}");
        let request = h.browsing.code_for_turn(MARK, since).expect("a code card");
        assert_eq!(request.label, "Shop");
        assert_eq!(request.site, SHOP);
    }

    #[tokio::test]
    async fn a_captcha_on_the_page_stops_it_without_a_card() {
        let h = harness().await;
        h.page(LOGIN, Asks::Captcha).await;
        let said = sign_in(&h.tool, json!({ "login_id": h.login.id }))
            .await
            .expect("a note, not an error");
        assert_eq!(said["captcha"], true);
        assert_eq!(said["note"], CAPTCHA_NOTE);
        assert!(h.approvals.pending(MARK).is_empty(), "no card at call time");
    }

    #[tokio::test]
    async fn a_captcha_after_the_password_stops_the_run() {
        let h = harness().await;
        let tab = h.page(LOGIN, Asks::Password { username: true }).await;
        tab.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .then
            .push((LOGIN.to_string(), Asks::Captcha));
        let (card, reply) = h.run(json!({ "login_id": h.login.id })).await;
        assert_eq!(card.status, ApprovalStatus::Failed, "{reply}");
        assert!(reply.contains("wants a person"), "{reply}");
        let tab = h.tab();
        let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            state.typed.len(),
            2,
            "username and password went in before the CAPTCHA: {:?}",
            state.typed
        );
    }

    #[tokio::test]
    async fn a_kept_profile_is_restored_when_the_session_signs_in() {
        let h = harness().await;
        h.store
            .save_profile(MARK, h.login.id, &Secret::new("the-json"))
            .await
            .expect("kept");
        h.page(LOGIN, Asks::Password { username: true }).await;
        let (card, reply) = h.run(json!({ "login_id": h.login.id })).await;
        assert_eq!(card.status, ApprovalStatus::Allowed, "{reply}");
        assert!(reply.contains("with the browser profile"), "{reply}");
        assert_eq!(
            h.tab()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .restored,
            ["the-json".to_string()]
        );
    }

    #[tokio::test]
    async fn a_profile_unused_for_30_days_is_not_restored() {
        let h = harness().await;
        h.store
            .save_profile(MARK, h.login.id, &Secret::new("the-json"))
            .await
            .expect("kept");
        let stale = chrono::Utc::now() - chrono::Duration::days(niles_logins::PROFILE_DAYS + 1);
        h.store.used(MARK, h.login.id, stale).await.expect("noted");
        h.page(LOGIN, Asks::Password { username: true }).await;
        let (card, reply) = h.run(json!({ "login_id": h.login.id })).await;
        assert_eq!(card.status, ApprovalStatus::Allowed, "{reply}");
        assert!(
            reply.contains("Signed in to Shop"),
            "the form filled instead: {reply}"
        );
        assert!(
            h.tab()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .restored
                .is_empty(),
            "nothing was restored"
        );
    }

    #[tokio::test]
    async fn a_session_signed_in_with_one_login_refuses_another() {
        let h = harness().await;
        let mail = save_mail(&h.store, MARK).await;
        h.page(LOGIN, Asks::Password { username: true }).await;
        let (card, reply) = h.run(json!({ "login_id": h.login.id })).await;
        assert_eq!(card.status, ApprovalStatus::Allowed, "{reply}");
        let said = sign_in(&h.tool, json!({ "login_id": mail.id }))
            .await
            .expect_err("one login to a session");
        assert!(
            said.to_string().contains("one login to a session"),
            "{said}"
        );
    }

    #[tokio::test]
    async fn nothing_the_model_the_log_or_tracing_gets_holds_a_saved_value() {
        let captured = Captured::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(captured.clone())
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .finish(),
        );
        let h = harness().await;
        // The page shows the username in its text, the address and a
        // button label, and the password as a field's value; one read
        // fails quoting the password.
        let tab = h
            .page(
                &format!("{SHOP}/account?u=mark.scott%2Btellnoone%40example.com"),
                Asks::Password { username: true },
            )
            .await;
        {
            let mut state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
            state.text =
                format!("Signed in as {HER_USER}\ntextbox \"Password\" = \"{HER_PASSWORD}\"");
            state.elements.insert(1, fake::button(1, 100, HER_USER));
        }
        let mut seen: Vec<String> = Vec::new();
        // An open and two reads: one fails quoting the password.
        let who = asker();
        let opened = approvals::ASKER
            .scope(
                who.clone(),
                tools::OpenPage(h.browsing.clone())
                    .execute(json!({ "url": format!("{SHOP}/account?u=mark.scott%2Btellnoone%40example.com") })),
            )
            .await
            .expect("opens");
        seen.push(opened.to_string());
        // One read then fails, quoting the password.
        h.tab()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .fail_read = Some(HER_PASSWORD.to_string());
        for _ in 0..2 {
            let read = approvals::ASKER
                .scope(
                    who.clone(),
                    tools::ReadPage(h.browsing.clone()).execute(json!({})),
                )
                .await;
            match read {
                Ok(read) => seen.push(read.to_string()),
                Err(e) => seen.push(e.to_string()),
            }
        }
        // A click holds a card; the element label is the username.
        let held = approvals::ASKER
            .scope(
                who.clone(),
                tools::act(
                    &h.browsing,
                    step::ActKind::Click,
                    json!({ "ref": 1, "reason": "look at the account" }),
                ),
            )
            .await
            .expect("held");
        seen.push(held.to_string());
        let pending = h.approvals.pending(MARK);
        seen.push(serde_json::to_string(&pending[0].card).expect("card"));
        h.approvals.lapse(&pending[0].card.id.clone()).await;
        // The sign-in card and its answer.
        let said = sign_in(&h.tool, json!({ "login_id": h.login.id }))
            .await
            .expect("held");
        seen.push(said.to_string());
        let card = h.held().await;
        seen.push(serde_json::to_string(&card).expect("card"));
        let pending = h.approvals.pending(MARK);
        let id = pending[0].card.id.clone();
        let held = h.approvals.take(&id, MARK).expect("waiting");
        let (answered, reply) = h
            .approvals
            .settle(held, Decision::Once, Via::App, Some(1))
            .await;
        seen.push(serde_json::to_string(&answered).expect("card"));
        seen.push(reply.clone());
        // A code card's ask, and entering its code.
        let bound = h.browsing.bound(MARK, h.serial()).expect("signed in");
        let request = h
            .browsing
            .ask_code(MARK, h.serial(), &bound, SHOP.to_string());
        seen.push(serde_json::to_string(&request).expect("card"));
        {
            let tab = h.tab();
            let mut state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
            state.asks = Asks::Code;
        }
        let card = h.browsing.take_code(MARK, &request.id).expect("the card");
        let entered = super::super::code::enter(&h.browsing, card, &Secret::new("493817")).await;
        seen.push(format!("{entered:?}"));
        assert_none_hold_a_saved_value(&seen, &captured);
    }

    /// None of what the model saw, and none of the log, holds a saved
    /// value in any form.
    fn assert_none_hold_a_saved_value(seen: &[String], captured: &Captured) {
        let joined = seen.join("\n");
        let upper_user = HER_USER.to_uppercase();
        assert!(!joined.to_uppercase().contains(&upper_user), "{joined}");
        assert!(!joined.contains(HER_PASSWORD), "{joined}");
        assert!(
            !joined.contains("mark.scott%2Btellnoone%40example.com"),
            "{joined}"
        );
        let logged = String::from_utf8_lossy(&captured.0.lock().unwrap()).into_owned();
        assert!(!logged.to_uppercase().contains(&upper_user), "{logged}");
        assert!(!logged.contains(HER_PASSWORD), "{logged}");
    }

    /// A tracing writer that keeps every line, for log assertions.
    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }
}
