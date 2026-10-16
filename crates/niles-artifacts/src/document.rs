//! A document: markdown, `{"markdown": "…"}`.
//!
//! Markdown rather than a tree of blocks because it is what Claude and
//! ChatGPT write best, in one go, and a change is a whole new text the
//! model cannot get structurally wrong. A checklist item is `- [ ]` and
//! a tick is that one character. The app renders it itself, element by
//! element, so nothing in it is ever HTML.

use crate::error::{Error, Result};
use crate::kinds::{Kind, Preview, PreviewLine, Style};
use serde_json::{Value, json};

pub(crate) const KIND: Kind = Kind::new(
    "document",
    "Document",
    "{\"markdown\": \"…\"}: the whole document as markdown. Headings, paragraphs, bulleted \
     and numbered lists, checklists (- [ ] and - [x], which the person can tick), simple \
     tables, bold, italic and links, and pictures the document keeps, each on a line of \
     its own: ![what it shows](niles-image:12), by the name artifact_add_image answers. No \
     HTML: it is shown as text. A picture from a web address is not shown.",
    "Writing",
    validate,
    preview,
);

/// Long enough for a two-week travel plan with every day written out;
/// short enough that a runaway model is stopped.
pub const MAX_CHARS: usize = 100_000;

/// How many lines a card shows, and how long each may be.
const PREVIEW_LINES: usize = 4;
const PREVIEW_CHARS: usize = 120;

fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid {
        kind: "document",
        reason: reason.into(),
    }
}

/// A bare string is taken as the markdown: a model that leaves out the
/// wrapper has still written the document.
fn validate(content: &Value) -> Result<Value> {
    let text = match content {
        Value::String(s) => s.as_str(),
        Value::Object(o) => o
            .get("markdown")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("content needs \"markdown\", a string"))?,
        _ => return Err(invalid("content is {\"markdown\": \"…\"}")),
    };
    let text: String = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let text = text.trim_end();
    if text.trim().is_empty() {
        return Err(invalid("the document is empty"));
    }
    let n = text.chars().count();
    if n > MAX_CHARS {
        return Err(invalid(format!(
            "the document is {n} characters, and at most {MAX_CHARS} are kept"
        )));
    }
    Ok(json!({ "markdown": text }))
}

fn preview(content: &Value) -> Preview {
    let text = content
        .get("markdown")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut lines = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.is_empty() || is_rule(line) || is_table_divider(line) || is_picture(line)
        {
            continue;
        }
        lines.push(preview_line(line));
        if lines.len() == PREVIEW_LINES {
            break;
        }
    }
    Preview { lines }
}

fn preview_line(line: &str) -> PreviewLine {
    let (style, checked, rest) = if let Some(rest) = heading(line) {
        (Style::Heading, None, rest)
    } else if let Some((checked, rest)) = checklist(line) {
        (Style::Check, Some(checked), rest)
    } else if let Some(rest) = bullet(line) {
        (Style::Bullet, None, rest)
    } else if numbered(line).is_some() {
        (Style::Number, None, line)
    } else if line.starts_with('|') {
        let cells: Vec<&str> = line
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect();
        return PreviewLine {
            text: cut(&plain(&cells.join(" · "))),
            style: Style::Text,
            checked: None,
        };
    } else {
        (Style::Text, None, line.trim_start_matches('>').trim_start())
    };
    PreviewLine {
        text: cut(&plain(rest)),
        style,
        checked,
    }
}

fn heading(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches('#');
    let level = line.len() - rest.len();
    ((1..=6).contains(&level) && rest.starts_with(' ')).then(|| rest.trim())
}

/// `- [ ] x`, `- [x] x`, with `*` or `+` as the bullet too.
pub(crate) fn checklist(line: &str) -> Option<(bool, &str)> {
    let rest = bullet(line)?;
    let checked = match rest.get(..3)? {
        "[ ]" => false,
        "[x]" | "[X]" => true,
        _ => return None,
    };
    Some((checked, rest[3..].trim_start()))
}

fn bullet(line: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|b| line.strip_prefix(b))
        .map(str::trim_start)
}

/// `1. x` or `1) x`.
fn numbered(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    rest.strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))
        .map(str::trim_start)
}

fn is_rule(line: &str) -> bool {
    line.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|c| line.chars().all(|x| x == *c || x == ' '))
}

/// A line that is only a picture, `![alt](niles-image:12)`: a card
/// shows words.
fn is_picture(line: &str) -> bool {
    line.starts_with("![") && line.ends_with(')') && line.contains("](")
}

fn is_table_divider(line: &str) -> bool {
    line.starts_with('|') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// The words without their markup: `**bold**` is bold, `[a](b)` is a.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find("](").and_then(|close| {
            let url = &after[close + 2..];
            url.find(')').map(|end| (close, close + 2 + end + 1))
        }) {
            Some((close, end)) => {
                if out.ends_with('!') {
                    out.pop();
                }
                out.push_str(&after[..close]);
                rest = &after[end..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "")
        .replace("__", "")
        .replace(['`', '*'], "")
        .trim()
        .to_string()
}

fn cut(text: &str) -> String {
    if text.chars().count() <= PREVIEW_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(PREVIEW_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(markdown: &str) -> Vec<(Style, String, Option<bool>)> {
        preview(&json!({ "markdown": markdown }))
            .lines
            .into_iter()
            .map(|l| (l.style, l.text, l.checked))
            .collect()
    }

    #[test]
    fn markdown_is_kept_as_written() {
        let v = validate(&json!({ "markdown": "# Thailand\n\n- Bangkok\n" })).unwrap();
        assert_eq!(v, json!({ "markdown": "# Thailand\n\n- Bangkok" }));
    }

    #[test]
    fn a_bare_string_is_the_markdown() {
        let v = validate(&json!("- passport")).unwrap();
        assert_eq!(v["markdown"], "- passport");
    }

    #[test]
    fn line_endings_and_control_characters_are_tidied() {
        let v = validate(&json!({ "markdown": "a\r\nb\u{0}\tc\rd" })).unwrap();
        assert_eq!(v["markdown"], "a\nb\tc\nd");
    }

    #[test]
    fn an_empty_document_is_refused() {
        assert!(validate(&json!({ "markdown": "  \n " })).is_err());
        assert!(validate(&json!({ "text": "hello" })).is_err());
        assert!(validate(&json!(42)).is_err());
    }

    #[test]
    fn a_runaway_document_is_refused() {
        let long = "a".repeat(MAX_CHARS + 1);
        let e = validate(&json!({ "markdown": long })).unwrap_err();
        assert!(e.to_string().contains("at most"), "{e}");
    }

    #[test]
    fn the_preview_is_the_first_lines_without_their_markup() {
        let got = lines(
            "# Thailand, two weeks\n\nTen days in the **south**, see [the map](https://x.example).\n\n- Bangkok\n- [x] Book flights\n- [ ] Ferry to Koh Tao\n",
        );
        assert_eq!(
            got,
            vec![
                (Style::Heading, "Thailand, two weeks".into(), None),
                (
                    Style::Text,
                    "Ten days in the south, see the map.".into(),
                    None
                ),
                (Style::Bullet, "Bangkok".into(), None),
                (Style::Check, "Book flights".into(), Some(true)),
            ]
        );
    }

    #[test]
    fn rules_fences_and_table_dividers_are_not_lines() {
        let got =
            lines("---\n```\ncode\n```\n| Day | Where |\n|---|---|\n| 1 | Bangkok |\n1. First");
        assert_eq!(
            got,
            vec![
                (Style::Text, "Day · Where".into(), None),
                (Style::Text, "1 · Bangkok".into(), None),
                (Style::Number, "1. First".into(), None),
            ]
        );
    }

    #[test]
    fn a_picture_is_not_a_line_of_the_preview_and_inside_one_is_its_words() {
        let got = lines(
            "![The palace](niles-image:3)

See ![the gate](niles-image:4) first.",
        );
        assert_eq!(got, vec![(Style::Text, "See the gate first.".into(), None)]);
    }

    #[test]
    fn a_long_line_is_cut() {
        let got = lines(&"word ".repeat(60));
        assert!(got[0].1.ends_with('…'));
        assert!(got[0].1.chars().count() <= PREVIEW_CHARS);
    }

    #[test]
    fn a_checklist_item_is_read_with_any_bullet_and_either_x() {
        assert_eq!(checklist("* [X] done"), Some((true, "done")));
        assert_eq!(checklist("+ [ ] todo"), Some((false, "todo")));
        assert_eq!(checklist("- [?] maybe"), None);
        assert_eq!(checklist("[ ] no bullet"), None);
    }
}
