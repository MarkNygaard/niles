//! Steps a schedule was allowed, when its person made it, to take on its
//! own (NIL-87): clicks, choices and submits on one saved login's sites,
//! and signing in with that login, during that schedule's own runs.
//!
//! Everything else still waits on its card: typing, a step on another
//! site, a session signed in with another login, an order and a page
//! asking for card details. Each step taken this way goes into Approvals'
//! history, marked as allowed by its schedule.

use std::sync::Arc;

use niles_api::chat::{ApprovalCard, CardField};
use niles_browser::Checkout;

use super::step::Action;
use super::{Browsing, Opened};
use crate::approvals::Asker;
use crate::schedules::{Allowance, allowance};

/// The schedule's permission, when this step falls within it: this run is
/// the asker's schedule's, the session is on the allowed login's sites
/// and signed in with no other login, and the step is no order and no
/// card-details page.
pub(super) async fn covers(
    browsing: &Browsing,
    asker: &Asker,
    opened: &Opened,
    login: Option<i64>,
    page: &Checkout,
) -> Option<Allowance> {
    let allows = allowance()?;
    if allows.person != asker.person || page.at_checkout || page.card_details {
        return None;
    }
    // Signing in: only with the allowed login itself.
    if login.is_some_and(|id| id != allows.login) {
        return None;
    }
    if let Some(bound) = browsing.bound(&asker.person, opened.serial)
        && bound.login != allows.login
    {
        return None;
    }
    let saved = browsing
        .store()?
        .get(&asker.person, allows.login)
        .await
        .ok()??;
    let origins: Vec<String> = saved
        .origins
        .iter()
        .map(|o| o.as_str().to_string())
        .collect();
    let address = opened.tab.address().await.ok()?;
    niles_browser::on_sites(&address, &origins).then_some(allows)
}

/// Whether `action` is a kind a schedule may take without asking: never
/// typing.
pub(super) fn kind_allowed(action: &Action) -> bool {
    matches!(
        action,
        Action::Click | Action::Choose { .. } | Action::Submit
    )
}

/// The card the step would have waited on, with the schedule named:
/// what Approvals' history keeps of it.
pub(super) fn marked(mut card: ApprovalCard, allows: &Allowance) -> ApprovalCard {
    card.fields.insert(
        0,
        CardField {
            label: "Schedule".into(),
            value: allows.name.clone(),
        },
    );
    card
}

/// Keep what was done in Approvals' history, as allowed by the schedule.
pub(super) async fn record(
    browsing: &Arc<Browsing>,
    asker: &Asker,
    card: &ApprovalCard,
    outcome: Result<(), String>,
) {
    browsing
        .approvals
        .allowed_by_schedule(asker, "Browser", card, outcome)
        .await;
}

/// What the model is told of a step its schedule took without asking.
pub(super) fn said(allows: &Allowance, what: &str) -> String {
    format!(
        "{what} The schedule \"{}\" may act on this site without asking, so it ran with no card.",
        allows.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{ASKER, Approvals, Ledger};
    use crate::browsing::fake::{self, FakeDriver, FakeTab};
    use crate::browsing::step::ActKind;
    use crate::browsing::tools::{OpenPage, act};
    use crate::schedules::RUN_ALLOWS;
    use niles_db::{ApprovalEntry, ApprovalLogged};
    use niles_logins::{LoginStore, MemoryLogins, NewLogin, Secret};
    use niles_tools::Tool;
    use serde_json::{Value, json};
    use std::sync::Mutex;

    const MARK: &str = "person:mark";
    const SHOP: &str = "https://shop.example";

    /// What Approvals' history keeps, in memory.
    #[derive(Default)]
    struct Kept(Mutex<Vec<ApprovalEntry>>);

    #[async_trait::async_trait]
    impl Ledger for Kept {
        async fn asked(&self, entry: &ApprovalEntry) -> Result<(), String> {
            self.0.lock().unwrap().push(entry.clone());
            Ok(())
        }
        async fn settled(&self, entry: &ApprovalEntry) -> Result<(), String> {
            let mut rows = self.0.lock().unwrap();
            if let Some(row) = rows.iter_mut().find(|r| r.approval == entry.approval) {
                *row = entry.clone();
            }
            Ok(())
        }
        async fn page(
            &self,
            _person: &str,
            _before: Option<i64>,
            _limit: i64,
        ) -> Result<Vec<ApprovalLogged>, String> {
            Ok(Vec::new())
        }
    }

    struct Harness {
        browsing: Arc<Browsing>,
        approvals: Arc<Approvals>,
        driver: Arc<FakeDriver>,
        kept: Arc<Kept>,
        login: i64,
    }

    async fn harness() -> Harness {
        let kept = Arc::new(Kept::default());
        let approvals = Arc::new(Approvals::new(
            niles_core::EventBus::default(),
            Some(kept.clone() as Arc<dyn Ledger>),
        ));
        let driver = Arc::new(FakeDriver::default());
        let store = Arc::new(MemoryLogins::new());
        let new = NewLogin::new(
            "Gym",
            &[SHOP.to_string()],
            Secret::new("mark@example.com"),
            Secret::new("a long gym password"),
        )
        .expect("a valid login");
        let login = store.add(MARK, &new).await.expect("kept").id;
        let browsing = crate::browsing::with_store(approvals.clone(), driver.clone(), store);
        Harness {
            browsing,
            approvals,
            driver,
            kept,
            login,
        }
    }

    fn mark() -> Asker {
        Asker {
            person: MARK.into(),
            slug: Some("mark".into()),
            satellite: None,
            conversation: Some(7),
            in_chat: true,
        }
    }

    fn allowed_on(person: &str, login: i64) -> Allowance {
        Allowance {
            person: person.into(),
            schedule: 3,
            name: "Book yoga".into(),
            login,
        }
    }

    impl Harness {
        async fn run<R>(&self, allows: Option<Allowance>, f: impl Future<Output = R>) -> R {
            RUN_ALLOWS.scope(allows, ASKER.scope(mark(), f)).await
        }

        async fn open(&self, address: &str) {
            self.run(
                None,
                OpenPage(self.browsing.clone()).execute(json!({ "url": address })),
            )
            .await
            .expect("opens");
        }

        fn tab(&self) -> Arc<FakeTab> {
            self.driver
                .tabs
                .lock()
                .unwrap()
                .last()
                .expect("a tab")
                .clone()
        }

        async fn step(&self, allows: Option<Allowance>, kind: ActKind, args: Value) -> Value {
            self.run(allows, act(&self.browsing, kind, args))
                .await
                .expect("answers")
        }

        async fn click(&self, allows: Option<Allowance>) -> Value {
            self.step(
                allows,
                ActKind::Click,
                json!({ "ref": 1, "reason": "book the class" }),
            )
            .await
        }
    }

    async fn on_the_book_page(h: &Harness, address: &str) {
        h.open(address).await;
        h.tab()
            .state()
            .elements
            .insert(1, fake::button(1, 100, "Book"));
    }

    #[tokio::test]
    async fn a_schedules_run_books_on_its_logins_site_with_no_card_and_keeps_it() {
        let h = harness().await;
        on_the_book_page(&h, &format!("{SHOP}/classes")).await;
        let done = h.click(Some(allowed_on(MARK, h.login))).await;
        assert!(
            done["done"]
                .as_str()
                .is_some_and(|d| d.contains("Book yoga")),
            "{done}"
        );
        assert!(h.approvals.pending(MARK).is_empty(), "no card");
        assert_eq!(h.tab().state().done, ["click"]);
        let kept = h.kept.0.lock().unwrap().clone();
        assert_eq!(kept.len(), 1, "kept in Approvals' history");
        assert_eq!(kept[0].status, "allowed");
        assert_eq!(kept[0].decision.as_deref(), Some("schedule"));
        assert!(kept[0].summary.contains("Book yoga"), "{}", kept[0].summary);
    }

    #[tokio::test]
    async fn the_same_click_in_the_chat_waits_on_its_card() {
        let h = harness().await;
        on_the_book_page(&h, &format!("{SHOP}/classes")).await;
        let held = h.click(None).await;
        assert_eq!(held["waiting_for_approval"], true);
        assert!(h.tab().state().done.is_empty());
    }

    #[tokio::test]
    async fn another_persons_schedule_allows_nothing_here() {
        let h = harness().await;
        on_the_book_page(&h, &format!("{SHOP}/classes")).await;
        let held = h.click(Some(allowed_on("person:majse", h.login))).await;
        assert_eq!(held["waiting_for_approval"], true);
    }

    #[tokio::test]
    async fn a_page_on_another_site_waits_on_its_card() {
        let h = harness().await;
        on_the_book_page(&h, "https://elsewhere.example/classes").await;
        let held = h.click(Some(allowed_on(MARK, h.login))).await;
        assert_eq!(held["waiting_for_approval"], true);
    }

    #[tokio::test]
    async fn a_login_that_isnt_saved_allows_nothing() {
        let h = harness().await;
        on_the_book_page(&h, &format!("{SHOP}/classes")).await;
        let held = h.click(Some(allowed_on(MARK, h.login + 99))).await;
        assert_eq!(held["waiting_for_approval"], true);
    }

    #[tokio::test]
    async fn typing_still_waits_on_its_card() {
        let h = harness().await;
        h.open(&format!("{SHOP}/classes")).await;
        h.tab()
            .state()
            .elements
            .insert(2, fake::textbox(2, 200, "Note"));
        let held = h
            .step(
                Some(allowed_on(MARK, h.login)),
                ActKind::Type,
                json!({ "ref": 2, "text": "front row", "reason": "a note" }),
            )
            .await;
        assert_eq!(held["waiting_for_approval"], true);
    }

    #[tokio::test]
    async fn a_checkout_still_waits_on_its_card() {
        let h = harness().await;
        on_the_book_page(&h, &format!("{SHOP}/checkout")).await;
        h.tab().state().checkout.at_checkout = true;
        let held = h.click(Some(allowed_on(MARK, h.login))).await;
        assert_eq!(held["waiting_for_approval"], true);
    }
}
