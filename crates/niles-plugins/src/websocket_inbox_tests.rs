use super::*;

fn message(conn: u32, generation: u64, size: usize) -> Delivery {
    Delivery::Message {
        conn,
        generation,
        message: wit::Message::Binary(vec![0; size]),
    }
}
fn status(conn: u32, generation: u64) -> Delivery {
    Delivery::Status {
        conn,
        generation,
        status: wit::ConnectionStatus::Connected,
    }
}

#[test]
fn loss_is_attributed_to_evicted_stream_before_surviving_messages() {
    let mut inbox = Inbox::default();
    inbox.register(1);
    inbox.register(2);
    inbox.push(message(1, 1, 1));
    for _ in 0..INBOX_COUNT {
        inbox.push(message(2, 1, 1));
    }
    assert!(matches!(
        inbox.pop(),
        Some(Delivery::Status {
            conn: 1,
            generation: 1,
            status: wit::ConnectionStatus::Overflow(1)
        })
    ));
    assert!(matches!(
        inbox.pop(),
        Some(Delivery::Message { conn: 2, .. })
    ));
}

#[test]
fn byte_limit_and_metadata_are_bounded() {
    let mut inbox = Inbox::default();
    inbox.register(1);
    for _ in 0..INBOX_COUNT {
        inbox.push(message(1, 1, MAX_MESSAGE));
    }
    assert!(inbox.bytes <= INBOX_BYTES);
    assert_eq!(inbox.overflow.len(), 1);
    assert!(matches!(
        inbox.pop(),
        Some(Delivery::Status {
            conn: 1,
            status: wit::ConnectionStatus::Overflow(_),
            ..
        })
    ));
}

#[test]
fn status_flood_reports_coalescing_and_remains_bounded() {
    let mut inbox = Inbox::default();
    for conn in 0..100 {
        inbox.register(conn);
    }
    for _ in 0..1000 {
        for conn in 0..100 {
            inbox.push(status(conn, 1));
        }
    }
    assert_eq!(inbox.streams.len(), MAX_CONNECTIONS);
    assert_eq!(inbox.items.len(), MAX_CONNECTIONS);
    assert!(inbox.overflow.is_empty());
    for _ in 0..MAX_CONNECTIONS {
        assert!(matches!(
            inbox.pop(),
            Some(Delivery::Status {
                status: wit::ConnectionStatus::Connected,
                ..
            })
        ));
    }
}

#[test]
fn reconnect_preserves_prior_messages_and_reports_discarded_count() {
    let mut inbox = Inbox::default();
    inbox.register(1);
    for _ in 0..=INBOX_COUNT {
        inbox.push(message(1, 1, 1));
    }
    inbox.push(status(1, 2));
    inbox.push(message(1, 1, 1));
    let seen: Vec<_> = std::iter::from_fn(|| inbox.pop()).collect();
    assert!(matches!(
        seen.first(),
        Some(Delivery::Status {
            status: wit::ConnectionStatus::Overflow(count),
            ..
        }) if *count == (INBOX_COUNT + 1) as u32
    ));
    assert!(matches!(
        seen.get(1),
        Some(Delivery::Status {
            generation: 2,
            status: wit::ConnectionStatus::Connected,
            ..
        })
    ));
    assert!(inbox.is_empty());
    assert_eq!(inbox.bytes, 0);
}

#[test]
fn slow_plugin_observes_complete_reconnect_boundary_in_order() {
    let mut inbox = Inbox::default();
    inbox.register(1);
    inbox.push(status(1, 1));
    inbox.push(message(1, 1, 1));
    inbox.push(Delivery::Status {
        conn: 1,
        generation: 1,
        status: wit::ConnectionStatus::Disconnected("peer closed".into()),
    });
    inbox.push(status(1, 2));
    let observed: Vec<_> = std::iter::from_fn(|| inbox.pop()).collect();
    assert!(matches!(
        observed.as_slice(),
        [
            Delivery::Status {
                status: wit::ConnectionStatus::Overflow(1),
                ..
            },
            Delivery::Status {
                status: wit::ConnectionStatus::Disconnected(_),
                ..
            },
            Delivery::Status {
                generation: 2,
                status: wit::ConnectionStatus::Connected,
                ..
            },
        ]
    ));
}

#[test]
fn close_and_reset_reject_late_actor_deliveries() {
    let mut inbox = Inbox::default();
    inbox.register(1);
    inbox.push(message(1, 1, 1));
    inbox.remove(1);
    inbox.push(status(1, 2));
    assert!(inbox.is_empty());
    inbox.register(2);
    inbox.push(message(2, 1, 1));
    inbox.clear();
    inbox.push(message(2, 1, 1));
    assert!(inbox.is_empty());
    assert_eq!(inbox.bytes, 0);
    assert!(inbox.streams.is_empty());
}

#[test]
fn repeated_reconnects_keep_a_bounded_boundary_and_exact_loss() {
    let mut inbox = Inbox::default();
    inbox.register(1);
    for generation in 1..=1000 {
        inbox.push(status(1, generation));
        inbox.push(message(1, generation, 1));
        inbox.push(Delivery::Status {
            conn: 1,
            generation,
            status: wit::ConnectionStatus::Disconnected("closed".into()),
        });
        assert!(inbox.statuses <= 2);
        assert!(inbox.items.len() <= 3);
        assert!(inbox.overflow.len() <= 1);
    }
    inbox.push(status(1, 1001));
    assert!(matches!(
        inbox.pop(),
        Some(Delivery::Status {
            status: wit::ConnectionStatus::Overflow(1000),
            ..
        })
    ));
    assert!(matches!(
        inbox.pop(),
        Some(Delivery::Status {
            status: wit::ConnectionStatus::Disconnected(_),
            ..
        })
    ));
    assert!(matches!(
        inbox.pop(),
        Some(Delivery::Status {
            status: wit::ConnectionStatus::Connected,
            ..
        })
    ));
    assert!(inbox.is_empty());
}

#[test]
fn connected_stays_before_messages_without_false_overflow() {
    let mut inbox = Inbox::default();
    inbox.register(1);
    inbox.push(status(1, 1));
    inbox.push(message(1, 1, 1));
    inbox.push(status(1, 1));
    inbox.push(Delivery::Status {
        conn: 1,
        generation: 1,
        status: wit::ConnectionStatus::Disconnected("closed".into()),
    });
    assert!(matches!(
        inbox.pop(),
        Some(Delivery::Status {
            status: wit::ConnectionStatus::Connected,
            ..
        })
    ));
    assert!(matches!(inbox.pop(), Some(Delivery::Message { .. })));
    assert!(matches!(
        inbox.pop(),
        Some(Delivery::Status {
            status: wit::ConnectionStatus::Disconnected(_),
            ..
        })
    ));
    assert!(inbox.is_empty());
}
