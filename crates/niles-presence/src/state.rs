//! Core presence types.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Resolved household presence state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "lowercase")]
pub enum HomeState {
    Home,
    Away,
    Unknown,
}

/// Manual override that can supersede sensor readings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Override {
    Auto,
    ForceHome,
    ForceAway,
}

impl Override {
    /// Parse a caller-facing state string into an override.
    ///
    /// Accepts `"auto"`, `"home"`, and `"away"`.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(Override::Auto),
            "home" => Ok(Override::ForceHome),
            "away" => Ok(Override::ForceAway),
            other => Err(Error::Parse {
                reason: format!("unknown override '{other}', expected auto|home|away"),
            }),
        }
    }
}

/// A single reading from a presence source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceSignal {
    pub source: String,
    pub anyone_home: bool,
    pub observed_at: DateTime<Utc>,
}

/// A reading as it appears in a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReading {
    pub source: String,
    pub anyone_home: bool,
    pub observed_at: DateTime<Utc>,
}

/// Full presence state returned by the aggregator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceSnapshot {
    pub state: HomeState,
    pub r#override: Override,
    pub sources: Vec<SourceReading>,
}

impl From<HomeState> for niles_core::PresenceState {
    fn from(value: HomeState) -> Self {
        match value {
            HomeState::Home => niles_core::PresenceState::Home,
            HomeState::Away => niles_core::PresenceState::Away,
            HomeState::Unknown => niles_core::PresenceState::Unknown,
        }
    }
}

#[cfg(test)]
mod presence_state_conv_tests {
    use super::*;
    use niles_core::PresenceState;

    #[test]
    fn home_maps_to_home() {
        assert_eq!(PresenceState::from(HomeState::Home), PresenceState::Home);
    }

    #[test]
    fn away_maps_to_away() {
        assert_eq!(PresenceState::from(HomeState::Away), PresenceState::Away);
    }

    #[test]
    fn unknown_maps_to_unknown() {
        assert_eq!(
            PresenceState::from(HomeState::Unknown),
            PresenceState::Unknown
        );
    }
}
