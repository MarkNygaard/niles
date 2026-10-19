use super::*;
use niles_notifications::{NotificationDelivery, QuietHoursConfig, QuietWindow, TransportOutcome};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

struct Delivery {
    outcome: TransportOutcome,
    gate: Option<Arc<tokio::sync::Semaphore>>,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl NotificationDelivery for Delivery {
    async fn deliver(&self, _: &str, _: Option<&str>, _: Priority) -> TransportOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            let _permit = gate.acquire().await.unwrap();
        }
        self.outcome
    }
}
fn tracker(delivery: Arc<Delivery>) -> NotificationRequests {
    let center = Arc::new(NotificationCenter::new(64).with_delivery(delivery));
    NotificationRequests::new(center, Handle::current())
}
async fn wait_for(
    tracker: &NotificationRequests,
    owner: &str,
    generation: u64,
    id: RequestId,
) -> RequestStatus {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = tracker.status(owner, generation, id).unwrap();
            if status != RequestStatus::Pending {
                return status;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn maps_each_center_terminal_outcome_and_status_is_nonconsuming() {
    for (transport, expected) in [
        (TransportOutcome::Completed, RequestStatus::Completed),
        (TransportOutcome::Partial, RequestStatus::Partial),
        (TransportOutcome::Failed, RequestStatus::Failed),
    ] {
        let delivery = Arc::new(Delivery {
            outcome: transport,
            gate: None,
            calls: AtomicUsize::new(0),
        });
        let tracker = tracker(delivery);
        let reservation = tracker.reserve("owner", 1).unwrap();
        let id = tracker
            .accept(reservation, "hello".into(), None, Priority::Important)
            .unwrap();
        assert_eq!(wait_for(&tracker, "owner", 1, id).await, expected);
        assert_eq!(tracker.status("owner", 1, id), Ok(expected));
        assert_eq!(tracker.status("owner", 1, id), Ok(expected));
    }
}

#[tokio::test(start_paused = true)]
async fn quiet_suppression_and_twenty_second_timeout_are_terminal() {
    let delivery = Arc::new(Delivery {
        outcome: TransportOutcome::Completed,
        gate: None,
        calls: AtomicUsize::new(0),
    });
    let quiet = QuietHoursConfig {
        enabled: true,
        window: Some(QuietWindow::new(
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        )),
        timezone: None,
    };
    let center = Arc::new(
        NotificationCenter::new(8)
            .with_delivery(delivery)
            .with_quiet_hours(quiet),
    );
    let quiet_tracker = NotificationRequests::new(center, Handle::current());
    let id = quiet_tracker
        .accept(
            quiet_tracker.reserve("quiet", 2).unwrap(),
            "quiet".into(),
            None,
            Priority::Routine,
        )
        .unwrap();
    assert_eq!(
        wait_for(&quiet_tracker, "quiet", 2, id).await,
        RequestStatus::Suppressed
    );

    let delivery = Arc::new(Delivery {
        outcome: TransportOutcome::Completed,
        gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
        calls: AtomicUsize::new(0),
    });
    let calls = &delivery.calls;
    let tracker = tracker(delivery.clone());
    let id = tracker
        .accept(
            tracker.reserve("timeout", 3).unwrap(),
            "stuck".into(),
            None,
            Priority::Important,
        )
        .unwrap();
    while calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;
    tokio::time::advance(DELIVERY_TIMEOUT).await;
    tokio::task::yield_now().await;
    assert_eq!(
        wait_for(&tracker, "timeout", 3, id).await,
        RequestStatus::Failed
    );
}

#[tokio::test(start_paused = true)]
async fn capacity_includes_pending_and_terminal_until_expiry() {
    let delivery = Arc::new(Delivery {
        outcome: TransportOutcome::Completed,
        gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
        calls: AtomicUsize::new(0),
    });
    let gate = delivery.gate.clone().unwrap();
    let tracker = tracker(delivery);
    let mut ids = Vec::new();
    for _ in 0..MAX_REQUESTS_PER_INSTANCE {
        let id = tracker
            .accept(
                tracker.reserve("owner", 7).unwrap(),
                "wait".into(),
                None,
                Priority::Important,
            )
            .unwrap();
        ids.push(id);
    }
    assert_eq!(
        tracker.reserve("owner", 7).err(),
        Some(TrackerError::Capacity)
    );
    gate.add_permits(MAX_REQUESTS_PER_INSTANCE);
    for id in &ids {
        assert_eq!(
            wait_for(&tracker, "owner", 7, *id).await,
            RequestStatus::Completed
        );
    }
    assert_eq!(
        tracker.reserve("owner", 7).err(),
        Some(TrackerError::Capacity)
    );
    tokio::time::advance(TERMINAL_RETENTION).await;
    assert!(tracker.reserve("owner", 7).is_ok());
}

#[tokio::test(start_paused = true)]
async fn ownership_generation_unknown_ids_and_expiry_are_denied() {
    let delivery = Arc::new(Delivery {
        outcome: TransportOutcome::Completed,
        gate: None,
        calls: AtomicUsize::new(0),
    });
    let tracker = tracker(delivery);
    let id = tracker
        .accept(
            tracker.reserve("owner-a", 11).unwrap(),
            "hello".into(),
            None,
            Priority::Important,
        )
        .unwrap();
    assert_eq!(
        tracker.status("owner-b", 11, id),
        Err(TrackerError::Unknown)
    );
    assert_eq!(
        tracker.status("owner-a", 12, id),
        Err(TrackerError::Unknown)
    );
    assert_eq!(
        tracker.status("owner-a", 11, RequestId(id.0.wrapping_add(1))),
        Err(TrackerError::Unknown)
    );
    assert_eq!(
        wait_for(&tracker, "owner-a", 11, id).await,
        RequestStatus::Completed
    );
    tokio::time::advance(TERMINAL_RETENTION).await;
    assert_eq!(
        tracker.status("owner-a", 11, id),
        Err(TrackerError::Unknown)
    );
}

#[tokio::test]
async fn teardown_cancels_work_rejects_old_generation_and_never_resurrects_status() {
    let delivery = Arc::new(Delivery {
        outcome: TransportOutcome::Completed,
        gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
        calls: AtomicUsize::new(0),
    });
    let gate = delivery.gate.clone().unwrap();
    let calls = &delivery.calls;
    let tracker = tracker(delivery.clone());
    let reservation = tracker.reserve("owner", 20).unwrap();
    let id = tracker
        .accept(reservation, "pending".into(), None, Priority::Important)
        .unwrap();
    while calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(tracker.status("owner", 20, id), Ok(RequestStatus::Pending));
    tracker.close_owner("owner", 20);
    assert_eq!(tracker.status("owner", 20, id), Err(TrackerError::Unknown));
    assert_eq!(
        tracker.reserve("owner", 20).err(),
        Some(TrackerError::Closed)
    );
    gate.add_permits(1);
    tokio::task::yield_now().await;
    assert_eq!(tracker.status("owner", 20, id), Err(TrackerError::Unknown));
    let newer = tracker
        .accept(
            tracker.reserve("owner", 21).unwrap(),
            "new".into(),
            None,
            Priority::Important,
        )
        .unwrap();
    assert_ne!(id, newer);
    tracker.close_owner("owner", 21);
}

#[tokio::test]
async fn dropping_reservation_releases_capacity_without_creating_a_request() {
    let delivery = Arc::new(Delivery {
        outcome: TransportOutcome::Completed,
        gate: None,
        calls: AtomicUsize::new(0),
    });
    let tracker = tracker(delivery.clone());
    let reservation = tracker.reserve("owner", 1).unwrap();
    let id = reservation.id;
    drop(reservation);
    assert_eq!(tracker.status("owner", 1, id), Err(TrackerError::Unknown));
    assert_ne!(tracker.reserve("owner", 1).unwrap().id, id);
    assert_eq!(delivery.calls.load(Ordering::SeqCst), 0);
}
