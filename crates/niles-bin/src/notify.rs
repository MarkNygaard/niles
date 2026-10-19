//! Telling one person something: kept for their Notifications page,
//! said on their own connections so the bell counts it, and pushed to
//! their devices. Every push Niles sends a person goes through here, so
//! the page has everything the lock screen had.
//!
//! Kept whenever there is a database, whether or not they turned
//! notifications on anywhere; pushed whenever there is a push key.

use niles_core::{Event, EventBus};
use niles_webpush::{Inbox, Notice, WebPush};
use std::sync::Arc;

pub(crate) struct Notify {
    inbox: Option<Arc<dyn Inbox>>,
    push: Option<Arc<WebPush>>,
    bus: EventBus,
}

impl Notify {
    pub(crate) fn new(
        inbox: Option<Arc<dyn Inbox>>,
        push: Option<Arc<WebPush>>,
        bus: EventBus,
    ) -> Self {
        Self { inbox, push, bus }
    }

    /// Where the page reads them from.
    pub(crate) fn inbox(&self) -> Option<Arc<dyn Inbox>> {
        self.inbox.clone()
    }

    /// `notice` for `person`. Nothing for a voice nobody recognised,
    /// which has no page to keep it on and no devices.
    pub(crate) async fn person(&self, person: &str, notice: &Notice) {
        if !(person.starts_with("person:") || person.starts_with("email:")) {
            return;
        }
        if let Some(inbox) = &self.inbox {
            match inbox.add(person, notice).await {
                Ok(kept) => {
                    self.bus.publish(Event::NotificationAdded {
                        person: person.to_string(),
                        id: kept.id,
                    });
                }
                Err(e) => tracing::warn!("[notify] {person}: {:?} was not kept: {e}", notice.title),
            }
        }
        let Some(push) = &self.push else {
            return;
        };
        match push.send(person, &notice.push()).await {
            Ok(delivery) if delivery.sent > 0 => tracing::info!(
                "[notify] {person}: told {} device(s) about {:?}",
                delivery.sent,
                notice.title
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(
                "[notify] {person}: no device was told about {:?}: {e}",
                notice.title
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_webpush::MemoryInbox;
    use niles_webpush::inbox::Kind;

    fn notice() -> Notice {
        Notice {
            kind: Kind::Artifact,
            title: "Thailand travel plan".into(),
            body: "Your Thailand travel plan is ready.".into(),
            url: "/#/chat?artifact=7".into(),
            tag: "artifact-7".into(),
        }
    }

    fn push() -> Arc<WebPush> {
        Arc::new(WebPush::new(
            Arc::new(niles_webpush::MemoryPush::default()),
            reqwest::Client::new(),
        ))
    }

    #[tokio::test]
    async fn a_push_is_kept_and_said_even_with_no_device_to_push_to() {
        let inbox = Arc::new(MemoryInbox::default());
        let bus = EventBus::default();
        let mut events = bus.subscribe();
        let notify = Notify::new(Some(inbox.clone()), Some(push()), bus);
        notify.person("person:mark", &notice()).await;

        let kept = inbox.page("person:mark", None, 10).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, "artifact");
        assert_eq!(kept[0].body, "Your Thailand travel plan is ready.");
        match events.try_recv() {
            Ok(Event::NotificationAdded { person, id }) => {
                assert_eq!(person, "person:mark");
                assert_eq!(id, kept[0].id);
            }
            other => panic!("expected NotificationAdded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn without_a_push_key_it_is_still_kept() {
        let inbox = Arc::new(MemoryInbox::default());
        let notify = Notify::new(Some(inbox.clone()), None, EventBus::default());
        notify.person("email:anna@example.com", &notice()).await;
        assert_eq!(inbox.unread("email:anna@example.com").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_voice_nobody_recognised_is_told_nothing() {
        let inbox = Arc::new(MemoryInbox::default());
        let bus = EventBus::default();
        let mut events = bus.subscribe();
        let notify = Notify::new(Some(inbox.clone()), None, bus);
        notify.person("satellite:10.0.0.2:4000", &notice()).await;
        assert_eq!(inbox.unread("satellite:10.0.0.2:4000").await.unwrap(), 0);
        assert!(events.try_recv().is_err());
    }
}
