//! What the Logins page changes ends a session open with that login
//! first: Sign out and Remove close it without keeping its profile; an
//! edit keeps the newest profile first. The order matters — the
//! session's save must land before the store forgets, and the store's
//! own rule decides what the change forgets.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_logins::{Credentials, Login, LoginEdit, LoginStore, NewLogin, Result, Secret};

use super::{Browsing, Save};

/// The store the Logins page and the API reach: every method delegates,
/// and the three that change a login end its open session first.
pub(crate) struct EndsSessions {
    inner: Arc<dyn LoginStore>,
    browsing: Arc<Browsing>,
}

/// Wrap the store: what the Logins page changes ends a session open
/// with that login first.
pub(crate) fn ending(inner: Arc<dyn LoginStore>, browsing: Arc<Browsing>) -> Arc<dyn LoginStore> {
    Arc::new(EndsSessions { inner, browsing })
}

#[async_trait]
impl LoginStore for EndsSessions {
    async fn list(&self, person: &str) -> Result<Vec<Login>> {
        self.inner.list(person).await
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<Login>> {
        self.inner.get(person, id).await
    }

    async fn add(&self, person: &str, login: &NewLogin) -> Result<Login> {
        self.inner.add(person, login).await
    }

    async fn update(&self, person: &str, id: i64, edit: &LoginEdit) -> Result<Option<Login>> {
        // The session's newest profile is kept first; the store's own
        // rule then decides whether the edit forgets it.
        self.browsing.end_login(person, id, Save::Profile).await;
        self.inner.update(person, id, edit).await
    }

    async fn remove(&self, person: &str, id: i64) -> Result<bool> {
        // The profile isn't kept again after it is forgotten.
        self.browsing.end_login(person, id, Save::Nothing).await;
        self.inner.remove(person, id).await
    }

    async fn sign_out(&self, person: &str, id: i64) -> Result<bool> {
        self.browsing.end_login(person, id, Save::Nothing).await;
        self.inner.sign_out(person, id).await
    }

    async fn credentials(&self, person: &str, id: i64) -> Result<Option<Credentials>> {
        self.inner.credentials(person, id).await
    }

    async fn secrets(&self, person: &str) -> Result<Vec<Credentials>> {
        self.inner.secrets(person).await
    }

    async fn save_profile(&self, person: &str, id: i64, profile: &Secret) -> Result<bool> {
        self.inner.save_profile(person, id, profile).await
    }

    async fn profile(&self, person: &str, id: i64) -> Result<Option<Secret>> {
        self.inner.profile(person, id).await
    }

    async fn used(&self, person: &str, id: i64, at: DateTime<Utc>) -> Result<bool> {
        self.inner.used(person, id, at).await
    }

    async fn drop_stale_profiles(&self, before: DateTime<Utc>) -> Result<u64> {
        self.inner.drop_stale_profiles(before).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::Approvals;
    use crate::browsing::fake::{self, FakeDriver};
    use crate::browsing::{Bound, with_store};
    use niles_api::chat::ApprovalStatus;
    use niles_core::EventBus;
    use niles_logins::MemoryLogins;

    const MARK: &str = "person:mark";
    const MAJSE: &str = "person:majse";
    const SHOP: &str = "https://shop.example";

    struct Harness {
        store: Arc<MemoryLogins>,
        wrapped: Arc<dyn LoginStore>,
        browsing: Arc<Browsing>,
        driver: Arc<FakeDriver>,
        login: Login,
    }

    async fn harness() -> Harness {
        let approvals = Arc::new(Approvals::new(EventBus::default(), None));
        let driver = Arc::new(FakeDriver::default());
        let store = Arc::new(MemoryLogins::new());
        let new = NewLogin::new(
            "Shop",
            &[SHOP.to_string()],
            Secret::new("mark.scott@example.com"),
            Secret::new("a password"),
        )
        .expect("a valid login");
        let login = store.add(MARK, &new).await.expect("kept");
        let browsing = with_store(approvals, driver.clone(), store.clone());
        let wrapped = ending(store.clone(), browsing.clone());
        Harness {
            store,
            wrapped,
            browsing,
            driver,
            login,
        }
    }

    impl Harness {
        /// A bound, signed-in session with the login.
        async fn bound(&self) {
            let asker = crate::approvals::Asker {
                person: MARK.into(),
                slug: Some("mark".into()),
                satellite: None,
                conversation: Some(1),
                in_chat: true,
            };
            let opened = self
                .browsing
                .session(&asker, super::super::Create::Yes)
                .await
                .expect("a session");
            fake::login_page(
                &self
                    .driver
                    .tabs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .last()
                    .unwrap()
                    .clone(),
                "https://shop.example/account",
                niles_browser::Asks::Nothing,
            );
            assert!(self.browsing.bind(
                MARK,
                opened.serial,
                Bound {
                    login: self.login.id,
                    label: "Shop".into(),
                    origins: vec![SHOP.to_string()],
                    signed_in: true,
                },
            ));
        }

        fn tab(&self) -> Arc<fake::FakeTab> {
            self.driver
                .tabs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .expect("a session was made")
                .clone()
        }
    }

    #[tokio::test]
    async fn sign_out_ends_the_session_without_keeping_its_profile_and_keeps_the_login() {
        let h = harness().await;
        h.bound().await;
        h.store
            .save_profile(MARK, h.login.id, &Secret::new("old cookies"))
            .await
            .expect("kept");
        assert!(
            h.wrapped
                .sign_out(MARK, h.login.id)
                .await
                .expect("signed out")
        );
        {
            let tab = h.tab();
            let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
            assert!(state.closed, "the session's tab is closed");
            assert!(
                !state.done.iter().any(|d| d.starts_with("profile")),
                "the profile wasn't kept again: {:?}",
                state.done
            );
        }
        assert_eq!(h.store.profile(MARK, h.login.id).await.expect("read"), None);
        assert!(h.store.get(MARK, h.login.id).await.expect("read").is_some());
    }

    #[tokio::test]
    async fn remove_ends_the_session_and_the_login() {
        let h = harness().await;
        h.bound().await;
        assert!(h.wrapped.remove(MARK, h.login.id).await.expect("removed"));
        assert!(
            h.tab()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .closed,
            "the session's tab is closed"
        );
        assert!(h.store.get(MARK, h.login.id).await.expect("read").is_none());
    }

    #[tokio::test]
    async fn an_edit_keeps_the_profile_first_then_ends_the_session() {
        let h = harness().await;
        h.bound().await;
        let edit = LoginEdit::new(
            "Shop",
            &[SHOP.to_string()],
            Secret::new("mark.scott@example.com"),
            None,
        )
        .expect("a valid edit");
        h.wrapped
            .update(MARK, h.login.id, &edit)
            .await
            .expect("edited")
            .expect("the login");
        let tab = h.tab();
        let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            state.done.iter().any(|d| d.starts_with("profile")),
            "the newest profile was kept first: {:?}",
            state.done
        );
        assert!(state.closed, "the session ended after the save");
    }

    #[tokio::test]
    async fn another_persons_sign_out_ends_nothing_of_theirs() {
        let h = harness().await;
        h.bound().await;
        assert!(
            !h.wrapped
                .sign_out(MAJSE, h.login.id)
                .await
                .expect("not theirs"),
            "another person's id signs nothing out"
        );
        let tab = h.tab();
        let state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!state.closed, "the session is untouched");
        assert!(state.done.is_empty(), "nothing ran: {:?}", state.done);
    }

    #[tokio::test]
    async fn a_waiting_card_lapses_when_its_login_is_signed_out() {
        let h = harness().await;
        h.bound().await;
        // A step card waits on the session.
        let card = niles_api::chat::ApprovalCard {
            id: "c1".into(),
            title: "Click \"Buy\"?".into(),
            subtitle: "shop.example".into(),
            icon: Some("browser".into()),
            fields: Vec::new(),
            body: None,
            allow_conversation: false,
            picture: true,
            status: ApprovalStatus::Pending,
            outcome: None,
        };
        let asker = crate::approvals::Asker {
            person: MARK.into(),
            slug: Some("mark".into()),
            satellite: None,
            conversation: Some(1),
            in_chat: true,
        };
        let serial = h
            .browsing
            .session(&asker, super::super::Create::No)
            .await
            .expect("open")
            .serial;
        let held = h.browsing.approvals.hold_step(
            &asker,
            Arc::new(NoopStep),
            "Browser",
            card,
            niles_api::approvals::Picture {
                mime: "image/jpeg",
                bytes: Vec::new(),
            },
        );
        h.browsing.hold(MARK, serial, held.id.clone());
        assert!(
            h.wrapped
                .sign_out(MARK, h.login.id)
                .await
                .expect("signed out")
        );
        assert!(
            h.browsing.approvals.take(&held.id, MARK).is_none(),
            "the waiting card lapsed with its session"
        );
    }

    struct NoopStep;

    #[async_trait::async_trait]
    impl niles_tools::Tool for NoopStep {
        fn descriptor(&self) -> niles_tools::ToolDescriptor {
            niles_tools::ToolDescriptor {
                name: "noop".into(),
                description: "A step that does nothing.".into(),
                parameters: serde_json::json!({ "type": "object" }),
            }
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> niles_tools::Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
    }
}
