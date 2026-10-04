//! Birthdays as they are kept: "MM-DD", or "YYYY-MM-DD" when somebody has
//! said the year. The year is optional because some would rather Niles
//! did not count.

use chrono::{Datelike, NaiveDate};

/// The year, if given, then month and day — for a day that exists.
pub fn parse(birthday: &str) -> Option<(Option<i32>, u32, u32)> {
    let parts: Vec<&str> = birthday.trim().split('-').collect();
    let (year, month, day) = match parts.as_slice() {
        [m, d] => (None, m.parse().ok()?, d.parse().ok()?),
        [y, m, d] => (Some(y.parse().ok()?), m.parse().ok()?, d.parse().ok()?),
        _ => return None,
    };
    // A leap year when there is none, so 29 February is a birthday.
    NaiveDate::from_ymd_opt(year.unwrap_or(2024), month, day)?;
    Some((year, month, day))
}

/// The stored form of a birthday, or why it is not one. A year must be
/// one somebody alive could have been born in.
pub fn normalise(birthday: &str, this_year: i32) -> Result<String, String> {
    let bad = || format!("{birthday:?} is not a birthday — give it as MM-DD or YYYY-MM-DD");
    let (year, month, day) = parse(birthday).ok_or_else(bad)?;
    match year {
        Some(y) if !(1900..=this_year).contains(&y) => {
            Err(format!("{y} is not a year somebody here was born in"))
        }
        Some(y) => Ok(format!("{y:04}-{month:02}-{day:02}")),
        None => Ok(format!("{month:02}-{day:02}")),
    }
}

/// Whether `today` is the birthday. Somebody born on 29 February has it
/// on the 28th in the years without one, rather than not at all.
pub fn is_today(birthday: &str, today: NaiveDate) -> bool {
    let Some((_, month, day)) = parse(birthday) else {
        return false;
    };
    (today.month(), today.day()) == (month, day)
        || ((month, day) == (2, 29)
            && (today.month(), today.day()) == (2, 28)
            && today.with_day(29).is_none())
}

/// How old they are on `today`, when the year is known.
pub fn age(birthday: &str, today: NaiveDate) -> Option<u32> {
    let (year, month, day) = parse(birthday)?;
    let born = NaiveDate::from_ymd_opt(year?, month, day)?;
    today.years_since(born)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_year_is_optional() {
        assert_eq!(normalise("10-03", 2026).unwrap(), "10-03");
        assert_eq!(normalise("1990-10-3", 2026).unwrap(), "1990-10-03");
        assert_eq!(normalise("2-29", 2026).unwrap(), "02-29");
    }

    #[test]
    fn a_birthday_is_a_day_that_exists() {
        assert!(normalise("02-30", 2026).is_err());
        assert!(
            normalise("1991-02-29", 2026).is_err(),
            "1991 was not a leap year"
        );
        assert!(normalise("3 October", 2026).is_err());
        assert!(normalise("13-01", 2026).is_err());
    }

    #[test]
    fn nobody_is_born_in_the_future_or_long_ago() {
        assert!(normalise("2027-01-01", 2026).is_err());
        assert!(normalise("1850-01-01", 2026).is_err());
    }

    #[test]
    fn a_birthday_comes_once_a_year() {
        assert!(is_today("10-03", day(2026, 10, 3)));
        assert!(is_today("1990-10-03", day(2027, 10, 3)));
        assert!(!is_today("10-03", day(2026, 10, 4)));
    }

    #[test]
    fn a_leap_day_birthday_is_kept_in_other_years() {
        assert!(is_today("02-29", day(2028, 2, 29)));
        assert!(!is_today("02-29", day(2028, 2, 28)));
        assert!(is_today("2000-02-29", day(2027, 2, 28)));
    }

    #[test]
    fn age_turns_over_on_the_day() {
        assert_eq!(age("1990-10-03", day(2026, 10, 2)), Some(35));
        assert_eq!(age("1990-10-03", day(2026, 10, 3)), Some(36));
        assert_eq!(age("10-03", day(2026, 10, 3)), None);
    }
}
