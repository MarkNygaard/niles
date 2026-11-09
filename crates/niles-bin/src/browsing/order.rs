//! The order card and the stop at card details.
//!
//! The step that places an order or pays waits on a card of its own,
//! with the total and the delivery address the page shows. Niles
//! decides it from the page, by the element's label and the checkout
//! around it, and errs towards the order card. Niles pays only with
//! what the site's account has saved: a page asking for card details
//! stops the run, and text holding a card number is never typed. See
//! ARCHITECTURE.md's "The order card" and "Payment".

use super::step::{self, Action};
use niles_api::chat::{ApprovalCard, ApprovalStatus, CardField};
use niles_browser::{Checkout, Element, Kind};
use serde_json::{Value, json};

/// What a read of a page asking for card details answers: no page
/// text, and the run stops here.
pub(super) const CARD_NOTE: &str = "This page asks for card details. Niles never asks for, keeps or types those, so the run stops here. Tell the person in one short sentence that the site wants card details, and that Niles pays only with what their account on the site has saved. Never ask them for card details, and do nothing more on this page.";

/// Why a held step whose page now asks for card details lapses: a
/// fixed sentence in the person's words, with no page text.
pub(super) const CARD_STOP: &str = "The page now asks for card details, which Niles never types, so nothing was done. Niles pays only with what your account on the site has saved.";

/// Why text holding a card number is never typed.
pub(super) const CARD_NUMBER: &str = "That text holds a card number, and Niles never types card details. It pays only with what the site's account has saved, so never ask the person for card details.";

/// What the order card shows when the page script found no total or
/// no delivery address there.
const NOT_FOUND: &str = "Niles couldn't find it on the page";

/// Words that place an order or pay wherever they are in a label, in
/// English and Danish, the household's shops; another language's order
/// button is caught by the checkout around it.
const ORDER_WORDS: &[&str] = &["pay", "buy", "purchase", "betal", "køb", "bestil"];

/// Word sequences that place an order or pay. "order" also counts as
/// the label's first word ("Order", "Order now").
const ORDER_PHRASES: &[&[&str]] = &[
    &["place", "order"],
    &["place", "your", "order"],
    &["place", "my", "order"],
    &["submit", "order"],
    &["submit", "your", "order"],
    &["complete", "order"],
    &["complete", "your", "order"],
    &["complete", "payment"],
    &["confirm", "order"],
    &["confirm", "your", "order"],
    &["confirm", "payment"],
    &["submit", "payment"],
    &["make", "payment"],
    &["finish", "order"],
    &["finalize", "order"],
    &["finalise", "order"],
    &["afgiv", "ordre"],
    &["afgiv", "bestilling"],
    &["gennemfør", "ordre"],
    &["gennemfør", "bestilling"],
    &["gennemfør", "betaling"],
    &["godkend", "ordre"],
    &["godkend", "betaling"],
    &["bekræft", "ordre"],
    &["bekræft", "bestilling"],
    &["send", "ordre"],
    &["send", "bestilling"],
];

/// Words that make a label about looking, not ordering: "Order
/// history", "Ways to pay".
const LOOKING: &[&str] = &[
    "history",
    "status",
    "details",
    "summary",
    "track",
    "tracking",
    "again",
    "ways",
    "how",
    "why",
    "options",
    "methods",
    "info",
    "information",
    "help",
    "terms",
    "policy",
    "faq",
    "learn",
    "read",
    "more",
    "historik",
    "detaljer",
    "oversigt",
    "igen",
    "vilkår",
    "hjælp",
    "læs",
    "mere",
];

/// Words that make a button on a checkout page plainly something other
/// than the order.
const ELSEWHERE: &[&str] = &[
    "add",
    "apply",
    "remove",
    "delete",
    "edit",
    "change",
    "update",
    "back",
    "cancel",
    "close",
    "sign",
    "log",
    "login",
    "logout",
    "show",
    "hide",
    "select",
    "choose",
    "use",
    "save",
    "cookie",
    "cookies",
    "accept",
    "reject",
    "decline",
    "allow",
    "menu",
    "search",
    "view",
    "open",
    "expand",
    "collapse",
    "increase",
    "decrease",
    "quantity",
    "code",
    "coupon",
    "voucher",
    "discount",
    "promo",
    "gift",
    "newsletter",
    "shopping",
    "basket",
    "cart",
    "tilføj",
    "anvend",
    "fjern",
    "slet",
    "ret",
    "rediger",
    "skift",
    "opdater",
    "tilbage",
    "annuller",
    "fortryd",
    "luk",
    "vis",
    "skjul",
    "vælg",
    "brug",
    "gem",
    "accepter",
    "afvis",
    "tillad",
    "søg",
    "rabatkode",
    "gavekort",
    "kode",
    "kurv",
    "indkøbskurv",
    "handle",
];

/// The order card's facts as the page had them when the card was made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Order {
    pub(super) total: Option<String>,
    pub(super) delivery: Option<String>,
}

/// Whether the step is the one that places an order or pays: decided
/// from the element's label and the checkout around it, erring
/// towards the order card. `page` is what the page script read.
#[must_use]
pub(super) fn of(element: &Element, action: &Action, page: &Checkout) -> Option<Order> {
    let words = words_of(&element.label);
    let looking = has_any(&words, LOOKING);
    let ordered = match action {
        Action::Click => {
            let by_label = matches!(element.kind, Kind::Button | Kind::Link | Kind::Other)
                && !looking
                && says_order(&words);
            // On a checkout, any button but a plainly other one: errs
            // towards the order card. Links navigate, so only their
            // label counts.
            let at_checkout = page.at_checkout
                && matches!(element.kind, Kind::Button)
                && !looking
                && !has_any(&words, ELSEWHERE);
            by_label || at_checkout
        }
        // Submitting a checkout's form is the order whatever field it is sent from.
        Action::Submit => page.at_checkout || (!looking && says_order(&words)),
        Action::Type { .. } | Action::Choose { .. } => false,
    };
    ordered.then(|| Order {
        total: fact(page.total.as_deref(), 60),
        delivery: fact(page.delivery.as_deref(), 200),
    })
}

fn words_of(label: &str) -> Vec<String> {
    label
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|piece| !piece.is_empty())
        .map(str::to_string)
        .collect()
}

fn has_any(words: &[String], list: &[&str]) -> bool {
    words.iter().any(|word| list.contains(&word.as_str()))
}

fn says_order(words: &[String]) -> bool {
    words
        .iter()
        .any(|word| ORDER_WORDS.contains(&word.as_str()))
        || words.first().is_some_and(|word| word == "order")
        || ORDER_PHRASES.iter().any(|phrase| {
            phrase.len() <= words.len()
                && words.windows(phrase.len()).any(|window| window == *phrase)
        })
}

/// A fact as the card shows it: squeezed, `None` when empty. The page
/// script already capped what it read.
fn fact(value: Option<&str>, max: usize) -> Option<String> {
    let squeezed: String = value?.split_whitespace().collect::<Vec<_>>().join(" ");
    if squeezed.is_empty() {
        return None;
    }
    Some(step::cut(&squeezed, max))
}

/// The order card: "Place this order on {host}?", with the total, the
/// delivery address, the page, the element and the model's reason.
pub(super) fn card(order: &Order, element: &Element, address: &str, reason: &str) -> ApprovalCard {
    let site = step::site_of(address);
    ApprovalCard {
        id: String::new(),
        title: format!(
            "Place this order on {}?",
            site.as_deref().unwrap_or("this site")
        ),
        subtitle: site.unwrap_or_else(|| "this page".into()),
        icon: Some("browser".into()),
        fields: vec![
            CardField {
                label: "Total".into(),
                value: order.total.clone().unwrap_or_else(|| NOT_FOUND.into()),
            },
            CardField {
                label: "Delivery address".into(),
                value: order.delivery.clone().unwrap_or_else(|| NOT_FOUND.into()),
            },
            CardField {
                label: "Page".into(),
                value: step::page_of(address),
            },
            CardField {
                label: "Element".into(),
                value: format!("\"{}\" ({})", step::label_of(element), element.kind.word()),
            },
            CardField {
                label: "The model's reason".into(),
                value: reason.to_string(),
            },
        ],
        body: None,
        allow_conversation: false,
        picture: true,
        status: ApprovalStatus::Pending,
        outcome: None,
    }
}

/// What a step on a page asking for card details answers: the stop
/// note, holding no card.
pub(super) fn stopped() -> Value {
    json!({ "card_details": true, "note": CARD_NOTE })
}

/// Whether `text` holds a card number: a run of 13 to 19 digits,
/// spaces and dashes allowed between them, that passes the Luhn check.
#[must_use]
pub(super) fn card_number(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_digit() && c != ' ' && c != '-')
        .filter_map(|run| {
            let digits: Vec<u32> = run.chars().filter_map(|c| c.to_digit(10)).collect();
            (13..=19).contains(&digits.len()).then_some(digits)
        })
        .any(|digits| luhn(&digits))
}

fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, digit)| {
            if i % 2 == 1 {
                let doubled = digit * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                *digit
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{self, Approvals, Via};
    use crate::browsing::fake::{self, FakeDriver, FakeTab};
    use crate::browsing::{self, step, tools};
    use niles_api::chat::{ApprovalStatus, Decision};
    use niles_core::EventBus;
    use niles_logins::{LoginStore, MemoryLogins, NewLogin, Secret};
    use niles_tools::Tool;
    use std::sync::Arc;

    const MARK: &str = "person:mark";
    const HER_USER: &str = "mark.scott+tellnoone@example.com";
    const HER_PASSWORD: &str = "correct horse battery staple 42";
    const SHOP: &str = "https://shop.example";
    const CHECKOUT: &str = "https://shop.example/checkout";

    fn placed() -> Checkout {
        Checkout {
            at_checkout: true,
            total: Some("412,50 kr.".into()),
            delivery: Some("Mark Nygaard, Street 1, 2100 Copenhagen".into()),
            card_details: false,
        }
    }

    #[test]
    fn an_order_label_is_an_order_wherever_it_is() {
        for label in [
            "Place order",
            "Place your order",
            "Pay now",
            "Pay 412,50 kr.",
            "Buy now",
            "Complete purchase",
            "Confirm and pay",
            "Order now",
            "Order",
            "Gennemfør køb",
            "Betal",
            "Bestil og betal",
            "Køb nu",
        ] {
            assert!(
                of(
                    &fake::button(1, 100, label),
                    &Action::Click,
                    &Checkout::default()
                )
                .is_some(),
                "{label}"
            );
        }
    }

    #[test]
    fn looking_and_other_buttons_are_not_orders() {
        for label in [
            "Order history",
            "Order details",
            "Track order",
            "Ways to pay",
            "Buy it again",
            "Add to basket",
            "PayPal",
            "Payment options",
            "Læg i kurv",
            "Continue",
        ] {
            assert!(
                of(
                    &fake::button(1, 100, label),
                    &Action::Click,
                    &Checkout::default()
                )
                .is_none(),
                "{label}"
            );
        }
    }

    #[test]
    fn at_a_checkout_almost_any_button_is_the_order() {
        let page = Checkout {
            at_checkout: true,
            ..Checkout::default()
        };
        for label in ["Continue", "Fortsæt", "", "Subscribe"] {
            assert!(
                of(&fake::button(1, 100, label), &Action::Click, &page).is_some(),
                "{label}"
            );
        }
        for label in [
            "Apply",
            "Change address",
            "Remove",
            "Back to basket",
            "Sign in",
            "Use this address",
            "Accept cookies",
            "Vælg tid",
            "Skift adresse",
            "Continue shopping",
        ] {
            assert!(
                of(&fake::button(1, 100, label), &Action::Click, &page).is_none(),
                "{label}"
            );
        }
        let mut link = fake::button(1, 100, "Terms and conditions");
        link.kind = Kind::Link;
        assert!(of(&link, &Action::Click, &page).is_none());
        let mut checkbox = fake::button(1, 100, "I accept the terms");
        checkbox.kind = Kind::Checkbox;
        assert!(of(&checkbox, &Action::Click, &page).is_none());
        let mut field = fake::textbox(2, 200, "Email");
        field.in_form = true;
        assert!(of(&field, &Action::Submit, &page).is_some());
        assert!(
            of(
                &fake::textbox(2, 200, "Email"),
                &Action::Type { text: "x".into() },
                &page
            )
            .is_none()
        );
        assert!(
            of(
                &field,
                &Action::Choose {
                    choice: niles_browser::Choice {
                        label: "x".into(),
                        value: "x".into(),
                    },
                },
                &page
            )
            .is_none()
        );
    }

    #[test]
    fn the_order_keeps_the_pages_total_and_address_squeezed_and_none_when_empty() {
        let page = Checkout {
            at_checkout: true,
            total: Some("  412,50\n kr. ".into()),
            delivery: Some("   ".into()),
            card_details: false,
        };
        let order =
            of(&fake::button(1, 100, "Place order"), &Action::Click, &page).expect("an order");
        assert_eq!(order.total.as_deref(), Some("412,50 kr."));
        assert_eq!(order.delivery, None);
    }

    #[test]
    fn a_card_number_is_seen_in_any_grouping() {
        for text in [
            "4242 4242 4242 4242",
            "4242-4242-4242-4242",
            "4111111111111111",
            "378282246310005",
            "my card is 4242 4242 4242 4242, thanks",
        ] {
            assert!(card_number(text), "{text}");
        }
        for text in [
            "4242 4242 4242 4241",
            "1234567890123",
            "+45 12 34 56 78",
            "2026-11-09",
            "12345678901234567890123",
        ] {
            assert!(!card_number(text), "{text}");
        }
    }

    struct Harness {
        browsing: Arc<browsing::Browsing>,
        approvals: Arc<Approvals>,
        driver: Arc<FakeDriver>,
    }

    fn harness() -> Harness {
        let approvals = Arc::new(Approvals::new(EventBus::default(), None));
        let driver = Arc::new(FakeDriver::default());
        let browsing = browsing::with_driver(approvals.clone(), driver.clone());
        Harness {
            browsing,
            approvals,
            driver,
        }
    }

    fn asker() -> approvals::Asker {
        approvals::Asker {
            person: MARK.into(),
            slug: Some("mark".into()),
            satellite: None,
            conversation: Some(1),
            in_chat: true,
        }
    }

    impl Harness {
        fn tab(&self) -> Arc<FakeTab> {
            self.driver
                .tabs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .expect("a session was made")
                .clone()
        }

        async fn as_mark<R>(&self, f: impl std::future::Future<Output = R>) -> R {
            approvals::ASKER.scope(asker(), f).await
        }

        async fn open(&self, url: &str) {
            self.as_mark(tools::OpenPage(self.browsing.clone()).execute(json!({ "url": url })))
                .await
                .expect("opens");
        }

        /// What the page shows now, and the element on it.
        fn set(&self, checkout: Checkout, element: Element) {
            let tab = self.tab();
            let mut state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
            state.checkout = checkout;
            state.elements.insert(element.reference.0, element);
        }

        async fn click(&self, reference: u32) -> niles_tools::Result<Value> {
            self.as_mark(tools::act(
                &self.browsing,
                step::ActKind::Click,
                json!({ "ref": reference, "reason": "place the order" }),
            ))
            .await
        }

        async fn type_into(&self, reference: u32, text: &str) -> niles_tools::Result<Value> {
            self.as_mark(tools::act(
                &self.browsing,
                step::ActKind::Type,
                json!({ "ref": reference, "text": text, "reason": "place the order" }),
            ))
            .await
        }

        async fn held(&self) -> niles_api::chat::ApprovalCard {
            let pending = self.approvals.pending(MARK);
            assert_eq!(pending.len(), 1, "one card waits");
            pending[0].card.clone()
        }

        /// Answer the waiting card; answers what settle did.
        async fn answer(&self, decision: Decision) -> (niles_api::chat::ApprovalCard, String) {
            let pending = self.approvals.pending(MARK);
            assert_eq!(pending.len(), 1, "one card waits");
            let id = pending[0].card.id.clone();
            let held = self.approvals.take(&id, MARK).expect("waiting");
            self.approvals
                .settle(held, decision, Via::App, Some(1))
                .await
        }
    }

    #[tokio::test]
    async fn an_order_step_gets_the_order_card_with_the_total_and_the_address() {
        let h = harness();
        h.open(CHECKOUT).await;
        h.set(placed(), fake::button(1, 100, "Place order"));
        let said = h.click(1).await.expect("held");
        assert_eq!(
            said["shown_to_the_person"],
            "Place this order on shop.example?"
        );
        let card = h.held().await;
        assert_eq!(card.subtitle, "shop.example");
        assert_eq!(card.icon.as_deref(), Some("browser"));
        assert!(card.picture);
        assert!(!card.allow_conversation);
        let fields: Vec<(String, String)> = card
            .fields
            .iter()
            .map(|f| (f.label.clone(), f.value.clone()))
            .collect();
        assert_eq!(
            fields,
            vec![
                ("Total".to_string(), "412,50 kr.".to_string()),
                (
                    "Delivery address".to_string(),
                    "Mark Nygaard, Street 1, 2100 Copenhagen".to_string()
                ),
                ("Page".to_string(), CHECKOUT.to_string()),
                (
                    "Element".to_string(),
                    "\"Place order\" (button)".to_string()
                ),
                (
                    "The model's reason".to_string(),
                    "place the order".to_string()
                ),
            ]
        );
        assert_eq!(
            h.approvals.buttons(&card.id),
            Some(vec![
                niles_webpush::Action::Review,
                niles_webpush::Action::Deny
            ])
        );
        assert!(
            h.tab()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .done
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_order_card_says_when_it_cant_find_the_total_or_the_address() {
        let h = harness();
        h.open(CHECKOUT).await;
        h.set(
            Checkout {
                at_checkout: true,
                ..Checkout::default()
            },
            fake::button(1, 100, "Place order"),
        );
        h.click(1).await.expect("held");
        let card = h.held().await;
        assert_eq!(card.fields[0].label, "Total");
        assert_eq!(card.fields[0].value, NOT_FOUND);
        assert_eq!(card.fields[1].label, "Delivery address");
        assert_eq!(card.fields[1].value, NOT_FOUND);
    }

    #[tokio::test]
    async fn allowed_the_order_card_places_the_order() {
        let h = harness();
        h.open(CHECKOUT).await;
        h.set(placed(), fake::button(1, 100, "Place order"));
        h.click(1).await.expect("held");
        let pending = h.approvals.pending(MARK);
        let id = pending[0].card.id.clone();
        let held = h.approvals.take(&id, MARK).expect("waiting");
        let then = held.then();
        let (card, reply) = h
            .approvals
            .settle(held, Decision::Once, Via::App, Some(1))
            .await;
        assert_eq!(card.status, ApprovalStatus::Allowed);
        assert_eq!(reply, "Done. Clicked it.");
        assert_eq!(
            h.tab().state.lock().unwrap_or_else(|e| e.into_inner()).done,
            ["click"]
        );
        assert!(crate::approvals::goes_on(then, &card));
    }

    #[tokio::test]
    async fn a_changed_total_lapses_the_order_card_and_a_new_card_shows_it() {
        let h = harness();
        h.open(CHECKOUT).await;
        h.set(placed(), fake::button(1, 100, "Place order"));
        h.click(1).await.expect("held");
        h.tab()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .checkout
            .total = Some("512,50 kr.".into());
        let (card, reply) = h.answer(Decision::Once).await;
        assert_eq!(card.status, ApprovalStatus::Lapsed);
        assert!(reply.contains("page changed"), "{reply}");
        assert!(
            h.tab()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .done
                .is_empty()
        );
        let pending = h.approvals.pending(MARK);
        assert_eq!(pending.len(), 1, "the new card waits");
        assert_eq!(pending[0].card.fields[0].value, "512,50 kr.");
    }

    #[tokio::test]
    async fn a_page_that_turns_into_a_checkout_gets_the_order_card_on_allow() {
        let h = harness();
        h.open(CHECKOUT).await;
        h.set(Checkout::default(), fake::button(1, 100, "Continue"));
        let said = h.click(1).await.expect("held");
        assert_eq!(said["shown_to_the_person"], "Click \"Continue\"?");
        h.tab()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .checkout
            .at_checkout = true;
        let (card, reply) = h.answer(Decision::Once).await;
        assert_eq!(card.status, ApprovalStatus::Lapsed);
        assert!(reply.contains("page changed"), "{reply}");
        let pending = h.approvals.pending(MARK);
        assert_eq!(pending.len(), 1, "the new card waits");
        assert_eq!(pending[0].card.title, "Place this order on shop.example?");
    }

    #[tokio::test]
    async fn a_card_details_form_stops_the_run() {
        let h = harness();
        h.open(CHECKOUT).await;
        h.set(
            Checkout {
                card_details: true,
                ..placed()
            },
            fake::button(1, 100, "Pay now"),
        );
        h.tab()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elements
            .insert(2, fake::textbox(2, 201, "Card number"));
        let read = h
            .as_mark(tools::ReadPage(h.browsing.clone()).execute(json!({})))
            .await
            .expect("read");
        assert_eq!(read["card_details"], true);
        assert_eq!(read["note"], CARD_NOTE);
        assert!(read.get("page").is_none(), "no page text: {read}");
        let clicked = h.click(1).await.expect("the stop is not an error");
        assert_eq!(clicked["card_details"], true);
        assert_eq!(clicked["note"], CARD_NOTE);
        let typed = h.type_into(2, "Mark Nygaard").await.expect("the stop");
        assert_eq!(typed["card_details"], true);
        assert_eq!(typed["note"], CARD_NOTE);
        assert!(h.approvals.pending(MARK).is_empty(), "no card is held");
        assert!(!h.browsing.waiting_step(MARK));
        assert!(
            h.tab()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .done
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_card_details_form_at_allow_lapses_the_step_and_ends_the_run() {
        let h = harness();
        h.open(CHECKOUT).await;
        h.set(placed(), fake::button(1, 100, "Place order"));
        h.click(1).await.expect("held");
        h.tab()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .checkout
            .card_details = true;
        let (card, reply) = h.answer(Decision::Once).await;
        assert_eq!(card.status, ApprovalStatus::Lapsed);
        assert_eq!(reply, CARD_STOP);
        assert!(
            h.tab()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .done
                .is_empty()
        );
        assert!(h.approvals.pending(MARK).is_empty(), "no new card");
    }

    #[tokio::test]
    async fn typing_a_card_number_is_refused_and_another_long_number_is_not() {
        let h = harness();
        h.open(CHECKOUT).await;
        h.set(placed(), fake::textbox(1, 100, "Notes"));
        let refused = h
            .type_into(1, "4242 4242 4242 4242")
            .await
            .expect_err("refused");
        assert!(refused.to_string().contains("card number"), "{refused}");
        assert!(h.approvals.pending(MARK).is_empty(), "no card is held");
        h.type_into(1, "1234567890123").await.expect("held");
        h.held().await;
    }

    #[tokio::test]
    async fn an_order_card_holds_no_saved_value() {
        let approvals = Arc::new(Approvals::new(EventBus::default(), None));
        let driver = Arc::new(FakeDriver::default());
        let store = Arc::new(MemoryLogins::new());
        let new = NewLogin::new(
            "Shop",
            &[SHOP.to_string()],
            Secret::new(HER_USER),
            Secret::new(HER_PASSWORD),
        )
        .expect("a valid login");
        store.add(MARK, &new).await.expect("kept");
        let browsing = browsing::with_store(approvals.clone(), driver.clone(), store);
        let h = Harness {
            browsing,
            approvals,
            driver,
        };
        h.open(CHECKOUT).await;
        h.set(
            Checkout {
                delivery: Some(format!("Mark, {HER_USER}")),
                ..placed()
            },
            fake::button(1, 100, "Place order"),
        );
        h.click(1).await.expect("held");
        let card = h.held().await;
        let json = serde_json::to_string(&card).expect("card");
        assert!(
            !json.to_uppercase().contains(&HER_USER.to_uppercase()),
            "the delivery address is scrubbed: {json}"
        );
        assert!(json.contains("•••"), "masked, not gone: {json}");
    }
}
