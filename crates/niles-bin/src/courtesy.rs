//! When Niles is polite, so that it stays pleasant rather than a tic.
//!
//! "Lights off, Sir." is charming once and grating the fourth time in a
//! minute; Alexa's "Good morning" is nice because it comes once. So both
//! are rationed per person: the form of address at most every half hour
//! on a short confirmation, and a greeting on the first thing said to
//! Niles in a morning.
//!
//! Kept in memory. A restart forgets who has been greeted, which costs at
//! worst a second "good morning" — not worth a table.

use chrono::NaiveDate;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long after "Sir" before a confirmation says it again.
pub const ADDRESS_EVERY: Duration = Duration::from_secs(30 * 60);

/// Greetings are for mornings: from 04:00 until noon, local time.
const MORNING: std::ops::Range<u32> = 4..12;

#[derive(Default)]
struct Seen {
    addressed_at: Option<Instant>,
    greeted_on: Option<NaiveDate>,
}

#[derive(Default)]
pub struct Courtesy {
    people: Mutex<HashMap<String, Seen>>,
}

impl Courtesy {
    /// Whether a short confirmation to `who` should end with their form
    /// of address now — and if so, that it did.
    pub fn address_now(&self, who: &str, now: Instant) -> bool {
        let mut people = self.people.lock().unwrap_or_else(|e| e.into_inner());
        let seen = people.entry(who.to_string()).or_default();
        let due = seen
            .addressed_at
            .is_none_or(|at| now.duration_since(at) >= ADDRESS_EVERY);
        if due {
            seen.addressed_at = Some(now);
        }
        due
    }

    /// Whether this is `who`'s first word to Niles this morning — and if
    /// so, that they have now been greeted. A greeting addresses them, so
    /// it also counts as the "Sir".
    pub fn greet_now(&self, who: &str, today: NaiveDate, hour: u32, now: Instant) -> bool {
        if !MORNING.contains(&hour) {
            return false;
        }
        let mut people = self.people.lock().unwrap_or_else(|e| e.into_inner());
        let seen = people.entry(who.to_string()).or_default();
        if seen.greeted_on == Some(today) {
            return false;
        }
        seen.greeted_on = Some(today);
        seen.addressed_at = Some(now);
        true
    }

    /// Take back a greeting that was never spoken — the turn was dropped
    /// — so the first thing they say that *is* answered gets it.
    pub fn ungreet(&self, who: &str) {
        let mut people = self.people.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(seen) = people.get_mut(who) {
            seen.greeted_on = None;
            seen.addressed_at = None;
        }
    }
}

/// "Good morning, Sir. Living room lights on." — and on their birthday,
/// "Good morning, and happy birthday, Sir."
///
/// Left alone when the reply already says it — "Niles, good morning" is
/// answered by the language model in kind, and two in a row is one too
/// many. The same goes for the birthday: the model knows the date too.
pub fn greeted(reply: &str, whom: &str, birthday: bool) -> String {
    let said = reply.to_lowercase();
    if said.contains("good morning") {
        return reply.to_string();
    }
    if birthday && !said.contains("happy birthday") {
        return format!("Good morning, and happy birthday, {whom}. {reply}");
    }
    format!("Good morning, {whom}. {reply}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn sir_comes_once_and_then_rests() {
        let c = Courtesy::default();
        let t = Instant::now();
        assert!(c.address_now("Mark", t));
        assert!(!c.address_now("Mark", t + Duration::from_secs(60)));
        assert!(!c.address_now("Mark", t + Duration::from_secs(29 * 60)));
        assert!(c.address_now("Mark", t + ADDRESS_EVERY));
    }

    #[test]
    fn each_person_has_their_own_turn() {
        let c = Courtesy::default();
        let t = Instant::now();
        assert!(c.address_now("Mark", t));
        assert!(c.address_now("Majse", t));
    }

    #[test]
    fn good_morning_is_said_once_a_morning() {
        let c = Courtesy::default();
        let t = Instant::now();
        assert!(c.greet_now("Mark", day(3), 7, t));
        assert!(!c.greet_now("Mark", day(3), 8, t));
        assert!(
            c.greet_now("Mark", day(4), 7, t),
            "tomorrow is another morning"
        );
    }

    #[test]
    fn nobody_is_greeted_good_morning_in_the_evening() {
        let c = Courtesy::default();
        assert!(!c.greet_now("Mark", day(3), 19, Instant::now()));
        assert!(!c.greet_now("Mark", day(3), 2, Instant::now()));
    }

    #[test]
    fn a_greeting_counts_as_the_sir() {
        let c = Courtesy::default();
        let t = Instant::now();
        assert!(c.greet_now("Mark", day(3), 7, t));
        assert!(
            !c.address_now("Mark", t),
            "not 'Good morning, Sir. Lights on, Sir.'"
        );
    }

    #[test]
    fn a_dropped_turn_does_not_use_up_the_greeting() {
        let c = Courtesy::default();
        let t = Instant::now();
        assert!(c.greet_now("Mark", day(3), 7, t));
        c.ungreet("Mark");
        assert!(c.greet_now("Mark", day(3), 7, t));
    }

    #[test]
    fn the_greeting_goes_first() {
        assert_eq!(
            greeted("Living room lights on.", "Sir", false),
            "Good morning, Sir. Living room lights on."
        );
    }

    #[test]
    fn on_their_birthday_the_greeting_says_so() {
        assert_eq!(
            greeted("Living room lights on.", "Sir", true),
            "Good morning, and happy birthday, Sir. Living room lights on."
        );
        assert_eq!(
            greeted("Happy birthday, Sir! Lights on.", "Sir", true),
            "Good morning, Sir. Happy birthday, Sir! Lights on."
        );
    }

    #[test]
    fn a_reply_that_already_says_good_morning_is_left_alone() {
        assert_eq!(
            greeted("Good morning, Sir! Lovely day.", "Sir", true),
            "Good morning, Sir! Lovely day."
        );
    }
}
