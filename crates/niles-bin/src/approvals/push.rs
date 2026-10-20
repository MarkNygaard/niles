//! What a held call's notifications say: the push while it waits, with
//! the buttons Android and computers draw, and the quiet one that
//! replaces it once it is answered or lapses. web/public/sw.js shows
//! them.

use super::{Allow, first_line, summary};
use chrono::{DateTime, Utc};
use niles_api::chat::{ApprovalCard, ApprovalStatus};
use niles_webpush::inbox::Kind;
use niles_webpush::{Action, Approval, Message, Notice};

/// Where everything about a held call links: its card. The app finds
/// the conversation it is in, or shows it in Approvals
/// (web/src/components/ChatPage.tsx).
pub(super) fn link(id: &str) -> String {
    format!("/#/chat?approval={id}")
}

/// One tag for everything about a held call, so each replaces the last
/// on a device instead of stacking under it
/// (web/src/lib/approvals.ts `approvalTag`).
pub(super) fn tag(id: &str) -> String {
    format!("approval-{id}")
}

/// The card's question and the line under it: "Send this email? · From your Gmail".
fn heading(card: &ApprovalCard) -> String {
    if card.subtitle.is_empty() {
        card.title.clone()
    } else {
        format!("{} · {}", card.title, card.subtitle)
    }
}

/// What a held call's notification says, kept for the page: its
/// question, whose it is, and what it would do.
pub(super) fn waiting_notice(card: &ApprovalCard) -> Notice {
    let line = summary(card);
    Notice {
        kind: Kind::Approval,
        title: heading(card),
        body: if line.is_empty() {
            "Waiting for your answer.".into()
        } else {
            line
        },
        url: link(&card.id),
        tag: tag(&card.id),
    }
}

/// Its push: the same words, the buttons, and until when it waits.
pub(super) fn waiting_push(card: &ApprovalCard, until: DateTime<Utc>, allow: Allow) -> Message {
    let mut message = waiting_notice(card).push();
    message.approval = Some(Approval {
        id: card.id.clone(),
        until,
        actions: buttons(allow),
    });
    message
}

/// Deny on every one, since saying no is never the risky answer.
/// Approve only where a yes aloud would count too; Review for the
/// rest, whose card has to be read first, and a button may work from
/// a lock screen.
pub(super) fn buttons(allow: Allow) -> Vec<Action> {
    let first = if allow.without_its_card() {
        Action::Approve
    } else {
        Action::Review
    };
    vec![first, Action::Deny]
}

/// What replaces its notification once it is answered or lapses, on
/// every device and without a sound: there is nothing left to do.
pub(super) fn answered_push(card: &ApprovalCard) -> Message {
    Message {
        title: heading(card),
        body: outcome_line(card),
        url: link(&card.id),
        tag: tag(&card.id),
        approval: None,
        quiet: true,
    }
}

/// What became of it, in the words its card in the chat uses
/// (web/src/components/ApprovalCard.tsx, `outcome`).
fn outcome_line(card: &ApprovalCard) -> String {
    match card.status {
        ApprovalStatus::Allowed => match card.outcome.as_deref() {
            Some(outcome) if outcome != "Done" => format!("Allowed. {outcome}"),
            _ => "Allowed. Done.".into(),
        },
        ApprovalStatus::Denied => "Denied. Nothing was done.".into(),
        ApprovalStatus::Lapsed => "Lapsed. Nothing was done.".into(),
        ApprovalStatus::Failed => format!(
            "Allowed, but it didn't work: {}",
            card.outcome
                .as_deref()
                .map_or_else(|| "no reason given".to_string(), first_line)
        ),
        ApprovalStatus::Pending => "Waiting for you.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_api::chat::CardField;

    fn card() -> ApprovalCard {
        ApprovalCard {
            id: "a1".into(),
            title: "Let Linear create issue?".into(),
            subtitle: "Linear".into(),
            icon: Some("linear".into()),
            fields: vec![CardField {
                label: "Title".into(),
                value: "WLED presets".into(),
            }],
            body: None,
            allow_conversation: true,
            status: ApprovalStatus::Pending,
            outcome: None,
        }
    }

    fn mail() -> ApprovalCard {
        ApprovalCard {
            id: "m1".into(),
            title: "Send this email?".into(),
            subtitle: "From your Gmail, mark@gmail.com".into(),
            icon: Some("mail".into()),
            fields: vec![CardField {
                label: "To".into(),
                value: "jakob@example.com".into(),
            }],
            body: Some("Hi Jakob.".into()),
            allow_conversation: false,
            status: ApprovalStatus::Pending,
            outcome: None,
        }
    }

    fn until() -> DateTime<Utc> {
        DateTime::from_timestamp_millis(1_792_531_320_000).unwrap()
    }

    #[test]
    fn a_waiting_push_asks_its_question_and_says_whose_it_is() {
        let m = waiting_push(&card(), until(), Allow::Anywhere);
        assert_eq!(m.title, "Let Linear create issue? · Linear");
        assert_eq!(m.body, "Title: WLED presets");
        assert_eq!(m.url, "/#/chat?approval=a1");
        assert_eq!(m.tag, "approval-a1");
        assert!(!m.quiet);
        let approval = m.approval.expect("asking");
        assert_eq!(approval.id, "a1");
        assert_eq!(approval.until, until());
    }

    #[test]
    fn a_call_a_yes_aloud_may_answer_has_approve_and_deny() {
        let m = waiting_push(&card(), until(), Allow::Anywhere);
        assert_eq!(m.approval.unwrap().actions, [Action::Approve, Action::Deny]);
    }

    #[test]
    fn a_mail_has_review_and_deny_and_never_approve() {
        let m = waiting_push(&mail(), until(), Allow::OnItsCard);
        assert_eq!(m.approval.unwrap().actions, [Action::Review, Action::Deny]);
    }

    #[test]
    fn the_page_keeps_the_same_words_and_link() {
        let notice = waiting_notice(&card());
        let m = waiting_push(&card(), until(), Allow::Anywhere);
        assert_eq!(notice.kind, Kind::Approval);
        assert_eq!(notice.title, m.title);
        assert_eq!(notice.body, m.body);
        assert_eq!(notice.url, m.url);
    }

    #[test]
    fn a_card_without_a_subtitle_is_titled_by_its_question() {
        let mut c = card();
        c.subtitle = String::new();
        assert_eq!(waiting_push(&c, until(), Allow::Anywhere).title, c.title);
    }

    #[test]
    fn a_card_with_nothing_to_summarise_still_says_it_waits() {
        let mut c = card();
        c.fields = Vec::new();
        c.subtitle = String::new();
        assert_eq!(waiting_notice(&c).body, "Waiting for your answer.");
    }

    #[test]
    fn an_answer_replaces_it_quietly_with_what_became_of_it() {
        let mut c = card();
        for (status, outcome, line) in [
            (ApprovalStatus::Allowed, Some("Done"), "Allowed. Done."),
            (ApprovalStatus::Allowed, None, "Allowed. Done."),
            (
                ApprovalStatus::Allowed,
                Some("Sent to jakob@example.com"),
                "Allowed. Sent to jakob@example.com",
            ),
            (ApprovalStatus::Denied, None, "Denied. Nothing was done."),
            (ApprovalStatus::Lapsed, None, "Lapsed. Nothing was done."),
            (
                ApprovalStatus::Failed,
                Some("401: the key was refused\nsecond line"),
                "Allowed, but it didn't work: 401: the key was refused",
            ),
        ] {
            c.status = status;
            c.outcome = outcome.map(str::to_string);
            let m = answered_push(&c);
            assert_eq!(m.body, line, "{status:?}");
            assert!(m.quiet);
            assert_eq!(m.approval, None);
            assert_eq!(m.tag, "approval-a1");
            assert_eq!(m.url, "/#/chat?approval=a1");
        }
    }
}
