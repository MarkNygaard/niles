//! Time-of-day type used throughout the scheduler.

use crate::error::{Error, Result};
use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

/// Minutes since midnight, in the range `0..1440`.
///
/// The scheduler works in wall-clock time-of-day; date and timezone
/// belong above this layer. The lighting curve is daily-periodic so
/// only the minute matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MinuteOfDay(u16);

impl MinuteOfDay {
    /// Construct from hours (0–23) and minutes (0–59).
    pub fn new(hour: u8, minute: u8) -> Result<Self> {
        if hour >= 24 {
            return Err(Error::InvalidTime {
                reason: format!("hour {hour} >= 24"),
            });
        }
        if minute >= 60 {
            return Err(Error::InvalidTime {
                reason: format!("minute {minute} >= 60"),
            });
        }
        Ok(Self(u16::from(hour) * 60 + u16::from(minute)))
    }

    /// Total minutes since midnight (`0..1440`).
    pub fn total_minutes(self) -> u16 {
        self.0
    }

    pub fn hour(self) -> u8 {
        (self.0 / 60) as u8
    }

    pub fn minute(self) -> u8 {
        (self.0 % 60) as u8
    }
}

impl fmt::Display for MinuteOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour(), self.minute())
    }
}

impl FromStr for MinuteOfDay {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let (h_str, m_str) = s.split_once(':').ok_or_else(|| Error::InvalidTime {
            reason: format!("'{s}' missing ':' separator"),
        })?;
        let hour: u8 = h_str
            .parse()
            .map_err(|e: ParseIntError| Error::InvalidTime {
                reason: format!("hour: {e}"),
            })?;
        let minute: u8 = m_str
            .parse()
            .map_err(|e: ParseIntError| Error::InvalidTime {
                reason: format!("minute: {e}"),
            })?;
        Self::new(hour, minute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_accepts_valid() {
        let m = MinuteOfDay::new(13, 45).unwrap();
        assert_eq!(m.total_minutes(), 825);
        assert_eq!(m.hour(), 13);
        assert_eq!(m.minute(), 45);
    }

    #[test]
    fn new_rejects_out_of_range() {
        assert!(MinuteOfDay::new(24, 0).is_err());
        assert!(MinuteOfDay::new(0, 60).is_err());
    }

    #[test]
    fn display_pads_with_zeros() {
        assert_eq!(MinuteOfDay::new(5, 45).unwrap().to_string(), "05:45");
        assert_eq!(MinuteOfDay::new(0, 0).unwrap().to_string(), "00:00");
        assert_eq!(MinuteOfDay::new(23, 59).unwrap().to_string(), "23:59");
    }

    #[test]
    fn from_str_accepts_valid() {
        let m: MinuteOfDay = "05:45".parse().unwrap();
        assert_eq!(m, MinuteOfDay::new(5, 45).unwrap());
    }

    #[test]
    fn from_str_rejects_invalid() {
        assert!("".parse::<MinuteOfDay>().is_err());
        assert!("5".parse::<MinuteOfDay>().is_err());
        assert!("24:00".parse::<MinuteOfDay>().is_err());
        assert!("12:60".parse::<MinuteOfDay>().is_err());
        assert!("ab:cd".parse::<MinuteOfDay>().is_err());
    }

    #[test]
    fn ordering() {
        let a = MinuteOfDay::new(5, 45).unwrap();
        let b = MinuteOfDay::new(6, 30).unwrap();
        assert!(a < b);
    }
}
