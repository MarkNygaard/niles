//! A podcast: a researched topic as a two-host episode's script, as JSON.
//!
//! Strict, as a trip is, because the model writes it and reads what is
//! wrong — and because Niles records the script line by line, so a line
//! that is not one host's turn, or a script over its word budget, fails
//! after the research rather than before it. The word budget is the one
//! hard promise made to the person: 150 words a minute, at most 8
//! minutes, so an episode never runs long on the speaker it plays on.

use crate::error::{Error, Result};
use crate::kinds::{Kind, Preview, PreviewLine, Style};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

pub const ID: &str = "podcast";
/// What two hosts say comfortably in a minute of MP3.
pub const WORDS_A_MINUTE: usize = 150;
/// The longest episode made: "never more than 8".
pub const MAX_MINUTES: u8 = 8;
/// What "a quick one" asks for.
pub const QUICK_MINUTES: u8 = 3;
/// What an unqualified ask gets.
pub const USUAL_MINUTES: u8 = 5;

/// The most words a script of `minutes` minutes may carry.
pub fn budget(minutes: u8) -> usize {
    minutes as usize * WORDS_A_MINUTE
}

/// A topic stated in one line.
const MAX_TOPIC: usize = 200;
/// A host's first name.
const MAX_HOST_NAME: usize = 40;
/// Eight minutes at four words a second, with room for breath.
const MAX_LINES: usize = 400;
const MIN_LINES: usize = 2;
/// One turn of speech: two breaths, said slowly.
const MAX_LINE: usize = 2_000;
const MAX_SOURCES: usize = 30;
const MAX_TITLE: usize = 200;
const MAX_URL: usize = 2_000;
/// The whole episode as JSON: the script of an 8-minute episode is
/// well under it.
const MAX_CHARS: usize = 100_000;

const PODCAST: &[&str] = &["topic", "minutes", "hosts", "lines", "sources"];
const LINE: &[&str] = &["host", "text"];
const SOURCE: &[&str] = &["title", "url"];

/// The first line of a card, and how much room it leaves.
const PREVIEW_CHARS: usize = 120;

pub(crate) const KIND: Kind = Kind::new(
    ID,
    "Podcast",
    "{\"topic\", \"minutes\", \"hosts\": [two names], \"lines\": [{\"host\", \"text\"}], \
     \"sources\": [{\"title\", \"url\"}]}: a podcast episode, two hosts talking a topic \
     through. topic is one line. minutes is the length asked for, a whole number from 1 to \
     8: 3 for a quick one, 5 when they did not say. hosts are the two hosts' first names, \
     the first host first. lines is the script in order, each line one host's turn: host is \
     one of the two names, text what they say, in plain spoken English (no markdown, \
     lists, stage directions or emoji). Both hosts speak. The script is at most 150 words a \
     minute, every line's text counted: 450 for 3 minutes, 750 for 5, 1,200 for 8. \
     sources are the pages it draws on, each with its title and its http or https address: \
     at least one, at most 30.",
    "Researching",
    validate,
    preview,
);

fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid {
        kind: ID,
        reason: reason.into(),
    }
}

fn validate(content: &Value) -> Result<Value> {
    match content {
        Value::String(text) => podcast(&from_text(text)?),
        other => podcast(other),
    }
}

/// A run that answered with the episode instead of saving it may have
/// put it in a code fence, or a sentence before it.
fn from_text(text: &str) -> Result<Value> {
    let text = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str(text) {
        return Ok(v);
    }
    match (text.find('{'), text.rfind('}')) {
        (Some(open), Some(close)) if open < close => serde_json::from_str(&text[open..=close])
            .map_err(|e| {
                invalid(format!(
                    "content is a JSON object, and this is not one: {e}"
                ))
            }),
        _ => Err(invalid(
            "content is a JSON object, {\"topic\": …, \"lines\": […]}, not text",
        )),
    }
}

fn podcast(content: &Value) -> Result<Value> {
    let o = fields(content, "the episode", PODCAST)?;
    let topic = one_line(o, "topic", "the episode", MAX_TOPIC)?
        .ok_or_else(|| invalid("the episode needs \"topic\": \"the Øresund bridge\""))?;
    let minutes = minutes(o)?;
    let hosts = hosts(o)?;
    let lines = lines(o, &hosts)?;
    let words = lines
        .iter()
        .map(|l| {
            l["text"]
                .as_str()
                .expect("validated")
                .split_whitespace()
                .count()
        })
        .sum::<usize>();
    let allowed = budget(minutes);
    if words > allowed {
        return Err(invalid(format!(
            "the script is {words} words, and a {minutes}-minute episode is at most \
             {allowed} (150 a minute): cut about {} words and save it again",
            words - allowed
        )));
    }
    let sources = sources(o)?;
    let kept = json!({
        "topic": topic,
        "minutes": minutes,
        "hosts": hosts,
        "lines": lines,
        "sources": sources,
    });
    let size = kept.to_string().len();
    if size > MAX_CHARS {
        return Err(invalid("the podcast is too long to keep"));
    }
    Ok(kept)
}

/// An object's fields, refusing any it does not have.
fn fields<'a>(value: &'a Value, at: &str, known: &[&str]) -> Result<&'a Map<String, Value>> {
    let o = value
        .as_object()
        .ok_or_else(|| invalid(format!("{at} is an object, {{\"{}\": …}}", known[0])))?;
    if let Some(extra) = o.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(invalid(format!(
            "{at} has {extra:?}, which is not part of a podcast; its fields are {}",
            known.join(", ")
        )));
    }
    Ok(o)
}

/// A field that is there: `null` is as good as left out.
fn present<'a>(o: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    o.get(key).filter(|v| !v.is_null())
}

fn string(o: &Map<String, Value>, key: &str, at: &str) -> Result<Option<String>> {
    match present(o, key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(v) => Err(invalid(format!("{at}: {key} is text, not {v}"))),
    }
}

/// `raw` on one line, or nothing when it is empty.
fn one_line(o: &Map<String, Value>, key: &str, at: &str, max: usize) -> Result<Option<String>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return Ok(None);
    }
    if line.chars().count() > max {
        return Err(invalid(format!(
            "{at}: {key} is at most {max} characters, and this is {}",
            line.chars().count()
        )));
    }
    Ok(Some(line))
}

fn minutes(o: &Map<String, Value>) -> Result<u8> {
    let at = "the episode";
    let Some(v) = present(o, "minutes") else {
        return Err(invalid(format!("{at} needs \"minutes\", its length")));
    };
    let whole = match v {
        Value::Number(n) => n.as_u64().or_else(|| {
            n.as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                .map(|f| f as u64)
        }),
        _ => None,
    };
    let Some(minutes) = whole else {
        return Err(invalid(format!(
            "{at}: minutes is the episode's length, a whole number of minutes from 1 to \
             {MAX_MINUTES}; an episode is at most {MAX_MINUTES} minutes"
        )));
    };
    if minutes < 1 || minutes > MAX_MINUTES as u64 {
        return Err(invalid(format!(
            "minutes is the episode's length, a whole number of minutes from 1 to \
             {MAX_MINUTES}; an episode is at most {MAX_MINUTES} minutes"
        )));
    }
    Ok(minutes as u8)
}

/// The two hosts' first names, the first host first, kept with the
/// first spelling that named them.
fn hosts(o: &Map<String, Value>) -> Result<[String; 2]> {
    let at = "the episode";
    let Some(Value::Array(hosts)) = present(o, "hosts") else {
        return Err(invalid(format!(
            "{at} needs \"hosts\": [\"Maya\", \"Leo\"], the two hosts' first names"
        )));
    };
    if hosts.len() != 2 {
        return Err(invalid(
            "hosts are the two hosts' first names, [\"Maya\", \"Leo\"]",
        ));
    }
    let mut names = Vec::new();
    for (i, h) in hosts.iter().enumerate() {
        let at = format!("host {}", i + 1);
        let Value::String(raw) = h else {
            return Err(invalid(format!("{at} is a name, not {h}")));
        };
        let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || name.chars().count() > MAX_HOST_NAME {
            return Err(invalid(
                "hosts are the two hosts' first names, [\"Maya\", \"Leo\"]",
            ));
        }
        names.push(name);
    }
    if names[0].eq_ignore_ascii_case(&names[1]) {
        return Err(invalid(
            "hosts are the two hosts' first names, [\"Maya\", \"Leo\"]",
        ));
    }
    Ok([names[0].clone(), names[1].clone()])
}

fn lines(o: &Map<String, Value>, hosts: &[String; 2]) -> Result<Vec<Value>> {
    let at = "the episode";
    let Some(Value::Array(lines)) = present(o, "lines") else {
        return Err(invalid(format!(
            "{at} needs \"lines\": [{{\"host\": \"{}\", \"text\": …}}, …], the script in \
             order",
            hosts[0]
        )));
    };
    if lines.len() < MIN_LINES || lines.len() > MAX_LINES {
        return Err(invalid(format!(
            "lines is the script, {MIN_LINES} to {MAX_LINES} of them, each one host's turn"
        )));
    }
    let mut spoken = [false, false];
    let mut kept = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let n = i + 1;
        let o = fields(line, &format!("line {n}"), LINE)?;
        let at = format!("line {n}");
        let Some(host) = string(o, "host", &at)? else {
            return Err(invalid(format!(
                "line {n} needs \"host\": \"{}\"",
                hosts[0]
            )));
        };
        let host = hosts
            .iter()
            .position(|h| h.eq_ignore_ascii_case(&host))
            .ok_or_else(|| {
                invalid(format!(
                    "line {n}'s host is {host:?}, and the hosts are {} and {}",
                    hosts[0], hosts[1]
                ))
            })?;
        spoken[host] = true;
        let Some(text) = string(o, "text", &at)? else {
            return Err(invalid(format!(
                "line {n} needs \"text\", what {} says",
                hosts[host]
            )));
        };
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() || text.chars().count() > MAX_LINE {
            return Err(invalid(format!(
                "line {n}'s text is what host {} says, a line of at most {MAX_LINE} \
                 characters",
                hosts[host]
            )));
        }
        kept.push(json!({ "host": hosts[host], "text": text }));
    }
    for (host, did) in hosts.iter().zip(spoken) {
        if !did {
            return Err(invalid(format!("both hosts speak: {host} has no line")));
        }
    }
    Ok(kept)
}

fn sources(o: &Map<String, Value>) -> Result<Vec<Value>> {
    let Some(Value::Array(sources)) = present(o, "sources") else {
        return Err(invalid(
            "sources lists the pages the script draws on: at least one",
        ));
    };
    if sources.is_empty() || sources.len() > MAX_SOURCES {
        return Err(invalid(format!(
            "sources lists the pages the script draws on: 1 to {MAX_SOURCES} of them"
        )));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut kept = Vec::with_capacity(sources.len());
    for (i, source) in sources.iter().enumerate() {
        let n = i + 1;
        let o = fields(source, &format!("source {n}"), SOURCE)?;
        let at = format!("source {n}");
        let title = one_line(o, "title", &at, MAX_TITLE)?
            .ok_or_else(|| invalid(format!("{at} needs \"title\", the page's title")))?;
        let Some(url) = string(o, "url", &at)? else {
            return Err(invalid(format!(
                "{at} needs \"url\", its http or https address"
            )));
        };
        let url = url.trim().to_string();
        if url.len() > MAX_URL || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(invalid(format!(
                "{at}: url is an http or https address, such as \
                 \"https://www.oresundsbron.com/\", not {url:?}"
            )));
        }
        let parsed = url::Url::parse(&url).map_err(|_| {
            invalid(format!(
                "{at}: url is an http or https address, such as \
                 \"https://www.oresundsbron.com/\", not {url:?}"
            ))
        })?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
            return Err(invalid(format!(
                "{at}: url is an http or https address, such as \
                 \"https://www.oresundsbron.com/\", not {url:?}"
            )));
        }
        if seen.insert(url.clone()) {
            kept.push(json!({ "title": title, "url": url }));
        }
    }
    if kept.is_empty() {
        return Err(invalid(
            "sources lists the pages the script draws on: at least one",
        ));
    }
    Ok(kept)
}

/// The hosts, the length, the first line and how much it draws on: the
/// card's three lines. Lines with missing data are skipped.
fn preview(content: &Value) -> Preview {
    let mut lines = Vec::new();
    if let (Some(minutes), Some(first), Some(second)) = (
        content["minutes"].as_u64(),
        content["hosts"][0].as_str(),
        content["hosts"][1].as_str(),
    ) {
        lines.push(PreviewLine {
            text: format!("{minutes} min · {first} and {second}"),
            style: Style::Heading,
            checked: None,
        });
    }
    if let (Some(host), Some(text)) = (
        content["lines"][0]["host"].as_str(),
        content["lines"][0]["text"].as_str(),
    ) {
        let mut line = format!("{host}: {text}");
        if line.chars().count() > PREVIEW_CHARS {
            line = line.chars().take(PREVIEW_CHARS).collect::<String>() + "…";
        }
        lines.push(PreviewLine {
            text: line,
            style: Style::Text,
            checked: None,
        });
    }
    if let Some(sources) = content["sources"].as_array() {
        lines.push(PreviewLine {
            text: match sources.len() {
                1 => "1 source".to_string(),
                n => format!("{n} sources"),
            },
            style: Style::Text,
            checked: None,
        });
    }
    Preview { lines }
}

/// One line of the script: which host says it, and what they say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub host: usize,
    pub text: String,
}

/// The script of a kept podcast: the two hosts in order, and every
/// line as (host index, text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub hosts: [String; 2],
    pub lines: Vec<Line>,
}

/// The script out of already-validated content, for recording: each
/// line's host name read as 0 or 1, in the hosts' order.
pub fn script(content: &Value) -> Option<Script> {
    let hosts = [
        content["hosts"][0].as_str()?.to_string(),
        content["hosts"][1].as_str()?.to_string(),
    ];
    let array = content["lines"].as_array()?;
    let mut lines = Vec::with_capacity(array.len());
    for line in array {
        let name = line["host"].as_str()?;
        let host = hosts.iter().position(|h| h == name).filter(|h| *h <= 1)?;
        lines.push(Line {
            host,
            text: line["text"].as_str()?.to_string(),
        });
    }
    Some(Script { hosts, lines })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `words` words split over lines that alternate between the two
    /// hosts, with one source.
    fn podcast_json(minutes: u8, words: usize) -> Value {
        let words_per_line = 5;
        let lines: Vec<Value> = (0..words.div_ceil(words_per_line))
            .map(|i| {
                json!({
                    "host": if i % 2 == 0 { "Maya" } else { "Leo" },
                    "text": (0..words_per_line.min(words - i * words_per_line))
                        .map(|w| format!("w{}", i * words_per_line + w))
                        .collect::<Vec<_>>()
                        .join(" "),
                })
            })
            .collect();
        json!({
            "topic": "The Øresund bridge",
            "minutes": minutes,
            "hosts": ["Maya", "Leo"],
            "lines": lines,
            "sources": [{ "title": "Øresundsbron", "url": "https://www.oresundsbron.com/" }],
        })
    }

    fn keep(content: &Value) -> Result<Value> {
        validate(content)
    }

    #[test]
    fn the_budget_is_150_words_a_minute() {
        for (minutes, at_budget) in [(3, 450), (5, 750), (8, 1200)] {
            let fits = podcast_json(minutes, at_budget);
            assert!(
                keep(&fits).is_ok(),
                "{minutes} minutes should accept {at_budget} words"
            );
            let over = podcast_json(minutes, at_budget + 1);
            let error = keep(&over).expect_err("refused");
            let reason = error.to_string();
            assert!(
                reason.contains(&format!("{} words", at_budget + 1))
                    && reason.contains(&format!("at most {at_budget}")),
                "unexpected: {reason}"
            );
        }
    }

    #[test]
    fn minutes_outside_1_to_8_are_refused() {
        assert!(keep(&podcast_json(0, 10)).is_err());
        assert!(keep(&podcast_json(9, 10)).is_err());
        let mut half = podcast_json(3, 10);
        half["minutes"] = json!(2.5);
        assert!(keep(&half).is_err());
    }

    #[test]
    fn an_unknown_field_is_refused_by_name() {
        let mut content = podcast_json(3, 10);
        content["mood"] = json!("calm");
        let error = keep(&content).expect_err("refused");
        assert!(
            error.to_string().contains("\"mood\""),
            "unexpected: {error}"
        );
    }

    #[test]
    fn a_line_by_a_third_host_is_refused() {
        let mut content = podcast_json(3, 20);
        content["lines"][0]["host"] = json!("Ada");
        let error = keep(&content).expect_err("refused");
        assert!(
            error
                .to_string()
                .contains("line 1's host is \"Ada\", and the hosts are Maya and Leo"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn a_host_who_never_speaks_is_refused() {
        let mut content = podcast_json(3, 20);
        for i in 0..content["lines"].as_array().unwrap().len() {
            content["lines"][i]["host"] = json!("Maya");
        }
        let error = keep(&content).expect_err("refused");
        assert!(
            error
                .to_string()
                .contains("both hosts speak: Leo has no line"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn an_episode_draws_on_at_least_one_http_source() {
        let mut none = podcast_json(3, 10);
        none["sources"] = json!([]);
        assert!(keep(&none).is_err());
        let mut ftp = podcast_json(3, 10);
        ftp["sources"] = json!([{ "title": "A file", "url": "ftp://example.com/file" }]);
        let error = keep(&ftp).expect_err("refused");
        assert!(
            error.to_string().contains("http or https"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn a_source_given_twice_is_kept_once() {
        let mut content = podcast_json(3, 10);
        content["sources"] = json!([
            { "title": "One", "url": "https://example.com/a" },
            { "title": "One again", "url": "https://example.com/a" },
            { "title": "Two", "url": "https://example.com/b" },
        ]);
        let kept = keep(&content).expect("kept");
        assert_eq!(kept["sources"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn an_episode_answered_as_text_in_a_code_fence_is_read() {
        let content = podcast_json(5, 20);
        let fenced = format!("Here is the episode:\n```json\n{content}\n```");
        let kept = keep(&Value::String(fenced)).expect("kept");
        assert_eq!(kept["minutes"], json!(5));
    }

    #[test]
    fn a_kept_podcast_is_kept_again_unchanged() {
        let kept = keep(&podcast_json(5, 100)).expect("kept");
        assert_eq!(keep(&kept).expect("again"), kept);
    }

    #[test]
    fn the_preview_is_hosts_length_first_line_and_sources() {
        let mut content = podcast_json(5, 20);
        content["lines"][0]["text"] = json!("Welcome. ".repeat(30).trim());
        let preview = preview(&content);
        assert_eq!(preview.lines.len(), 3);
        assert_eq!(preview.lines[0].text, "5 min · Maya and Leo");
        assert!(
            preview.lines[1].text.starts_with("Maya: Welcome.")
                && preview.lines[1].text.ends_with('…'),
            "unexpected: {}",
            preview.lines[1].text
        );
        assert_eq!(preview.lines[1].text.chars().count(), PREVIEW_CHARS + 1);
        assert_eq!(preview.lines[2].text, "1 source");
    }

    #[test]
    fn script_maps_hosts_to_zero_and_one_in_order() {
        let kept = keep(&podcast_json(3, 30)).expect("kept");
        let script = script(&kept).expect("a script");
        assert_eq!(script.hosts, ["Maya", "Leo"]);
        assert_eq!(script.lines.len(), 6);
        assert_eq!(script.lines[0].host, 0);
        assert_eq!(script.lines[1].host, 1);
        assert_eq!(script.lines[2].host, 0);
        assert!(script.lines.iter().all(|l| !l.text.is_empty()));
    }
}
