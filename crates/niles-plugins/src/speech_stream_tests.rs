use super::*;
use niles_tts::speech_session::{Eligibility, SpeechEligibility, SpeechOutput};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;

#[tokio::test]
async fn owner_teardown_discards_unpolled_results_and_allows_replacement() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(Output::default())),
            policy(),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let room = RoomName::parse("office").unwrap();
        let session = host.open("addon", room.clone()).unwrap();
        host.submit("addon", session, "dart".into()).unwrap();
        tokio::task::yield_now().await;
        host.close_owner("addon");
        let replacement = loop {
            match host.open("addon", room.clone()) {
                Ok(id) => break id,
                Err(SpeechError::Busy) => tokio::task::yield_now().await,
                other => panic!("unexpected reopen: {other:?}"),
            }
        };
        assert_ne!(replacement, session);
        assert_eq!(host.poll("addon", session), Err(SpeechError::NotOwned));
        host.close_owner("addon");
    })
    .await
    .unwrap();
}

fn policy() -> SpeechEligibility {
    SpeechEligibility::new(|_: PluginOwner, _: RoomName| async { Eligibility::Eligible })
}
struct Output {
    started: Arc<Notify>,
    gate: Option<Arc<tokio::sync::Semaphore>>,
    delivered: Arc<Mutex<Vec<(String, String)>>>,
    fail: bool,
}

#[tokio::test]
async fn open_requires_a_configured_route_and_names_the_selected_room() {
    let host = SessionSpeechHost::with_route_validator(
        Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(Output::default())),
            policy(),
        )),
        Handle::current(),
        |room| room.as_str() == "office",
    );
    let room = RoomName::parse("study").unwrap();
    let error = host.open("addon", room.clone()).unwrap_err();
    assert_eq!(error, SpeechError::NoSatellite(room));
    assert!(matches!(
        map_error(error),
        HostError::Unavailable(message) if message.contains("study")
    ));

    let session = host
        .open("addon", RoomName::parse("office").unwrap())
        .unwrap();
    host.close_owner("addon");
    assert_ne!(session, 0);
}

impl Default for Output {
    fn default() -> Self {
        Self {
            started: Arc::new(Notify::new()),
            gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
            delivered: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        }
    }
}
fn callbacks(output: Arc<Output>) -> SpeechOutput<(String, String)> {
    let prepare = |room: RoomName, text: String| async move { Ok((room.to_string(), text)) };
    let deliver_output = output;
    SpeechOutput::new(prepare, move |(room, text): (String, String)| {
        let output = deliver_output.clone();
        async move {
            output.started.notify_one();
            if let Some(gate) = &output.gate {
                gate.acquire().await.unwrap().forget();
            }
            output
                .delivered
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((room, text));
            if output.fail { Err(()) } else { Ok(()) }
        }
    })
}

struct LivePolicy {
    eligibility: Arc<tokio::sync::Mutex<Eligibility>>,
    checks: AtomicUsize,
    delivery_check_started: Arc<Notify>,
    delivery_check_gate: Arc<Notify>,
}
impl LivePolicy {
    fn eligible() -> Self {
        Self {
            eligibility: Arc::new(tokio::sync::Mutex::new(Eligibility::Eligible)),
            checks: AtomicUsize::new(0),
            delivery_check_started: Arc::new(Notify::new()),
            delivery_check_gate: Arc::new(Notify::new()),
        }
    }
}
fn live_policy(policy: Arc<LivePolicy>) -> SpeechEligibility {
    SpeechEligibility::new(move |_: PluginOwner, _: RoomName| {
        let policy = policy.clone();
        async move {
            if policy.checks.fetch_add(1, Ordering::SeqCst) == 2 {
                policy.delivery_check_started.notify_one();
                policy.delivery_check_gate.notified().await;
            }
            policy.eligibility.lock().await.clone()
        }
    })
}

async fn statuses_until(
    host: &SessionSpeechHost<(String, String)>,
    owner: &str,
    session: u32,
    expected: usize,
) -> Vec<(u64, SpeechOutcome)> {
    let mut statuses = Vec::new();
    while statuses.len() < expected {
        statuses.extend(host.poll(owner, session).unwrap());
        tokio::task::yield_now().await;
    }
    statuses
}

#[tokio::test]
async fn current_thread_calls_return_and_close_preserves_all_results() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(Output::default())),
            policy(),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let session = host
            .open("addon", RoomName::parse("office").unwrap())
            .unwrap();
        let first = host.submit("addon", session, "twenty".into()).unwrap();
        tokio::task::yield_now().await;
        assert!(host.poll("addon", session).unwrap().is_empty());
        let second = host.submit("addon", session, "total forty".into()).unwrap();
        host.close("addon", session);
        assert_eq!(
            host.submit("addon", session, "late".into()),
            Err(SpeechError::Closed)
        );
        let mut observed = Vec::new();
        while observed.len() < 2 {
            observed.extend(host.poll("addon", session).unwrap());
            tokio::task::yield_now().await;
        }
        assert_eq!(
            observed,
            vec![
                (first, SpeechOutcome::Cancelled),
                (second, SpeechOutcome::Cancelled)
            ]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn closed_session_is_closed_for_its_owner_but_not_owned_by_another() {
    let host = SessionSpeechHost::new(
        Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(Output::default())),
            policy(),
        )),
        Handle::current(),
    );
    let session = host
        .open("owner-a", RoomName::parse("office").unwrap())
        .unwrap();
    host.close("owner-a", session);
    assert_eq!(
        host.submit("owner-a", session, "late".into()),
        Err(SpeechError::Closed)
    );
    assert_eq!(
        host.submit("owner-b", session, "foreign".into()),
        Err(SpeechError::NotOwned)
    );
}

#[tokio::test]
async fn session_ids_wrap_without_reusing_retained_closed_sessions_and_requests_are_u64() {
    let host = SessionSpeechHost::new(
        Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(Output::default())),
            policy(),
        )),
        Handle::current(),
    );
    *host.next_session.lock().unwrap() = u32::MAX;
    *host.next_request.lock().unwrap() = u64::from(u32::MAX) + 1;
    let room = RoomName::parse("office").unwrap();
    let first = host.open("owner-a", room.clone()).unwrap();
    let request = host
        .submit("owner-a", first, "wide request id".into())
        .unwrap();
    assert_eq!(request, u64::from(u32::MAX) + 1);
    host.close("owner-a", first);
    let replacement = loop {
        match host.open("owner-a", room.clone()) {
            Ok(id) => break id,
            Err(SpeechError::Busy) => tokio::task::yield_now().await,
            other => panic!("unexpected reopen: {other:?}"),
        }
    };
    assert_ne!(replacement, first);
    host.close_owner("owner-a");
}

#[tokio::test]
async fn plain_thread_calls_need_no_current_runtime_and_capacity_is_bounded() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(Output::default())),
            policy(),
        ));
        let host = Arc::new(SessionSpeechHost::new(manager, Handle::current()));
        let other = host.clone();
        let session = std::thread::spawn(move || {
            let session = other
                .open("addon", RoomName::parse("office").unwrap())
                .unwrap();
            for _ in 0..MAX_RETAINED_RESULTS {
                other.submit("addon", session, "dart".into()).unwrap();
            }
            assert_eq!(
                other.submit("addon", session, "excess".into()),
                Err(SpeechError::Limit)
            );
            assert!(other.poll("addon", session).unwrap().is_empty());
            other.close("addon", session);
            session
        })
        .join()
        .unwrap();
        let mut observed = Vec::new();
        while observed.len() < MAX_RETAINED_RESULTS {
            observed.extend(host.poll("addon", session).unwrap());
            tokio::task::yield_now().await;
        }
        assert!(
            observed
                .iter()
                .all(|(_, result)| *result == SpeechOutcome::Cancelled)
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn accepted_requests_report_ordered_ids_and_exact_room_text_delivery() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let output = Output {
            gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
            ..Output::default()
        };
        let gate = output.gate.clone().unwrap();
        let delivered = output.delivered.clone();
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(output)),
            policy(),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let session = host
            .open("addon", RoomName::parse("office").unwrap())
            .unwrap();
        let first = host
            .submit("addon", session, "first exact text".into())
            .unwrap();
        let second = host
            .submit("addon", session, "second exact text".into())
            .unwrap();
        gate.add_permits(2);

        assert_eq!(
            statuses_until(&host, "addon", session, 2).await,
            [
                (first, SpeechOutcome::Replaced),
                (second, SpeechOutcome::Delivered)
            ]
        );
        assert_eq!(
            *delivered.lock().unwrap_or_else(|error| error.into_inner()),
            [("office".into(), "second exact text".into())]
        );
        assert!(host.poll("addon", session).unwrap().is_empty());
        host.close_owner("addon");
    })
    .await
    .expect("ordered delivery exceeded its five-second test budget");
}

#[tokio::test(start_paused = true)]
async fn queued_host_request_expires_behind_gated_output_and_reports_once_in_order() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let output = Output::default();
        let gate = output.gate.clone().unwrap();
        let started = output.started.clone();
        let delivered = output.delivered.clone();
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(output)),
            policy(),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let session = host
            .open("addon", RoomName::parse("office").unwrap())
            .unwrap();

        let first = host
            .submit("addon", session, "gated first output".into())
            .unwrap();
        started.notified().await;
        let expired = host
            .submit("addon", session, "must never reach output".into())
            .unwrap();
        tokio::time::advance(niles_tts::speech_session::QUEUED_UTTERANCE_TTL).await;
        gate.add_permits(1);

        assert_eq!(
            statuses_until(&host, "addon", session, 2).await,
            [
                (first, SpeechOutcome::Delivered),
                (expired, SpeechOutcome::Expired)
            ]
        );
        assert!(host.poll("addon", session).unwrap().is_empty());
        assert_eq!(
            *delivered.lock().unwrap_or_else(|error| error.into_inner()),
            [("office".into(), "gated first output".into())]
        );
        host.close_owner("addon");
    })
    .await
    .expect("queued expiry exceeded its five-second test budget");
}

#[tokio::test]
async fn overload_preserves_accepted_statuses_and_draining_allows_later_submission() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let output = Output::default();
        let gate = output.gate.clone().unwrap();
        let delivered = output.delivered.clone();
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(output)),
            policy(),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let session = host
            .open("addon", RoomName::parse("office").unwrap())
            .unwrap();
        let accepted: Vec<_> = (0..MAX_RETAINED_RESULTS)
            .map(|index| {
                host.submit("addon", session, format!("accepted-{index}"))
                    .unwrap()
            })
            .collect();
        assert_eq!(
            host.submit("addon", session, "rejected overload".into()),
            Err(SpeechError::Limit)
        );
        assert!(host.poll("addon", session).unwrap().is_empty());

        gate.add_permits(MAX_RETAINED_RESULTS);
        assert_eq!(
            statuses_until(&host, "addon", session, MAX_RETAINED_RESULTS).await,
            accepted
                .iter()
                .copied()
                .enumerate()
                .map(|(index, request)| {
                    (
                        request,
                        if index + 1 == MAX_RETAINED_RESULTS {
                            SpeechOutcome::Delivered
                        } else {
                            SpeechOutcome::Replaced
                        },
                    )
                })
                .collect::<Vec<_>>()
        );
        assert!(host.poll("addon", session).unwrap().is_empty());

        let subsequent = host
            .submit("addon", session, "accepted after drain".into())
            .unwrap();
        gate.add_permits(1);
        assert_eq!(
            statuses_until(&host, "addon", session, 1).await,
            [(subsequent, SpeechOutcome::Delivered)]
        );
        assert_eq!(
            delivered
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .len(),
            2
        );
        host.close_owner("addon");
    })
    .await
    .expect("overload and drain exceeded its five-second test budget");
}

#[tokio::test]
async fn close_during_blocked_delivery_eligibility_cancels_and_releases_reservations() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let policy = LivePolicy::eligible();
        let delivery_check_started = policy.delivery_check_started.clone();
        let delivered_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let output = Output {
            gate: Some(delivered_gate.clone()),
            ..Output::default()
        };
        let delivered = output.delivered.clone();
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(output)),
            live_policy(Arc::new(policy)),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let room = RoomName::parse("office").unwrap();
        let session = host.open("addon", room.clone()).unwrap();
        let request = host
            .submit("addon", session, "cancel at policy gate".into())
            .unwrap();
        delivery_check_started.notified().await;

        host.close("addon", session);
        let mut observed = Vec::new();
        while observed.is_empty() {
            observed.extend(host.poll("addon", session).unwrap());
            tokio::task::yield_now().await;
        }
        assert_eq!(observed, [(request, SpeechOutcome::Cancelled)]);
        assert!(host.poll("addon", session).unwrap().is_empty());
        assert!(
            delivered
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        );

        let replacement = loop {
            match host.open("addon", room.clone()) {
                Ok(id) => break id,
                Err(SpeechError::Busy) => tokio::task::yield_now().await,
                other => panic!("unexpected reopen after blocked-policy close: {other:?}"),
            }
        };
        assert_ne!(replacement, session);
        let next = host
            .submit("addon", replacement, "replacement succeeds".into())
            .unwrap();
        delivered_gate.add_permits(1);
        assert_eq!(
            statuses_until(&host, "addon", replacement, 1).await,
            [(next, SpeechOutcome::Delivered)]
        );
        assert_eq!(
            *delivered.lock().unwrap_or_else(|error| error.into_inner()),
            [("office".into(), "replacement succeeds".into())]
        );
        host.close_owner("addon");
    })
    .await
    .expect("blocked-policy close exceeded its five-second test budget");
}

#[tokio::test]
async fn rate_limit_is_reported_with_the_submission_correlation_id() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let output = Output {
            gate: None,
            ..Output::default()
        };
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(output)),
            policy(),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let room = RoomName::parse("office").unwrap();
        let mut accepted = 0;
        while accepted < niles_tts::speech_session::OWNER_RATE_LIMIT {
            let session = loop {
                match host.open("addon", room.clone()) {
                    Ok(id) => break id,
                    Err(SpeechError::Busy) => tokio::task::yield_now().await,
                    other => panic!("unexpected open result: {other:?}"),
                }
            };
            let count =
                (niles_tts::speech_session::OWNER_RATE_LIMIT - accepted).min(MAX_RETAINED_RESULTS);
            let ids: Vec<_> = (0..count)
                .map(|_| host.submit("addon", session, "quota".into()).unwrap())
                .collect();
            let statuses = statuses_until(&host, "addon", session, count).await;
            assert_eq!(
                statuses
                    .iter()
                    .map(|(request, _)| *request)
                    .collect::<Vec<_>>(),
                ids
            );
            accepted += count;
            host.close("addon", session);
        }
        let session = loop {
            match host.open("addon", room.clone()) {
                Ok(id) => break id,
                Err(SpeechError::Busy) => tokio::task::yield_now().await,
                other => panic!("unexpected open result after quota: {other:?}"),
            }
        };
        let request = host
            .submit("addon", session, "rate limited".into())
            .unwrap();
        assert_eq!(
            statuses_until(&host, "addon", session, 1).await,
            [(request, SpeechOutcome::RateLimited)]
        );
        host.close_owner("addon");
    })
    .await
    .expect("rate-limit correlation exceeded its ten-second test budget");
}

#[tokio::test]
async fn output_transport_failure_is_a_single_terminal_failed_status() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let output = Output {
            gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
            fail: true,
            ..Output::default()
        };
        let gate = output.gate.clone().unwrap();
        let delivered = output.delivered.clone();
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(output)),
            policy(),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let session = host
            .open("addon", RoomName::parse("office").unwrap())
            .unwrap();
        let request = host
            .submit("addon", session, "transport fails".into())
            .unwrap();
        gate.add_permits(1);
        assert_eq!(
            statuses_until(&host, "addon", session, 1).await,
            [(request, SpeechOutcome::Failed)]
        );
        assert!(host.poll("addon", session).unwrap().is_empty());
        assert_eq!(
            *delivered.lock().unwrap_or_else(|error| error.into_inner()),
            [("office".into(), "transport fails".into())]
        );
        host.close_owner("addon");
    })
    .await
    .expect("transport failure exceeded its five-second test budget");
}

#[tokio::test]
async fn live_policy_changes_are_terminal_and_rejected_text_never_reaches_output() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for (eligibility, expected) in [
            (Eligibility::Busy, SpeechOutcome::Busy),
            (Eligibility::QuietHours, SpeechOutcome::Quiet),
            (Eligibility::Unavailable, SpeechOutcome::Unavailable),
            (Eligibility::Disabled, SpeechOutcome::Disabled),
        ] {
            let policy = LivePolicy::eligible();
            let delivery_check_started = policy.delivery_check_started.clone();
            let delivery_check_gate = policy.delivery_check_gate.clone();
            let eligibility_state = policy.eligibility.clone();
            let output = Output::default();
            let delivered = output.delivered.clone();
            let manager = Arc::new(SpeechSessionManager::new(
                callbacks(Arc::new(output)),
                live_policy(Arc::new(policy)),
            ));
            let host = SessionSpeechHost::new(manager, Handle::current());
            let session = host
                .open("addon", RoomName::parse("office").unwrap())
                .unwrap();
            let request = host
                .submit("addon", session, "must not be spoken".into())
                .unwrap();
            delivery_check_started.notified().await;
            *eligibility_state.lock().await = eligibility;
            delivery_check_gate.notify_one();

            assert_eq!(
                statuses_until(&host, "addon", session, 1).await,
                [(request, expected)]
            );
            assert!(host.poll("addon", session).unwrap().is_empty());
            assert!(
                delivered
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .is_empty()
            );
            host.close_owner("addon");
        }
    })
    .await
    .expect("live policy outcome exceeded its five-second test budget");
}

#[tokio::test]
async fn unrelated_owner_cannot_poll_submit_or_close_another_sessions_work() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let output = Output {
            gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
            ..Output::default()
        };
        let gate = output.gate.clone().unwrap();
        let delivered = output.delivered.clone();
        let manager = Arc::new(SpeechSessionManager::new(
            callbacks(Arc::new(output)),
            policy(),
        ));
        let host = SessionSpeechHost::new(manager, Handle::current());
        let session = host
            .open("owner-a", RoomName::parse("office").unwrap())
            .unwrap();
        let request = host
            .submit("owner-a", session, "owner a text".into())
            .unwrap();

        assert_eq!(host.poll("owner-b", session), Err(SpeechError::NotOwned));
        assert_eq!(
            host.submit("owner-b", session, "intrusion".into()),
            Err(SpeechError::NotOwned)
        );
        host.close("owner-b", session);
        gate.add_permits(1);
        assert_eq!(
            statuses_until(&host, "owner-a", session, 1).await,
            [(request, SpeechOutcome::Delivered)]
        );
        assert!(host.poll("owner-a", session).unwrap().is_empty());
        assert_eq!(
            *delivered.lock().unwrap_or_else(|error| error.into_inner()),
            [("office".into(), "owner a text".into())]
        );
        host.close_owner("owner-a");
    })
    .await
    .expect("owner isolation exceeded its five-second test budget");
}
