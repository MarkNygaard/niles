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
    assert_eq!(inbox.overflow.len(), MAX_CONNECTIONS);
    for _ in 0..MAX_CONNECTIONS {
        assert!(matches!(
            inbox.pop(),
            Some(Delivery::Status {
                status: wit::ConnectionStatus::Overflow(999),
                ..
            })
        ));
    }
}

#[test]
fn newer_generation_discards_old_messages_and_rejects_late_delivery() {
    let mut inbox = Inbox::default();
    inbox.register(1);
    for _ in 0..=INBOX_COUNT {
        inbox.push(message(1, 1, 1));
    }
    inbox.push(status(1, 2));
    inbox.push(message(1, 1, 1));
    assert!(matches!(
        inbox.pop(),
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
