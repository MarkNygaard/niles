use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Whose data this is: the voice their sign-in is linked to when it is,
/// so what they say to a satellite and what they type meet in one
/// place; otherwise the address they signed in with.
///
/// The chat history keys people the same way, through this function,
/// so the two cannot drift apart.
pub fn person_key(speaker: Option<&str>, email: &str) -> String {
    match speaker {
        Some(slug) => format!("person:{slug}"),
        None => format!("email:{email}"),
    }
}

/// What sort of workout, as far as a training plan cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Kind {
    Run,
    Walk,
    Ride,
    Strength,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Run => "run",
            Kind::Walk => "walk",
            Kind::Ride => "ride",
            Kind::Strength => "strength",
            Kind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "run" | "runs" | "running" => Some(Kind::Run),
            "walk" | "walks" | "walking" | "hike" | "hiking" => Some(Kind::Walk),
            "ride" | "rides" | "cycling" | "bike" => Some(Kind::Ride),
            "strength" => Some(Kind::Strength),
            "other" => Some(Kind::Other),
            _ => None,
        }
    }

    /// From the workout's name as Apple Health has it: "Outdoor Run",
    /// "Indoor Cycle", "Traditional Strength Training", "Hiking".
    pub fn from_workout_name(name: &str) -> Kind {
        let name = name.to_ascii_lowercase();
        let has = |w: &str| name.contains(w);
        if has("run") || has("jog") {
            Kind::Run
        } else if has("walk") || has("hik") {
            Kind::Walk
        } else if has("cycl") || has("bik") || has("ride") {
            Kind::Ride
        } else if has("strength") || has("weight") {
            Kind::Strength
        } else {
            Kind::Other
        }
    }
}

/// One workout, in canonical units: metres, seconds, beats per minute,
/// kilocalories.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Activity {
    /// Where it came from: `health_auto_export`.
    pub source: String,
    /// The source's own id for it, unique per person and source: the
    /// same workout sent twice is the same row.
    pub external_id: String,
    pub started_at: DateTime<Utc>,
    pub kind: Kind,
    /// As the source named it, "Outdoor Run": the kind loses whether it
    /// was on a treadmill.
    pub name: String,
    pub distance_m: Option<f64>,
    pub duration_s: Option<f64>,
    pub moving_s: Option<f64>,
    pub avg_hr: Option<f64>,
    pub max_hr: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub energy_kcal: Option<f64>,
    /// The workout as it arrived, for the plan and the review to come:
    /// splits, heart-rate samples and the route are in here.
    #[serde(skip)]
    pub raw: serde_json::Value,
}

impl Activity {
    /// The time a pace is worked out over: moving time when the source
    /// knows it, otherwise the whole of it.
    pub fn pace_seconds(&self) -> Option<f64> {
        self.moving_s.or(self.duration_s).filter(|s| *s > 0.0)
    }
}

/// One day of a person's body: what the watch and the scale said.
/// Every field is optional because each arrives on its own.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Wellness {
    pub date: NaiveDate,
    pub resting_hr: Option<f64>,
    pub hrv_ms: Option<f64>,
    pub sleep_s: Option<f64>,
    pub weight_kg: Option<f64>,
    pub body_fat_pct: Option<f64>,
}

impl Wellness {
    pub fn on(date: NaiveDate) -> Self {
        Self {
            date,
            ..Self::default()
        }
    }

    /// `newer`'s readings over these, keeping what `newer` does not say.
    pub fn merge(&mut self, newer: &Wellness) {
        let take = |old: &mut Option<f64>, new: Option<f64>| {
            if new.is_some() {
                *old = new;
            }
        };
        take(&mut self.resting_hr, newer.resting_hr);
        take(&mut self.hrv_ms, newer.hrv_ms);
        take(&mut self.sleep_s, newer.sleep_s);
        take(&mut self.weight_kg, newer.weight_kg);
        take(&mut self.body_fat_pct, newer.body_fat_pct);
    }

    pub fn is_empty(&self) -> bool {
        self.resting_hr.is_none()
            && self.hrv_ms.is_none()
            && self.sleep_s.is_none()
            && self.weight_kg.is_none()
            && self.body_fat_pct.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_and_the_address_linked_to_it_are_one_person() {
        assert_eq!(person_key(Some("majse"), "majse@example.com"), "person:majse");
        assert_eq!(person_key(None, "majse@example.com"), "email:majse@example.com");
        assert_ne!(
            person_key(None, "mark@example.com"),
            person_key(None, "majse@example.com")
        );
    }

    #[test]
    fn workout_names_become_kinds() {
        for (name, kind) in [
            ("Outdoor Run", Kind::Run),
            ("Indoor Run", Kind::Run),
            ("Running", Kind::Run),
            ("Outdoor Walk", Kind::Walk),
            ("Hiking", Kind::Walk),
            ("Indoor Cycle", Kind::Ride),
            ("Traditional Strength Training", Kind::Strength),
            ("Functional Strength Training", Kind::Strength),
            ("Yoga", Kind::Other),
            ("Pool Swim", Kind::Other),
        ] {
            assert_eq!(Kind::from_workout_name(name), kind, "{name}");
        }
    }

    #[test]
    fn a_newer_reading_wins_and_a_missing_one_keeps_the_old() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        let mut kept = Wellness {
            resting_hr: Some(52.0),
            weight_kg: Some(61.0),
            ..Wellness::on(day)
        };
        kept.merge(&Wellness {
            weight_kg: Some(60.6),
            ..Wellness::on(day)
        });
        assert_eq!(kept.resting_hr, Some(52.0));
        assert_eq!(kept.weight_kg, Some(60.6));
    }
}
