use chrono::{DateTime, Utc};
use serde::Serialize;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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

    /// From Strava's `sport_type` (or the older `type`): "TrailRun",
    /// "GravelRide", "WeightTraining".
    pub fn from_sport_type(sport: &str) -> Kind {
        match sport {
            "Run" | "TrailRun" | "VirtualRun" => Kind::Run,
            "Walk" | "Hike" => Kind::Walk,
            "Ride" | "MountainBikeRide" | "GravelRide" | "EBikeRide" | "EMountainBikeRide"
            | "VirtualRide" | "Velomobile" | "Handcycle" => Kind::Ride,
            "WeightTraining" | "Crossfit" => Kind::Strength,
            _ => Kind::Other,
        }
    }
}

/// One workout, in canonical units: metres, seconds, beats per minute,
/// kilocalories.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Activity {
    /// Where it came from: `strava`.
    pub source: String,
    /// The source's own id for it, unique per person and source: the
    /// same workout fetched twice is the same row.
    pub external_id: String,
    pub started_at: DateTime<Utc>,
    pub kind: Kind,
    /// As the person named it, "Morning Run": the kind loses whether it
    /// was on a treadmill.
    pub name: String,
    pub distance_m: Option<f64>,
    /// From start to finish, pauses included.
    pub duration_s: Option<f64>,
    pub moving_s: Option<f64>,
    pub avg_hr: Option<f64>,
    pub max_hr: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub energy_kcal: Option<f64>,
    /// The workout as it arrived, for the plan and the review to come:
    /// the kilometre splits and the route are in here.
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

/// A person's own Strava API app, as they typed it on their profile.
/// Each person makes their own because a new Strava app may only be
/// used by the athlete who made it.
#[derive(Clone, PartialEq)]
pub struct StravaApp {
    pub client_id: String,
    pub client_secret: String,
}

impl std::fmt::Debug for StravaApp {
    /// Never the secret: a `Debug` that prints it is a secret in a log
    /// line waiting to happen.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StravaApp")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq)]
pub struct StravaTokens {
    pub access_token: String,
    /// Strava may hand back a new one on every refresh, and the old one
    /// stops working the moment it does.
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for StravaTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StravaTokens")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Whom the tokens read for, as Strava named them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Athlete {
    pub id: i64,
    pub name: String,
}

/// One person's Strava, as kept.
#[derive(Debug, Clone, PartialEq)]
pub struct StravaLink {
    pub app: StravaApp,
    /// `None` until they connect, and again once they disconnect.
    pub tokens: Option<StravaTokens>,
    pub athlete: Option<Athlete>,
    /// The last sync that got through.
    pub last_sync: Option<DateTime<Utc>>,
    /// Why the last sync did not, when it did not.
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_and_the_address_linked_to_it_are_one_person() {
        assert_eq!(
            person_key(Some("majse"), "majse@example.com"),
            "person:majse"
        );
        assert_eq!(
            person_key(None, "majse@example.com"),
            "email:majse@example.com"
        );
        assert_ne!(
            person_key(None, "mark@example.com"),
            person_key(None, "majse@example.com")
        );
    }

    #[test]
    fn sport_types_become_kinds() {
        for (sport, kind) in [
            ("Run", Kind::Run),
            ("TrailRun", Kind::Run),
            ("VirtualRun", Kind::Run),
            ("Walk", Kind::Walk),
            ("Hike", Kind::Walk),
            ("Ride", Kind::Ride),
            ("GravelRide", Kind::Ride),
            ("VirtualRide", Kind::Ride),
            ("WeightTraining", Kind::Strength),
            ("Yoga", Kind::Other),
            ("Swim", Kind::Other),
            ("SomethingNew", Kind::Other),
        ] {
            assert_eq!(Kind::from_sport_type(sport), kind, "{sport}");
        }
    }

    #[test]
    fn secrets_stay_out_of_debug() {
        let app = StravaApp {
            client_id: "12345".into(),
            client_secret: "the-secret".into(),
        };
        let tokens = StravaTokens {
            access_token: "the-access".into(),
            refresh_token: "the-refresh".into(),
            expires_at: Utc::now(),
        };
        let shown = format!("{app:?} {tokens:?}");
        assert!(shown.contains("12345"));
        for secret in ["the-secret", "the-access", "the-refresh"] {
            assert!(!shown.contains(secret), "{shown}");
        }
    }
}
