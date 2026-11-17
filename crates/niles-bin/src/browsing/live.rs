//! The live view (NIL-82): the person watches their session's page as
//! it changes, takes the browser over, and gives it back.
//!
//! While the person has the browser, Niles doesn't act on it: every
//! tool that would touch the page answers [`PERSON_HAS_IT`]. A step
//! waiting on its card and a code card lapse when they take it over.
//! What they see and do never reaches the model, the log or a turn;
//! the model learns only the page it reads once the browser is back.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use niles_api::browser::{Control, Go, LiveView, Refused};
use niles_api::chat::Handoff;
use niles_browser::{Frame, Input, VIEW_HEIGHT, VIEW_WIDTH};
use tokio::sync::watch;

use super::Browsing;

/// What a browsing tool answers while the person has the browser.
pub(crate) const PERSON_HAS_IT: &str = "The person has the browser now: they took it over, or Niles handed it to them. Don't use the browser. End your turn now; Niles carries on when they give it back.";

/// The browser goes back to Niles after this long with no input from
/// the person who has it.
pub(super) const HOLD: Duration = Duration::from_secs(15 * 60);

/// The least time between two pictures: slow while the person only
/// watches, quick while they work the page.
const WATCHING: u64 = 500;
const WORKING: u64 = 100;

/// The longest reason a handoff card shows.
pub(super) const MAX_WHY: usize = 200;

/// A session's live side: who has the browser, and who watches it.
pub(super) struct Live {
    pub(super) control: Control,
    /// When the person last did something, while they have it.
    pub(super) last_input: Instant,
    /// Niles's handoff, while the person has the browser for it, and
    /// when it was made: the turn it belongs to is found by that.
    pub(super) handoff: Option<(Handoff, Instant)>,
    viewers: usize,
    pace: Arc<AtomicU64>,
    frames: Option<watch::Receiver<Option<Frame>>>,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            control: Control::Niles,
            last_input: Instant::now(),
            handoff: None,
            viewers: 0,
            pace: Arc::new(AtomicU64::new(WATCHING)),
            frames: None,
        }
    }
}

impl Live {
    fn to(&mut self, control: Control) {
        self.control = control;
        self.last_input = Instant::now();
        let pace = match control {
            Control::Niles => WATCHING,
            Control::Person => WORKING,
        };
        self.pace.store(pace, Ordering::Relaxed);
        if control == Control::Niles {
            self.handoff = None;
        }
    }

    /// Whether the person has the browser and hasn't let it lapse.
    pub(super) fn held(&self, now: Instant) -> bool {
        self.control == Control::Person && now.duration_since(self.last_input) < HOLD
    }
}

/// One live view's pictures. Dropping it stops the pictures when it
/// was the last one watching.
pub(crate) struct Viewer {
    frames: watch::Receiver<Option<Frame>>,
    browsing: Arc<Browsing>,
    person: String,
    serial: u64,
}

impl Viewer {
    /// The next picture; `None` once the page or the view ended.
    pub(crate) async fn next(&mut self) -> Option<Arc<[u8]>> {
        loop {
            self.frames.changed().await.ok()?;
            if let Some(frame) = self.frames.borrow_and_update().as_ref() {
                return Some(frame.jpeg.clone());
            }
        }
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        let browsing = self.browsing.clone();
        let person = std::mem::take(&mut self.person);
        let serial = self.serial;
        tokio::spawn(async move { browsing.unwatch(&person, serial).await });
    }
}

impl Browsing {
    /// Tell the person's app their session changed.
    pub(super) fn changed(&self, person: &str) {
        self.bus.publish(niles_core::Event::BrowserChanged {
            person: person.to_string(),
        });
    }

    /// The person's session as the app shows it, if one is open.
    pub(crate) async fn live_view(&self, person: &str) -> Option<LiveView> {
        let (tab, control, handoff) = {
            let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let open = sessions.get(person)?;
            (
                open.tab.clone(),
                open.live.control,
                open.live.handoff.as_ref().map(|(h, _)| h.clone()),
            )
        };
        let address = tab.address().await.unwrap_or_default();
        // The address is the page's own and goes to its person only;
        // their saved values are taken out as from anything else.
        let address = match self.scrub(person).await {
            Ok(scrub) => scrub.text(&address),
            Err(_) => String::new(),
        };
        Some(LiveView {
            address,
            control,
            handoff,
            width: VIEW_WIDTH,
            height: VIEW_HEIGHT,
        })
    }

    /// The person takes their browser over: Niles stops, and what waits
    /// on a card of this session lapses.
    pub(crate) async fn take_over(&self, person: &str) -> Result<(), Refused> {
        let (holding, code) = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let open = sessions.get_mut(person).ok_or(Refused::NoSession)?;
            open.live.to(Control::Person);
            open.used = Instant::now();
            (open.holding.take(), open.code.take())
        };
        if let Some(id) = holding {
            self.approvals.lapse(&id).await;
        }
        if let Some(id) = code {
            self.drop_code(&id);
        }
        tracing::info!("[browser] {person} took the browser over");
        self.changed(person);
        Ok(())
    }

    /// Niles hands the person their browser, for `why`, on `site`: a
    /// CAPTCHA, or something the model can't do. The card rides on the
    /// turn that asked.
    pub(super) fn hand_over(&self, person: &str, serial: u64, why: String, site: String) {
        {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let Some(open) = sessions.get_mut(person) else {
                return;
            };
            if open.serial != serial {
                return;
            }
            open.live.to(Control::Person);
            open.live.handoff = Some((
                Handoff {
                    id: crate::approvals::new_id(),
                    site,
                    why,
                },
                Instant::now(),
            ));
            open.used = Instant::now();
        }
        tracing::info!("[browser] Niles handed {person} the browser");
        self.changed(person);
    }

    /// The handoff Niles made at or after `since`, for that turn's card.
    pub(crate) fn handoff_for_turn(&self, person: &str, since: Instant) -> Option<Handoff> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let (handoff, made) = sessions.get(person)?.live.handoff.as_ref()?;
        (*made >= since).then(|| handoff.clone())
    }

    /// The person gives the browser back. Refused when they don't have
    /// it, so a second tap starts no second turn.
    pub(crate) fn give_back(&self, person: &str) -> Result<(), Refused> {
        {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let open = sessions.get_mut(person).ok_or(Refused::NoSession)?;
            if open.live.control != Control::Person {
                return Err(Refused::NotInControl);
            }
            open.live.to(Control::Niles);
            open.used = Instant::now();
        }
        tracing::info!("[browser] {person} gave the browser back");
        self.changed(person);
        Ok(())
    }

    /// Whether the person has their browser now.
    pub(super) fn held_by_person(&self, person: &str) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(person)
            .is_some_and(|open| open.live.control == Control::Person)
    }

    /// The person's session's tab and its lock, while they have it.
    fn theirs(&self, person: &str) -> Result<super::Opened, Refused> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let open = sessions.get_mut(person).ok_or(Refused::NoSession)?;
        if open.live.control != Control::Person {
            return Err(Refused::NotInControl);
        }
        let now = Instant::now();
        open.live.last_input = now;
        open.used = now;
        Ok(open.opened())
    }

    /// One input from the person's live view, as its JSON text. Never
    /// logged: it can be what they type into a page.
    pub(crate) async fn input(&self, person: &str, text: &str) -> Result<(), Refused> {
        let input: Input = serde_json::from_str(text).map_err(|_| Refused::NotInput)?;
        if !input.checked() {
            return Err(Refused::NotInput);
        }
        let opened = self.theirs(person)?;
        let _tab = opened.lock.lock().await;
        opened.tab.input(&input).await.map_err(|_| Refused::Gone)
    }

    /// Back or Reload, while the person has the browser.
    pub(crate) async fn go(&self, person: &str, go: Go) -> Result<(), Refused> {
        let opened = self.theirs(person)?;
        let _tab = opened.lock.lock().await;
        let done = match go {
            Go::Back => opened.tab.back().await,
            Go::Reload => opened.tab.reload().await,
        };
        done.map_err(|_| Refused::Gone)
    }

    /// The pictures of the person's session, starting them for the
    /// first one watching.
    pub(crate) async fn watch_page(self: &Arc<Self>, person: &str) -> Option<Viewer> {
        let (tab, serial, pace, running) = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let open = sessions.get_mut(person)?;
            open.live.viewers += 1;
            (
                open.tab.clone(),
                open.serial,
                open.live.pace.clone(),
                open.live.frames.clone(),
            )
        };
        let frames = match running {
            Some(frames) => frames,
            None => match tab.screencast(pace).await {
                Ok(frames) => {
                    let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(open) = sessions.get_mut(person)
                        && open.serial == serial
                    {
                        open.live.frames = Some(frames.clone());
                    }
                    frames
                }
                Err(_) => {
                    self.unwatch(person, serial).await;
                    return None;
                }
            },
        };
        let mut frames = frames;
        // The newest picture counts as unseen, so a viewer that joins a
        // running view gets one at once.
        frames.mark_changed();
        Some(Viewer {
            frames,
            browsing: self.clone(),
            person: person.to_string(),
            serial,
        })
    }

    /// One viewer fewer; the last one stops the pictures.
    async fn unwatch(&self, person: &str, serial: u64) {
        let stop = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let Some(open) = sessions.get_mut(person) else {
                return;
            };
            if open.serial != serial {
                return;
            }
            open.live.viewers = open.live.viewers.saturating_sub(1);
            (open.live.viewers == 0)
                .then(|| open.live.frames.take().map(|_| open.tab.clone()))
                .flatten()
        };
        if let Some(tab) = stop {
            tab.stop_screencast().await;
        }
    }

    /// Give the browser back to Niles where the person let it sit: no
    /// new turn, and their app is told.
    pub(super) fn lapse_control(&self, now: Instant) {
        let lapsed: Vec<String> = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions
                .iter_mut()
                .filter(|(_, open)| open.live.control == Control::Person && !open.live.held(now))
                .map(|(person, open)| {
                    open.live.to(Control::Niles);
                    open.used = now;
                    person.clone()
                })
                .collect()
        };
        for person in lapsed {
            tracing::info!("[browser] {person}'s hold on the browser lapsed");
            self.changed(&person);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{ASKER, Approvals, Asker};
    use crate::browsing::fake::{self, FakeDriver, FakeTab};
    use crate::browsing::handover::HandOver;
    use crate::browsing::step::ActKind;
    use crate::browsing::tools::{OpenPage, ReadPage, act};
    use niles_tools::Tool;
    use serde_json::{Value, json};

    const MARK: &str = "person:mark";
    const MAJSE: &str = "person:majse";
    const SHOP: &str = "https://shop.example";
    const CLICK: &str = r#"{"type": "press", "x": 10, "y": 20, "press": "down", "button": "left"}"#;

    struct Harness {
        browsing: Arc<Browsing>,
        approvals: Arc<Approvals>,
        driver: Arc<FakeDriver>,
    }

    fn harness() -> Harness {
        let approvals = Arc::new(Approvals::new(niles_core::EventBus::default(), None));
        let driver = Arc::new(FakeDriver::default());
        let browsing = crate::browsing::with_driver(approvals.clone(), driver.clone());
        Harness {
            browsing,
            approvals,
            driver,
        }
    }

    fn mark() -> Asker {
        Asker {
            person: MARK.into(),
            slug: Some("mark".into()),
            satellite: None,
            conversation: Some(1),
            in_chat: true,
        }
    }

    impl Harness {
        async fn as_mark<R>(&self, f: impl std::future::Future<Output = R>) -> R {
            ASKER.scope(mark(), f).await
        }

        async fn open(&self) {
            self.as_mark(OpenPage(self.browsing.clone()).execute(json!({ "url": SHOP })))
                .await
                .expect("opens");
        }

        async fn read(&self) -> Value {
            self.as_mark(ReadPage(self.browsing.clone()).execute(json!({})))
                .await
                .expect("answers")
        }

        async fn click(&self) -> Value {
            self.as_mark(act(
                &self.browsing,
                ActKind::Click,
                json!({ "ref": 1, "reason": "put it in the basket" }),
            ))
            .await
            .expect("answers")
        }

        async fn hand_over(&self, reason: Value) -> niles_tools::Result<Value> {
            self.as_mark(HandOver(self.browsing.clone()).execute(json!({ "reason": reason })))
                .await
        }

        fn tab(&self) -> Arc<FakeTab> {
            self.driver
                .tabs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .expect("a session")
                .clone()
        }

        fn control(&self) -> Control {
            self.browsing
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(MARK)
                .expect("a session")
                .live
                .control
        }
    }

    #[tokio::test]
    async fn taking_over_keeps_niles_off_and_lapses_a_waiting_step() {
        let h = harness();
        h.open().await;
        h.tab()
            .state()
            .elements
            .insert(1, fake::button(1, 100, "Add to basket"));
        h.click().await;
        assert_eq!(h.approvals.pending(MARK).len(), 1, "held on its card");

        h.browsing.take_over(MARK).await.expect("taken");
        assert!(h.approvals.pending(MARK).is_empty(), "the step lapsed");
        assert_eq!(h.read().await["note"], PERSON_HAS_IT);
        assert_eq!(h.click().await["note"], PERSON_HAS_IT);
        let reopened = h
            .as_mark(
                OpenPage(h.browsing.clone()).execute(json!({ "url": SHOP, "new_session": true })),
            )
            .await
            .expect("a note");
        assert_eq!(reopened["note"], PERSON_HAS_IT, "their session stays");
        assert!(!h.tab().state().closed);
    }

    #[tokio::test]
    async fn input_reaches_the_page_only_from_the_person_who_has_it() {
        let h = harness();
        h.open().await;
        assert_eq!(
            h.browsing.input(MARK, CLICK).await,
            Err(Refused::NotInControl),
            "Niles has it"
        );
        h.browsing.take_over(MARK).await.expect("taken");
        h.browsing.input(MARK, CLICK).await.expect("passed on");
        assert_eq!(
            h.browsing.input(MAJSE, CLICK).await,
            Err(Refused::NoSession),
            "nobody else's"
        );
        for refused in [
            "not json",
            r#"{"type": "move", "x": 5000, "y": 1}"#,
            r#"{"type": "key", "press": "down", "key": "F12"}"#,
        ] {
            assert_eq!(
                h.browsing.input(MARK, refused).await,
                Err(Refused::NotInput),
                "{refused}"
            );
        }
        assert_eq!(h.tab().state().inputs.len(), 1, "only the click arrived");
        h.browsing.go(MARK, Go::Back).await.expect("back");
        assert_eq!(h.tab().state().moved, ["back"]);
    }

    #[tokio::test]
    async fn the_browser_comes_back_once() {
        let h = harness();
        h.open().await;
        assert_eq!(h.browsing.give_back(MARK), Err(Refused::NotInControl));
        h.browsing.take_over(MARK).await.expect("taken");
        assert_eq!(h.control(), Control::Person);
        h.browsing.give_back(MARK).expect("given back");
        assert_eq!(h.control(), Control::Niles);
        assert_eq!(
            h.browsing.give_back(MARK),
            Err(Refused::NotInControl),
            "a second tap starts nothing"
        );
        assert_ne!(h.read().await["note"], PERSON_HAS_IT, "Niles reads again");
    }

    #[tokio::test]
    async fn every_viewer_shares_one_stream_and_the_last_stops_it() {
        let h = harness();
        h.open().await;
        assert!(
            h.browsing.watch_page(MAJSE).await.is_none(),
            "nobody else's"
        );
        let mut first = h.browsing.watch_page(MARK).await.expect("watching");
        let second = h.browsing.watch_page(MARK).await.expect("watching");
        assert_eq!(h.tab().state().casts, 1, "one stream for both");
        let picture = first.next().await.expect("a picture");
        assert_eq!(&picture[..2], &[0xFF, 0xD8]);
        drop(first);
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(h.tab().state().uncasts, 0, "one still watches");
        drop(second);
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(h.tab().state().uncasts, 1, "the last one stopped it");
    }

    #[tokio::test]
    async fn a_hold_left_alone_goes_back_to_niles_and_the_session_stays() {
        let h = harness();
        h.open().await;
        let mut told = h.browsing.bus.subscribe();
        h.browsing.take_over(MARK).await.expect("taken");
        assert!(matches!(
            told.recv().await,
            Ok(niles_core::Event::BrowserChanged { person }) if person == MARK
        ));
        h.browsing
            .tend(Instant::now() + HOLD + Duration::from_secs(1))
            .await;
        assert_eq!(h.control(), Control::Niles, "lapsed");
        assert!(!h.tab().state().closed, "lapsed, not reaped");
        assert!(matches!(
            told.recv().await,
            Ok(niles_core::Event::BrowserChanged { .. })
        ));
    }

    #[tokio::test]
    async fn the_model_hands_over_with_a_reason_the_card_shows() {
        let h = harness();
        let none = h.hand_over(json!("Scan the code")).await.expect("a note");
        assert!(none.get("handed_over").is_none(), "nothing open yet");
        h.open().await;
        let since = Instant::now();
        assert!(h.hand_over(json!("x".repeat(MAX_WHY + 1))).await.is_err());
        let handed = h
            .hand_over(json!("Scan the QR code with your banking app."))
            .await
            .expect("handed over");
        assert_eq!(handed["handed_over"], true);
        let card = h.browsing.handoff_for_turn(MARK, since).expect("a card");
        assert_eq!(card.why, "Scan the QR code with your banking app.");
        assert_eq!(card.site, SHOP);
        let view = h.browsing.live_view(MARK).await.expect("open");
        assert_eq!(view.control, Control::Person);
        assert_eq!(view.handoff, Some(card));
        h.browsing.give_back(MARK).expect("given back");
        let view = h.browsing.live_view(MARK).await.expect("open");
        assert_eq!(view.handoff, None, "the card is done");
    }

    #[tokio::test]
    async fn a_voice_never_gets_the_hand_over_tool() {
        let h = harness();
        let voice = Asker {
            satellite: Some("10.0.0.9:10700".parse().expect("an address")),
            in_chat: false,
            ..mark()
        };
        let offered = ASKER
            .scope(voice, async { HandOver(h.browsing.clone()).offered() })
            .await;
        assert!(!offered);
    }
}
