//! The `request_login` tool and the add-login card it leaves with the
//! turn that asked for one.
//!
//! When a page needs a login Niles doesn't have, the model asks the
//! person for one. The values never reach the model: the card's form
//! posts straight to `/me/logins`, and the conversation carries on with
//! Niles's fixed words carrying only the login's id, label and site.
//! Why it works this way is in ARCHITECTURE.md's "Saved logins".

use super::tools::{invalid, note};
use super::{Browsing, NOT_FOR_JOBS, State};
use niles_api::chat::{LoginRequest, LoginRequestStatus, NotAdded};
use niles_logins::{LoginRef, LoginStore, Origin};
use niles_tools::{Tool, ToolDescriptor};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The said line of the turn after a login was added from its card. The
/// web mirrors it, to find the turn the card's answer started.
pub(crate) const ADDED_A_LOGIN: &str = "Added a login";

const TOOL: &str = "request_login";

/// What the model is told when the card is shown: end the turn, say
/// what the card is for, never ask for the values.
const ASKED: &str = "Nothing is saved yet. Under your reply the person has a card with Add login, a form you never see. End your turn now and say in one short sentence what the card is for. Never ask them for the username or the password in the chat. When they have added it, Niles starts your next turn and tells you the login's id, label and site.";

/// What the model is told when the person already saved this site.
const ALREADY: &str = "The person already has a saved login for this site. Niles keeps its username and password: you never see them, and never ask the person for them. To sign in, open the site's sign-in page and call browser_sign_in with the login's id.";

/// What the model is told when there is no store to keep a login in.
pub(super) const NO_LOGINS: &str = "Saved logins need Niles's database and its secrets key, and this Niles doesn't have both. Tell the person in one short sentence.";

/// What the model is told when nobody is signed in to the chat.
pub(super) const NOBODY: &str = "Saved logins belong to a signed-in person, and nobody is signed in to this chat. Tell the person in one short sentence to sign in to the app first.";

/// A card no turn took within the hour belonged to a turn that failed:
/// its entry is dropped, so an old card can never ride a new turn.
const FORGOTTEN_AFTER: Duration = Duration::from_secs(60 * 60);

/// The saved logins' store, and the cards on their way to the turn that
/// keeps them.
pub(crate) struct Logins {
    store: Option<Arc<dyn LoginStore>>,
    asked: Mutex<Vec<Asked>>,
}

/// One card waiting for the turn that asked for it.
#[derive(Clone)]
struct Asked {
    person: String,
    card: LoginRequest,
    made: Instant,
}

impl Logins {
    pub(crate) fn new(store: Option<Arc<dyn LoginStore>>) -> Self {
        Self {
            store,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Asked>> {
        self.asked.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a store keeps the logins: without one the tool is not
    /// offered, and a card has nothing to wait for.
    pub(crate) fn kept(&self) -> bool {
        self.store.is_some()
    }

    /// Keep the new card for the person's next turn. The newest ask
    /// wins: one turn shows one card.
    fn ask(&self, person: &str, label: String, site: &Origin) -> LoginRequest {
        let mut asked = self.lock();
        let now = Instant::now();
        asked.retain(|a| now.duration_since(a.made) < FORGOTTEN_AFTER);
        asked.retain(|a| a.person != person);
        let card = LoginRequest {
            id: crate::approvals::new_id(),
            label,
            site: site.as_str().into(),
            status: LoginRequestStatus::Waiting,
            login: None,
        };
        asked.push(Asked {
            person: person.to_string(),
            card: card.clone(),
            made: now,
        });
        card
    }

    /// The card a turn of `person`'s that began at `since` left asking,
    /// to keep with the turn. It mirrors [`crate::approvals::Approvals::for_turn`],
    /// but takes rather than reads, because the card is kept with the
    /// turn from then on.
    pub(crate) fn for_turn(&self, person: &str, since: Instant) -> Option<LoginRequest> {
        let mut asked = self.lock();
        let mut newest: Option<Asked> = None;
        asked.retain(|a| {
            if a.person == person && a.made >= since {
                if newest.as_ref().is_none_or(|f| a.made > f.made) {
                    newest = Some(a.clone());
                }
                false
            } else {
                true
            }
        });
        newest.map(|a| a.card)
    }

    /// The login a card was answered with, checked against the card's
    /// site. Answers why, never the username or the password.
    pub(crate) async fn added(
        &self,
        person: &str,
        card: &LoginRequest,
        login: i64,
    ) -> Result<LoginRef, NotAdded> {
        let Some(store) = &self.store else {
            return Err(NotAdded::NoSuchLogin);
        };
        let found = match store.get(person, login).await {
            Ok(found) => found,
            Err(e) => {
                tracing::warn!("[logins] {e}");
                return Err(NotAdded::Unavailable);
            }
        };
        let Some(found) = found else {
            return Err(NotAdded::NoSuchLogin);
        };
        let Some(site) = found
            .origins
            .iter()
            .find(|origin| origin.as_str() == card.site)
        else {
            return Err(NotAdded::OtherSite);
        };
        Ok(LoginRef {
            id: found.id,
            label: found.label,
            site: site.clone(),
        })
    }
}

/// The card's login was added through `/me/logins`: check the login is
/// the asker's and names the card's site, mark the card added exactly
/// once, and start the conversation's next turn carrying only the
/// login's id, label and site. Answers Niles's reply.
pub(crate) async fn added_from_card(
    app: Arc<std::sync::OnceLock<crate::dispatch::DispatchCtx>>,
    who: &str,
    speaker: Option<&str>,
    id: i64,
    request: &str,
    login: i64,
) -> Result<String, NotAdded> {
    let ctx = app.get().ok_or(NotAdded::NoSuchConversation)?;
    let log = ctx.chat_log.as_ref().ok_or(NotAdded::NoSuchConversation)?;
    let person = crate::chat_log::person(speaker, who);
    let conversation = log
        .conversation(&person, id)
        .await
        .ok()
        .flatten()
        .ok_or(NotAdded::NoSuchConversation)?;
    let card = conversation
        .turns
        .iter()
        .filter_map(|t| t.login_request.as_ref())
        .find(|card| card.id == request)
        .ok_or(NotAdded::NoSuchRequest)?;
    if card.status != LoginRequestStatus::Waiting {
        return Err(NotAdded::NotWaiting);
    }
    let added = ctx.logins.added(&person, card, login).await?;
    let answered = LoginRequest {
        status: LoginRequestStatus::Added,
        login: Some(login),
        ..card.clone()
    };
    match log.claim_login_request(&person, id, &answered).await {
        Ok(true) => {}
        Ok(false) => return Err(NotAdded::NotWaiting),
        Err(e) => {
            // The conversation went between the read and the claim: the
            // card is answered or gone, and no turn starts twice.
            tracing::warn!("[chat-log] {e}");
            return Err(NotAdded::NotWaiting);
        }
    }
    tracing::info!("[logins] {person} added the login a card asked for");
    let follow = crate::app::CarryOn {
        who: who.to_string(),
        speaker: speaker.map(str::to_string),
        conversation: id,
        turns: conversation
            .turns
            .iter()
            .map(|t| (t.said.clone(), t.reply.clone(), t.by.clone()))
            .collect(),
        person,
    };
    // The next turn runs in a task of its own, which the handler
    // awaits: a phone that locks mid-turn must not leave it unkept.
    tokio::spawn(crate::app::carry_on_with(
        app,
        follow,
        crate::browsing::ADDED_A_LOGIN,
        crate::browsing::added_text(&added),
        format!("Saved your login for {}.", added.label),
    ))
    .await
    .map_err(|e| {
        tracing::error!("[logins] the turn after an added login did not finish: {e}");
        NotAdded::NotWaiting
    })
}

/// The person key of a chat nobody is signed in to.
pub(super) fn nobody() -> String {
    niles_training::person_key(None, niles_api::chat::ANYONE)
}

/// What the turn a card's answer starts says: Niles's fixed words only,
/// carrying `{id, label, site}` and nothing the person typed.
#[must_use]
pub(crate) fn added_text(added: &LoginRef) -> String {
    format!(
        "Added a login. The person saved the login you asked for with request_login: {}. \
         Niles keeps its username and password: you never see them, and never ask the person for \
         them. To sign in, open the site's sign-in page and call browser_sign_in with its id.",
        json!({ "id": added.id, "label": added.label, "site": added.site.as_str() }),
    )
}

/// Ask the person to add a saved login: `request_login`.
pub(super) struct RequestLogin {
    pub(super) browsing: Arc<Browsing>,
    pub(super) logins: Arc<Logins>,
}

/// A person-typed value's error, as the model's argument error: the
/// reason, which never quotes what was typed.
fn refused(e: niles_logins::Error) -> niles_tools::Error {
    match e {
        niles_logins::Error::Invalid { reason, .. } => invalid(TOOL, reason),
        other => invalid(TOOL, other.to_string()),
    }
}

#[async_trait::async_trait]
impl Tool for RequestLogin {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: TOOL.into(),
            description: "Ask the person to add a saved login for a site, when a page needs a login Niles doesn't have. Ask for one site at a time. Answers at once with the login when they already have one for the site. Otherwise Niles shows a card with Add login, a form you never see; never ask for the username or password in the chat. End your turn then: when they have added it, Niles starts your next turn and tells you its id, label and site.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "site": { "type": "string", "title": "Site", "description": "The site's address, such as https://shop.example. Only its origin counts." },
                    "label": { "type": "string", "title": "Label", "description": "What to call the site on the card, such as Shop. At most 60 characters." }
                },
                "required": ["site", "label"],
            }),
        }
    }

    fn offered(&self) -> bool {
        super::tools::in_the_chat().is_some_and(|a| a.person != nobody())
            && self.logins.kept()
            && matches!(self.browsing.state(), State::Ready(_))
    }

    async fn execute(&self, args: Value) -> niles_tools::Result<Value> {
        let Some(asker) = super::tools::in_the_chat() else {
            return Err(invalid(TOOL, NOT_FOR_JOBS));
        };
        if asker.person == nobody() {
            return Ok(note(NOBODY));
        }
        let Some(store) = self.logins.store.clone() else {
            return Ok(note(NO_LOGINS));
        };
        let site = Origin::parse(args["site"].as_str().unwrap_or_default()).map_err(refused)?;
        let label =
            niles_logins::label(args["label"].as_str().unwrap_or_default()).map_err(refused)?;
        let listed = store.list(&asker.person).await.map_err(|e| {
            niles_tools::Error::Remote(format!("saved logins couldn't be read: {e}"))
        })?;
        let refs: Vec<LoginRef> = listed
            .iter()
            .filter(|login| login.origins.contains(&site))
            .map(|login| LoginRef {
                id: login.id,
                label: login.label.clone(),
                site: site.clone(),
            })
            .collect();
        if !refs.is_empty() {
            return Ok(json!({ "saved": true, "logins": refs, "note": ALREADY }));
        }
        let card = self.logins.ask(&asker.person, label, &site);
        // No site and no label: the log is read by everyone signed in,
        // and neither is theirs to know.
        tracing::info!("[logins] {} was asked for a login", asker.person);
        Ok(json!({
            "waiting_for_the_person": true,
            "shown_to_the_person": format!("{} wants a login", card.label),
            "note": ASKED,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{self, Approvals, Asker};
    use crate::browsing::fake::FakeDriver;
    use crate::browsing::{Found, with_driver};
    use niles_api::chat::ANYONE;
    use niles_core::EventBus;
    use niles_logins::{MemoryLogins, NewLogin, Secret};
    use std::net::SocketAddr;

    const MARK: &str = "person:mark";
    const HER_USER: &str = "mark.scott+tellnoone@example.com";
    const HER_PASSWORD: &str = "correct horse battery staple 42";

    struct Harness {
        browsing: Arc<Browsing>,
        logins: Arc<Logins>,
        store: Arc<MemoryLogins>,
        tool: RequestLogin,
    }

    fn harness() -> Harness {
        let browsing = with_driver(
            Arc::new(Approvals::new(EventBus::default(), None)),
            Arc::new(FakeDriver::default()),
        );
        let store = Arc::new(MemoryLogins::new());
        let logins = Arc::new(Logins::new(Some(store.clone())));
        let tool = RequestLogin {
            browsing: browsing.clone(),
            logins: logins.clone(),
        };
        Harness {
            browsing,
            logins,
            store,
            tool,
        }
    }

    fn asker(satellite: Option<SocketAddr>, in_chat: bool) -> Asker {
        Asker {
            person: MARK.into(),
            slug: Some("mark".into()),
            satellite,
            conversation: Some(1),
            in_chat,
        }
    }

    /// A browsing with nothing found: browsing isn't set up.
    fn no_browser(approvals: Arc<Approvals>) -> Arc<Browsing> {
        crate::browsing::built(approvals, Found::Nothing, None)
    }

    async fn ask(tool: &RequestLogin, args: Value) -> niles_tools::Result<Value> {
        approvals::ASKER
            .scope(asker(None, true), async { tool.execute(args).await })
            .await
    }

    async fn save_shop(store: &MemoryLogins, person: &str) -> niles_logins::Login {
        let new = NewLogin::new(
            "Shop",
            &["https://shop.example".to_string()],
            Secret::new(HER_USER),
            Secret::new(HER_PASSWORD),
        )
        .expect("a valid login");
        store.add(person, &new).await.expect("kept")
    }

    #[tokio::test]
    async fn asks_with_a_card_and_tells_the_model_to_end_its_turn() {
        let h = harness();
        let said = ask(
            &h.tool,
            json!({ "site": "shop.example/login?next=/basket", "label": "Shop" }),
        )
        .await
        .expect("asked");
        assert_eq!(said["waiting_for_the_person"], true);
        assert_eq!(said["shown_to_the_person"], "Shop wants a login");
        assert!(
            said["note"]
                .as_str()
                .is_some_and(|n| n.contains("End your turn")),
            "{said}"
        );
        let card = h
            .logins
            .for_turn(MARK, Instant::now() - Duration::from_secs(1))
            .expect("the turn's card");
        assert_eq!(card.site, "https://shop.example");
        assert_eq!(card.label, "Shop");
        assert_eq!(card.status, LoginRequestStatus::Waiting);
        assert_eq!(
            h.logins
                .for_turn(MARK, Instant::now() - Duration::from_secs(1)),
            None,
            "taken: the card goes with the one turn"
        );
    }

    #[tokio::test]
    async fn an_international_site_is_asked_in_its_xn_form() {
        let h = harness();
        ask(
            &h.tool,
            json!({ "site": "https://bücher.example", "label": "Bücher" }),
        )
        .await
        .expect("asked");
        let card = h
            .logins
            .for_turn(MARK, Instant::now() - Duration::from_secs(1))
            .expect("the turn's card");
        assert_eq!(card.site, "https://xn--bcher-kva.example");
    }

    #[tokio::test]
    async fn a_saved_login_is_answered_at_once_without_a_card() {
        let h = harness();
        let kept = save_shop(&h.store, MARK).await;
        let said = ask(&h.tool, json!({ "site": "shop.example", "label": "Shop" }))
            .await
            .expect("answered");
        assert_eq!(said["saved"], true);
        let text = said.to_string();
        assert!(
            text.contains(&format!("\"id\":{}", kept.id)),
            "the id is the model's to know: {text}"
        );
        assert!(text.contains("\"site\":\"https://shop.example\""), "{text}");
        assert!(!text.contains(HER_USER), "no username: {text}");
        assert!(!text.contains(HER_PASSWORD), "no password: {text}");
        assert_eq!(
            h.logins
                .for_turn(MARK, Instant::now() - Duration::from_secs(1)),
            None,
            "no card: nothing was asked"
        );
    }

    #[tokio::test]
    async fn a_site_off_the_public_internet_or_another_scheme_is_refused() {
        let h = harness();
        for site in [
            "https://192.168.1.10",
            "ftp://shop.example",
            "https://shop.example:8443",
        ] {
            let err = ask(&h.tool, json!({ "site": site, "label": "Shop" }))
                .await
                .expect_err(site);
            assert!(
                matches!(err, niles_tools::Error::InvalidArgs { .. }),
                "{site}: {err}"
            );
        }
        assert_eq!(
            h.logins
                .for_turn(MARK, Instant::now() - Duration::from_secs(1)),
            None,
            "a refused ask leaves no card"
        );
    }

    #[tokio::test]
    async fn the_newest_ask_wins_within_a_turn() {
        let h = harness();
        ask(
            &h.tool,
            json!({ "site": "https://one.example", "label": "One" }),
        )
        .await
        .expect("asked");
        ask(
            &h.tool,
            json!({ "site": "https://two.example", "label": "Two" }),
        )
        .await
        .expect("asked");
        let card = h
            .logins
            .for_turn(MARK, Instant::now() - Duration::from_secs(1))
            .expect("the turn's card");
        assert_eq!(card.label, "Two");
        assert_eq!(card.site, "https://two.example");
    }

    #[tokio::test]
    async fn offered_only_in_the_chat_with_a_store_and_a_browser() {
        let h = harness();
        // No ASKER scope: not in a chat at all.
        assert!(!h.tool.offered());
        // A satellite asker, or one outside the chat: not the app's chat.
        for asker in [
            asker(Some("10.0.0.1:80".parse().expect("an address")), false),
            asker(None, false),
        ] {
            let offered = approvals::ASKER
                .scope(asker, async { h.tool.offered() })
                .await;
            assert!(!offered, "only the app's chat is offered the tool");
        }
        // Nobody signed in.
        let nobody = approvals::ASKER
            .scope(
                Asker {
                    person: format!("email:{ANYONE}"),
                    slug: None,
                    satellite: None,
                    conversation: None,
                    in_chat: true,
                },
                async { h.tool.offered() },
            )
            .await;
        assert!(!nobody, "a chat nobody is signed in to gets no tool");
        // No store, and no browser.
        let no_store = RequestLogin {
            browsing: h.browsing.clone(),
            logins: Arc::new(Logins::new(None)),
        };
        let offered = approvals::ASKER
            .scope(asker(None, true), async { no_store.offered() })
            .await;
        assert!(!offered, "no store, no tool");
        let off = RequestLogin {
            browsing: no_browser(Arc::new(Approvals::new(EventBus::default(), None))),
            logins: h.logins.clone(),
        };
        let offered = approvals::ASKER
            .scope(asker(None, true), async { off.offered() })
            .await;
        assert!(!offered, "no browser, no tool");
        // Everything in place: offered.
        let offered = approvals::ASKER
            .scope(asker(None, true), async { h.tool.offered() })
            .await;
        assert!(offered, "the chat, a store and a browser: offered");
    }

    #[tokio::test]
    async fn nobody_signed_in_is_told_to_sign_in() {
        let h = harness();
        let said = approvals::ASKER
            .scope(
                Asker {
                    person: format!("email:{ANYONE}"),
                    slug: None,
                    satellite: None,
                    conversation: None,
                    in_chat: true,
                },
                async {
                    h.tool
                        .execute(json!({ "site": "shop.example", "label": "Shop" }))
                        .await
                },
            )
            .await
            .expect("a note, not an error");
        assert_eq!(said["note"], NOBODY);
    }

    #[tokio::test]
    async fn the_next_turn_and_the_kept_card_hold_no_secret() {
        let h = harness();
        let kept = save_shop(&h.store, MARK).await;
        let card = h.logins.ask(
            MARK,
            "Shop".into(),
            &Origin::parse("https://shop.example").expect("a site"),
        );
        let added = h
            .logins
            .added(MARK, &card, kept.id)
            .await
            .expect("the login the card asked for");
        assert_eq!(added.id, kept.id);
        let text = added_text(&added);
        assert!(text.contains(&format!("\"id\":{}", kept.id)), "{text}");
        assert!(text.contains("Shop"), "{text}");
        assert!(text.contains("https://shop.example"), "{text}");
        assert!(!text.contains(HER_USER), "no username: {text}");
        assert!(!text.contains(HER_PASSWORD), "no password: {text}");
        let kept_card = LoginRequest {
            status: LoginRequestStatus::Added,
            login: Some(kept.id),
            ..card
        };
        let json = serde_json::to_string(&kept_card).expect("serialises");
        assert!(!json.contains(HER_USER), "no username: {json}");
        assert!(!json.contains(HER_PASSWORD), "no password: {json}");
    }

    #[tokio::test]
    async fn added_refuses_another_persons_login_and_one_for_another_site() {
        let h = harness();
        let hers = save_shop(&h.store, "person:majse").await;
        let card = h.logins.ask(
            MARK,
            "Shop".into(),
            &Origin::parse("https://shop.example").expect("a site"),
        );
        assert_eq!(
            h.logins.added(MARK, &card, hers.id).await,
            Err(NotAdded::NoSuchLogin),
            "her login is not his"
        );
        let kept = save_shop(&h.store, MARK).await;
        let elsewhere = h.logins.ask(
            MARK,
            "Mail".into(),
            &Origin::parse("https://mail.example").expect("a site"),
        );
        assert_eq!(
            h.logins.added(MARK, &elsewhere, kept.id).await,
            Err(NotAdded::OtherSite),
            "the login doesn't name the card's site"
        );
    }

    #[test]
    fn the_added_text_brings_the_browser_tools() {
        let added = LoginRef {
            id: 7,
            label: "Shop".into(),
            site: Origin::parse("https://shop.example").expect("a site"),
        };
        let names =
            niles_tools::relevance::relevant_tool_names(&added_text(&added)).expect("classified");
        assert!(
            names
                .iter()
                .any(|n| n == &"request_login" || n.starts_with("browser_")),
            "the browser group goes with the turn: {names:?}"
        );
    }
}
