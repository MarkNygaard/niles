//! iCloud's calendars over CalDAV (RFC 4791), signed in with the Apple
//! ID and an app-specific password.
//!
//! Apple offers nothing narrower than that password, which opens the
//! account's calendars, contacts and mail alike; read only is therefore
//! kept by the service above this, not by Apple.

use crate::error::{Error, Result};
use crate::graph::{first_line, http_error};
use crate::ics;
use crate::model::{Event, NewEvent};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;

pub const ICLOUD: &str = "https://caldav.icloud.com/";
const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";

/// The Apple ID and its app-specific password.
#[derive(Debug, Clone)]
pub struct Login {
    pub user: String,
    pub password: String,
}

/// Where the account keeps its calendars — what is stored with the
/// connection, so finding it again is not three requests per question.
/// Refuses a wrong password as such.
pub async fn discover(http: &reqwest::Client, login: &Login) -> Result<String> {
    let principal = propfind(
        http,
        login,
        ICLOUD,
        "0",
        r#"<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#,
    )
    .await?;
    let principal = href_in(&principal, DAV, "current-user-principal")
        .ok_or_else(|| refused(200, "iCloud named no account"))?;
    let principal = absolute(ICLOUD, &principal);
    let home = propfind(
        http,
        login,
        &principal,
        "0",
        r#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#,
    )
    .await?;
    let home = href_in(&home, CALDAV, "calendar-home-set")
        .ok_or_else(|| refused(200, "iCloud named no calendars for the account"))?;
    Ok(absolute(&principal, &home))
}

/// The account's calendars that hold events, by address — the default
/// one first, as the server lists them.
pub async fn calendars(http: &reqwest::Client, login: &Login, home: &str) -> Result<Vec<String>> {
    let body = propfind(
        http,
        login,
        home,
        "1",
        r#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:resourcetype/><c:supported-calendar-component-set/></d:prop></d:propfind>"#,
    )
    .await?;
    let doc = roxmltree::Document::parse(&body).map_err(|e| refused(207, &e.to_string()))?;
    let mut out = Vec::new();
    for response in doc
        .descendants()
        .filter(|n| n.has_tag_name((DAV, "response")))
    {
        let is_calendar = response
            .descendants()
            .filter(|n| n.has_tag_name((DAV, "resourcetype")))
            .any(|t| t.children().any(|c| c.has_tag_name((CALDAV, "calendar"))));
        if !is_calendar {
            continue;
        }
        // Reminders are calendars too, of to-dos. Only events here.
        let components: Vec<&str> = response
            .descendants()
            .filter(|n| n.has_tag_name((CALDAV, "comp")))
            .filter_map(|n| n.attribute("name"))
            .collect();
        if !components.is_empty() && !components.contains(&"VEVENT") {
            continue;
        }
        if let Some(href) = response
            .children()
            .find(|n| n.has_tag_name((DAV, "href")))
            .and_then(|n| n.text())
        {
            out.push(absolute(home, href.trim()));
        }
    }
    Ok(out)
}

/// The events overlapping `[from, to)` in every one of the account's
/// calendars, repeating ones expanded into their occurrences by the
/// server.
pub async fn events(
    http: &reqwest::Client,
    login: &Login,
    home: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tz: Tz,
) -> Result<Vec<Event>> {
    let (start, end) = (
        from.format("%Y%m%dT%H%M%SZ").to_string(),
        to.format("%Y%m%dT%H%M%SZ").to_string(),
    );
    let query = format!(
        r#"<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-data><c:expand start="{start}" end="{end}"/></c:calendar-data></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{start}" end="{end}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
    );
    let mut out = Vec::new();
    for calendar in calendars(http, login, home).await? {
        let body = dav(http, login, "REPORT", &calendar, "1", Some(query.clone())).await?;
        let doc = roxmltree::Document::parse(&body).map_err(|e| refused(207, &e.to_string()))?;
        for response in doc
            .descendants()
            .filter(|n| n.has_tag_name((DAV, "response")))
        {
            let Some(href) = response
                .children()
                .find(|n| n.has_tag_name((DAV, "href")))
                .and_then(|n| n.text())
            else {
                continue;
            };
            let Some(data) = response
                .descendants()
                .find(|n| n.has_tag_name((CALDAV, "calendar-data")))
                .and_then(|n| n.text())
            else {
                continue;
            };
            out.extend(ics::events(data, &absolute(&calendar, href.trim()), tz));
        }
    }
    // The window asked for, and nothing a loose server sent besides.
    let (from_local, to_local) = (
        from.with_timezone(&tz).naive_local(),
        to.with_timezone(&tz).naive_local(),
    );
    out.retain(|e| e.end > from_local && e.start < to_local);
    Ok(out)
}

/// A new event, in the account's first calendar of events.
pub async fn create(
    http: &reqwest::Client,
    login: &Login,
    home: &str,
    event: &NewEvent,
    tz: Tz,
) -> Result<Event> {
    let calendar = calendars(http, login, home)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| refused(404, "the account has no calendar of events"))?;
    let uid = format!(
        "{:016x}{:016x}@niles",
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    let href = format!(
        "{}/{}.ics",
        calendar.trim_end_matches('/'),
        uid.trim_end_matches("@niles")
    );
    put(http, login, &href, ics::build(&uid, event, tz), true).await?;
    Ok(Event {
        id: href,
        subject: event.subject.clone(),
        start: event.start,
        end: event.end,
        all_day: event.all_day,
        location: event.location.clone(),
        recurring: false,
    })
}

/// Change an event's time, title or place, keeping the rest of it.
/// Not one occurrence of a series: that is an exception written into
/// the series, which is not done from here.
pub async fn update(
    http: &reqwest::Client,
    login: &Login,
    id: &str,
    event: &NewEvent,
    tz: Tz,
) -> Result<Event> {
    let text = single(http, login, id, &event.subject).await?;
    put(http, login, id, ics::rewrite(&text, event, tz), false).await?;
    Ok(Event {
        id: id.to_string(),
        subject: event.subject.clone(),
        start: event.start,
        end: event.end,
        all_day: event.all_day,
        location: event.location.clone(),
        recurring: false,
    })
}

pub async fn delete(http: &reqwest::Client, login: &Login, id: &str, what: &str) -> Result<()> {
    single(http, login, id, what).await?;
    dav(http, login, "DELETE", id, "0", None).await.map(|_| ())
}

/// The resource behind an event, refused when it is a series — or one
/// occurrence of one, whose id carries its RECURRENCE-ID.
async fn single(http: &reqwest::Client, login: &Login, id: &str, what: &str) -> Result<String> {
    let repeating = || Error::Repeating {
        what: what.to_string(),
    };
    if id.contains('#') {
        return Err(repeating());
    }
    let text = dav(http, login, "GET", id, "0", None).await?;
    if ics::repeats(&text) {
        return Err(repeating());
    }
    Ok(text)
}

async fn put(
    http: &reqwest::Client,
    login: &Login,
    href: &str,
    body: String,
    new: bool,
) -> Result<()> {
    let mut request = http
        .put(href)
        .basic_auth(&login.user, Some(&login.password))
        .header("Content-Type", "text/calendar; charset=utf-8")
        .body(body);
    if new {
        request = request.header("If-None-Match", "*");
    }
    let response = request.send().await.map_err(http_error)?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let text = response.text().await.unwrap_or_default();
        Err(refused(status.as_u16(), &text))
    }
}

async fn propfind(
    http: &reqwest::Client,
    login: &Login,
    url: &str,
    depth: &str,
    body: &str,
) -> Result<String> {
    dav(http, login, "PROPFIND", url, depth, Some(body.to_string())).await
}

async fn dav(
    http: &reqwest::Client,
    login: &Login,
    method: &str,
    url: &str,
    depth: &str,
    body: Option<String>,
) -> Result<String> {
    let method =
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| refused(0, &e.to_string()))?;
    let mut request = http
        .request(method, url)
        .basic_auth(&login.user, Some(&login.password))
        .header("Depth", depth);
    if let Some(body) = body {
        request = request
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body);
    }
    let response = request.send().await.map_err(http_error)?;
    let status = response.status();
    let text = response.text().await.map_err(http_error)?;
    // Apple answers an Apple ID it knows with a wrong password 401, and
    // one it does not know at all 403: worth telling apart, since the
    // fixes differ.
    if status.as_u16() == 401 {
        return Err(refused(
            401,
            "Apple knows that Apple ID but not that password. It has to be an app-specific \
             password, made at appleid.apple.com, not the Apple ID's own",
        ));
    }
    if status.as_u16() == 403 && url == ICLOUD {
        return Err(refused(
            403,
            "Apple does not know that Apple ID. Use the address you sign in to Apple with",
        ));
    }
    if !status.is_success() {
        return Err(refused(status.as_u16(), &text));
    }
    Ok(text)
}

/// An app-specific password as Apple writes it, `abcd-efgh-ijkl-mnop`,
/// however it was typed: with spaces where a phone's autofill put them,
/// without dashes, in capitals. Anything else is left as it came.
pub fn app_password(typed: &str) -> String {
    let letters: String = typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if letters.len() == 16 && letters.chars().all(|c| c.is_ascii_lowercase()) {
        letters
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("-")
    } else {
        typed.trim().to_string()
    }
}

/// The text of the `href` inside the first `<ns:local>`.
fn href_in(body: &str, ns: &str, local: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(body).ok()?;
    doc.descendants()
        .find(|n| n.has_tag_name((ns, local)))?
        .descendants()
        .find(|n| n.has_tag_name((DAV, "href")))?
        .text()
        .map(|t| t.trim().to_string())
}

/// An `href` as a whole address: servers answer with paths, and iCloud
/// with whole addresses on another host.
pub fn absolute(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let origin_end = base
        .find("://")
        .and_then(|i| base[i + 3..].find('/').map(|j| i + 3 + j))
        .unwrap_or(base.len());
    format!("{}{}", &base[..origin_end], href)
}

fn refused(status: u16, reason: &str) -> Error {
    Error::Provider {
        provider: "iCloud",
        status,
        reason: first_line(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_an_app_password_as_apple_does() {
        for typed in [
            "abcd-efgh-ijkl-mnop",
            "abcd efgh ijkl mnop",
            "abcdefghijklmnop",
            " ABCD-EFGH-IJKL-MNOP ",
        ] {
            assert_eq!(app_password(typed), "abcd-efgh-ijkl-mnop", "{typed:?}");
        }
        assert_eq!(app_password(" hunter2 "), "hunter2");
    }

    #[test]
    fn makes_an_href_whole() {
        assert_eq!(
            absolute("https://caldav.icloud.com/", "/1234/principal/"),
            "https://caldav.icloud.com/1234/principal/"
        );
        assert_eq!(
            absolute(
                "https://caldav.icloud.com/1234/principal/",
                "https://p52-caldav.icloud.com:443/1234/calendars/"
            ),
            "https://p52-caldav.icloud.com:443/1234/calendars/"
        );
        assert_eq!(
            absolute(
                "https://p52-caldav.icloud.com:443/1234/calendars/",
                "/1234/calendars/home/"
            ),
            "https://p52-caldav.icloud.com:443/1234/calendars/home/"
        );
    }

    #[test]
    fn finds_the_home_in_a_multistatus() {
        let body = r#"<?xml version="1.0"?><multistatus xmlns="DAV:"><response><href>/1234/principal/</href><propstat><prop><calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:">https://p52-caldav.icloud.com:443/1234/calendars/</href></calendar-home-set></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;
        assert_eq!(
            href_in(body, CALDAV, "calendar-home-set").as_deref(),
            Some("https://p52-caldav.icloud.com:443/1234/calendars/")
        );
    }
}
