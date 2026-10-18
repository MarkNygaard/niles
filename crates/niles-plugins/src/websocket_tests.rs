use super::*;

#[tokio::test(start_paused = true)]
async fn queued_send_does_not_shorten_retry_deadline() {
    let (tx, mut rx) = mpsc::channel(OUTBOX_COUNT);
    let shared = Shared {
        connected: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        generation: AtomicU64::new(0),
        out_count: AtomicUsize::new(1),
        out_bytes: AtomicUsize::new(1),
    };
    let deadline = tokio::time::Instant::now() + BACKOFF[0];
    let worker = tokio::spawn(async move { wait_for_retry(&mut rx, deadline, &shared).await });
    tx.send(Command::Send {
        message: Message::Text("x".into()),
        generation: 0,
    })
    .await
    .unwrap();
    tokio::task::yield_now().await;
    assert!(!worker.is_finished());
    tokio::time::advance(BACKOFF[0] - Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert!(!worker.is_finished());
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(!worker.await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn close_interrupts_retry_wait_immediately() {
    let (tx, mut rx) = mpsc::channel(OUTBOX_COUNT);
    let shared = Shared {
        connected: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        generation: AtomicU64::new(0),
        out_count: AtomicUsize::new(0),
        out_bytes: AtomicUsize::new(0),
    };
    let deadline = tokio::time::Instant::now() + BACKOFF[0];
    let worker = tokio::spawn(async move { wait_for_retry(&mut rx, deadline, &shared).await });
    tx.send(Command::Close).await.unwrap();
    assert!(worker.await.unwrap());
}

#[test]
fn successful_handshake_resets_consecutive_retry_sequence() {
    assert_eq!(
        retry_waits([false, false, true, false, false]),
        [BACKOFF[0], BACKOFF[1], BACKOFF[0], BACKOFF[1]]
    );
    assert_eq!(
        retry_waits([false; 10]),
        [
            BACKOFF[0], BACKOFF[1], BACKOFF[2], BACKOFF[3], BACKOFF[4], BACKOFF[5], BACKOFF[5],
            BACKOFF[5], BACKOFF[5], BACKOFF[5]
        ]
    );
}

fn connection() -> (
    Connection,
    mpsc::Receiver<Command>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = mpsc::channel(OUTBOX_COUNT);
    let task = tokio::spawn(std::future::pending());
    let connection = Connection {
        owner: None,
        shared: Arc::new(Shared {
            connected: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            out_count: AtomicUsize::new(0),
            out_bytes: AtomicUsize::new(0),
        }),
        tx,
        task: task.abort_handle(),
        handle: tokio::runtime::Handle::current(),
    };
    (connection, rx, task)
}

#[tokio::test]
async fn full_queue_close_cancels_task_and_rejects_further_sends() {
    let (connection, _rx, task) = connection();
    for _ in 0..OUTBOX_COUNT {
        connection.send(wit::Message::Text("x".into())).unwrap();
    }
    assert!(matches!(
        connection.send(wit::Message::Text("x".into())),
        Err(HostError::Limit(_))
    ));
    assert_eq!(
        connection.shared.out_count.load(Ordering::SeqCst),
        OUTBOX_COUNT
    );
    connection.close();
    assert!(matches!(
        connection.send(wit::Message::Text("x".into())),
        Err(HostError::Unavailable(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
}

#[tokio::test]
async fn drop_cancels_task_without_waiting_for_network() {
    let (connection, _rx, task) = connection();
    drop(connection);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
}

#[tokio::test]
async fn queued_close_has_bounded_grace_even_when_task_is_stuck() {
    let (connection, _rx, task) = connection();
    connection.close();
    drop(connection);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
}

#[tokio::test]
async fn outgoing_size_and_byte_budget_are_enforced_without_counter_leaks() {
    let (connection, _rx, _task) = connection();
    assert!(matches!(
        connection.send(wit::Message::Binary(vec![0; MAX_MESSAGE + 1])),
        Err(HostError::Limit(_))
    ));
    assert_eq!(connection.shared.out_count.load(Ordering::SeqCst), 0);
    for _ in 0..OUTBOX_BYTES / MAX_MESSAGE {
        connection
            .send(wit::Message::Binary(vec![0; MAX_MESSAGE]))
            .unwrap();
    }
    assert!(matches!(
        connection.send(wit::Message::Text("x".into())),
        Err(HostError::Limit(_))
    ));
    assert_eq!(
        connection.shared.out_bytes.load(Ordering::SeqCst),
        OUTBOX_BYTES
    );
}

#[test]
fn endpoint_and_header_validation_rejects_authority_changes() {
    let target = WsTarget {
        host: "example.com".into(),
        port: 443,
        tls: true,
        local: false,
        loopback: false,
    };
    assert!(check_url("wss://example.com/path?q=1", &target).is_ok());
    for url in [
        "ws://example.com/path",
        "wss://other.example/path",
        "wss://example.com:444/path",
        "wss://user:secret@example.com/path",
        "wss://example.com/path#fragment",
    ] {
        assert!(check_url(url, &target).is_err(), "{url}");
    }
    for name in [
        "Host",
        "Connection",
        "Upgrade",
        "Sec-WebSocket-Key",
        "bad name",
    ] {
        assert!(check_headers(&[(name.into(), "value".into())]).is_err());
    }
    assert!(check_headers(&[("Authorization".into(), "value\r\nInjected: yes".into())]).is_err());
    assert!(check_headers(&[("Authorization".into(), "Bearer synthetic".into())]).is_ok());
}

#[test]
fn address_policy_rejects_loopback_metadata_and_addresses_outside_grant_kind() {
    let public = WsTarget {
        host: "stream.example.com".into(),
        port: 443,
        tls: true,
        local: false,
        loopback: false,
    };
    for ip in ["127.0.0.1", "::1", "169.254.169.254", "fd00:ec2::254"] {
        assert!(public.allows(ip.parse().unwrap()).is_err(), "{ip}");
    }
    assert!(public.allows("192.168.1.8".parse().unwrap()).is_err());
    assert!(public.allows("93.184.216.34".parse().unwrap()).is_ok());

    let local = WsTarget {
        host: "192.168.1.50".into(),
        port: 80,
        tls: false,
        local: true,
        loopback: false,
    };
    assert!(local.allows("192.168.1.50".parse().unwrap()).is_ok());
    assert!(local.allows("93.184.216.34".parse().unwrap()).is_err());

    let dev_loopback = WsTarget {
        loopback: true,
        ..local
    };
    assert!(dev_loopback.allows("127.0.0.1".parse().unwrap()).is_ok());
    assert!(
        dev_loopback
            .allows("169.254.169.254".parse().unwrap())
            .is_err()
    );
}
