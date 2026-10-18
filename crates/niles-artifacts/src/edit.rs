//! Changing part of what an artifact keeps without writing the rest out
//! again: exact replacements in a text (a document's markdown, a page's
//! HTML), and set, insert and remove at a JSON pointer in a kind's JSON.
//! A model writes slowly and reads fast. Every edit is made in order,
//! each on what the ones before it left, and all of them or none. What
//! results is checked by whoever keeps it, as a whole new version would
//! be.

use serde_json::{Value, json};

use crate::error::{Error, Result};

fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid {
        kind: "edit",
        reason: reason.into(),
    }
}

/// `text` with each of `edits`, a list of {old, new}, made in turn: each
/// old found exactly once in the text as the edits before it left it.
pub fn text(text: &str, edits: &Value) -> Result<String> {
    let parsed = entries(edits)?
        .iter()
        .enumerate()
        .map(|(i, e)| replacement(i + 1, e))
        .collect::<Result<Vec<_>>>()?;
    replace(text, &parsed)
}

/// `content`, of kind `kind`, with each of `edits` made in turn: a
/// document's as text on its markdown, any other kind's by
/// {op, path, value} at a JSON pointer.
pub fn content(kind: &str, content: &Value, edits: &Value) -> Result<Value> {
    let label = crate::kinds::kind(kind)
        .map_or("artifact", |k| k.label)
        .to_lowercase();
    let list = entries(edits)?;
    if kind == crate::document::KIND.id {
        for e in &list {
            if e.get("old").is_none() {
                return Err(invalid(
                    "a document is changed by its text: each edit is {old, new}",
                ));
            }
        }
        let Some(markdown) = content["markdown"].as_str() else {
            return Err(invalid("the document has no markdown to change"));
        };
        let parsed = list
            .iter()
            .enumerate()
            .map(|(i, e)| replacement(i + 1, e))
            .collect::<Result<Vec<_>>>()?;
        return Ok(json!({ "markdown": replace(markdown, &parsed)? }));
    }
    for e in &list {
        if e.get("op").is_none() {
            return Err(invalid(format!(
                "a {label} is changed by place: each edit is {{op, path, value}}, path a JSON \
                 pointer such as /days/2/title"
            )));
        }
    }
    let mut edited = content.clone();
    for (i, e) in list.iter().enumerate() {
        apply(&mut edited, i + 1, &operation(i + 1, e)?)?;
    }
    Ok(edited)
}

fn entries(edits: &Value) -> Result<Vec<Value>> {
    let list = match edits {
        Value::Array(list) => list.clone(),
        // A model that writes the list as text has still written it.
        Value::String(s) => {
            serde_json::from_str(s).map_err(|_| invalid("edits is a list of edits"))?
        }
        _ => return Err(invalid("edits is a list of edits")),
    };
    if list.is_empty() {
        return Err(invalid("edits is empty: give at least one"));
    }
    Ok(list)
}

struct Replace {
    old: String,
    new: String,
}

fn replacement(n: usize, entry: &Value) -> Result<Replace> {
    let bad = || invalid(format!("edit {n} is {{old, new}}, both text"));
    let (Some(old), Some(new)) = (entry.get("old"), entry.get("new")) else {
        return Err(bad());
    };
    let (Some(old), Some(new)) = (old.as_str(), new.as_str()) else {
        return Err(bad());
    };
    if old.is_empty() {
        return Err(invalid(format!(
            "edit {n}: old is empty; it is the exact text to replace"
        )));
    }
    // Kept text is \n-only (page::page and the document validator
    // normalise), so an edit written with \r\n must still match it.
    Ok(Replace {
        old: old.replace("\r\n", "\n"),
        new: new.replace("\r\n", "\n"),
    })
}

fn replace(text: &str, edits: &[Replace]) -> Result<String> {
    let mut text = text.to_string();
    for (n, e) in edits.iter().enumerate() {
        let n = n + 1;
        match occurrences(&text, &e.old) {
            0 => {
                return Err(invalid(format!(
                    "edit {n}: its old text is not found; copy it exactly as it is now, from \
                     artifact_get"
                )));
            }
            1 => text = text.replacen(&e.old, &e.new, 1),
            k => {
                return Err(invalid(format!(
                    "edit {n}: its old text is found {k} times; include more of the text around \
                     it, so it is found once"
                )));
            }
        }
    }
    Ok(text)
}

fn occurrences(text: &str, old: &str) -> usize {
    let first = old.chars().next().map_or(1, char::len_utf8);
    let (mut count, mut at) = (0, 0);
    while let Some(found) = text[at..].find(old) {
        count += 1;
        // One past the match's first char, not one past its end, so
        // overlapping matches each count: "aa" in "aaa" is 2.
        at += found + first;
    }
    count
}

enum Op {
    Set { path: String, value: Value },
    Insert { path: String, value: Value },
    Remove { path: String },
}

fn operation(n: usize, entry: &Value) -> Result<Op> {
    let Some(op) = entry.get("op").and_then(Value::as_str) else {
        return Err(invalid(format!("edit {n}: op is set, insert or remove")));
    };
    let Some(path) = entry.get("path").and_then(Value::as_str) else {
        return Err(invalid(format!(
            "edit {n}: path is a JSON pointer such as /days/2/title"
        )));
    };
    match op {
        "set" | "insert" => {
            // Present-and-null is a value, like serde's null.
            let Some(value) = entry.get("value") else {
                return Err(invalid(format!("edit {n}: {op} needs value")));
            };
            let path = path.to_string();
            let value = value.clone();
            if op == "set" {
                Ok(Op::Set { path, value })
            } else {
                Ok(Op::Insert { path, value })
            }
        }
        "remove" => Ok(Op::Remove {
            path: path.to_string(),
        }),
        _ => Err(invalid(format!("edit {n}: op is set, insert or remove"))),
    }
}

fn apply(content: &mut Value, n: usize, op: &Op) -> Result<()> {
    let bad = |reason: String| invalid(format!("edit {n}: {reason}"));
    let path = match op {
        Op::Set { path, .. } | Op::Insert { path, .. } | Op::Remove { path } => path,
    };
    if path.is_empty() {
        return Err(bad(
            "path is empty, which is the whole content; save all of it with artifact_update"
                .to_string(),
        ));
    }
    if !path.starts_with('/') {
        return Err(bad(
            "path is a JSON pointer starting with /, such as /days/2/items/0/time".to_string(),
        ));
    }
    let (parent, last) = path.rsplit_once('/').expect("a leading / leaves a split");
    let last = last.replace("~1", "/").replace("~0", "~");
    let shown = if parent.is_empty() {
        "the content".to_string()
    } else {
        parent.to_string()
    };
    let Some(target) = content.pointer_mut(parent) else {
        return Err(bad(format!("{shown} is not in it")));
    };
    match target {
        Value::Object(map) => match op {
            Op::Set { value, .. } => {
                map.insert(last, value.clone());
                Ok(())
            }
            Op::Insert { .. } => Err(bad(format!("{shown} is not a list; set adds a field"))),
            Op::Remove { path } => {
                if map.remove(&last).is_some() {
                    Ok(())
                } else {
                    Err(bad(format!("{path} is not there to remove")))
                }
            }
        },
        Value::Array(list) => match op {
            Op::Set { path, value } => {
                let i = element(n, path, &last, &shown, list.len())?;
                list[i] = value.clone();
                Ok(())
            }
            Op::Insert { path, value } => {
                let i = match last.as_str() {
                    "-" => list.len(),
                    _ => element(n, path, &last, &shown, list.len())?,
                };
                if i > list.len() {
                    return Err(bad(format!(
                        "{path} is past the end of a list of {}; insert adds an entry",
                        list.len()
                    )));
                }
                list.insert(i, value.clone());
                Ok(())
            }
            Op::Remove { path } => {
                let i = element(n, path, &last, &shown, list.len())?;
                list.remove(i);
                Ok(())
            }
        },
        _ => Err(bad(format!("{shown} is a value, with nothing inside it"))),
    }
}

/// The index `last` names in a list of `len`, or why it does not.
fn element(n: usize, path: &str, last: &str, shown: &str, len: usize) -> Result<usize> {
    let bad = |reason: String| invalid(format!("edit {n}: {reason}"));
    if last == "-" {
        return Err(bad("- is the end of a list, for insert".to_string()));
    }
    let Some(i) = index(last) else {
        return Err(bad(format!(
            "{last} is not an index into the list at {shown}"
        )));
    };
    if i >= len {
        return Err(bad(format!(
            "{path} is past the end of a list of {len}; insert adds an entry"
        )));
    }
    Ok(i)
}

/// serde_json's rule for a list index: no leading `+`, and no leading
/// zero unless the string is exactly "0".
fn index(s: &str) -> Option<usize> {
    if s.starts_with('+') || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacements_are_made_in_order_each_on_what_the_last_left() {
        let edited = text(
            "a b c",
            &json!([{ "old": "b", "new": "B" }, { "old": "a B", "new": "A B" }]),
        )
        .unwrap();
        assert_eq!(edited, "A B c");
    }

    #[test]
    fn an_old_text_not_there_refuses_the_whole_call() {
        let e = text(
            "a b c",
            &json!([{ "old": "b", "new": "B" }, { "old": "zz", "new": "Z" }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("edit 2"), "{e}");
        assert!(e.to_string().contains("not found"), "{e}");
    }

    #[test]
    fn an_old_text_found_more_than_once_asks_for_more_of_it() {
        let e = text("x x x", &json!([{ "old": "x", "new": "y" }])).unwrap_err();
        assert!(e.to_string().contains("found 3 times"), "{e}");
        assert!(e.to_string().contains("include more"), "{e}");
    }

    #[test]
    fn an_old_text_that_overlaps_itself_counts_each_place() {
        let e = text("aaa", &json!([{ "old": "aa", "new": "b" }])).unwrap_err();
        assert!(e.to_string().contains("found 2 times"), "{e}");
    }

    #[test]
    fn no_edits_an_empty_old_or_a_half_edit_is_refused() {
        assert!(
            text("a", &json!([]))
                .unwrap_err()
                .to_string()
                .contains("at least one")
        );
        assert!(
            text("a", &json!([{ "old": "", "new": "b" }]))
                .unwrap_err()
                .to_string()
                .contains("old is empty")
        );
        assert!(
            text("a", &json!([{ "old": "a" }]))
                .unwrap_err()
                .to_string()
                .contains("both text")
        );
        assert!(
            text("a", &json!({ "old": "a" }))
                .unwrap_err()
                .to_string()
                .contains("list of edits")
        );
        assert!(
            text("a", &json!("not a list"))
                .unwrap_err()
                .to_string()
                .contains("list of edits")
        );
    }

    #[test]
    fn windows_line_endings_in_an_edit_match_the_kept_text() {
        let edited = text("one\ntwo", &json!([{ "old": "one\r\ntwo", "new": "1\n2" }])).unwrap();
        assert_eq!(edited, "1\n2");
    }

    #[test]
    fn edits_written_as_text_holding_a_list_are_read() {
        let edited = text("a b", &json!("[{\"old\": \"b\", \"new\": \"B\"}]")).unwrap();
        assert_eq!(edited, "a B");
    }

    #[test]
    fn set_insert_and_remove_by_json_pointer() {
        let trip = json!({
            "a": [{ "t": "x" }, { "t": "y" }],
            "list": ["b", "c"],
            "obj": { "k": 1 }
        });
        let edited = content(
            "trip",
            &trip,
            &json!([
                { "op": "set", "path": "/a/0/t", "value": "set" },
                { "op": "set", "path": "/obj/new", "value": 2 },
                { "op": "insert", "path": "/list/0", "value": "first" },
                { "op": "insert", "path": "/list/-", "value": "last" },
                { "op": "remove", "path": "/a/1" }
            ]),
        )
        .unwrap();
        assert_eq!(edited["a"], json!([{ "t": "set" }]));
        assert_eq!(edited["list"], json!(["first", "b", "c", "last"]));
        assert_eq!(edited["obj"], json!({ "k": 1, "new": 2 }));
        assert_eq!(trip["a"][1]["t"], "y", "the original is untouched");
    }

    #[test]
    fn a_place_that_is_not_there_is_refused_by_where() {
        let days = || json!({ "days": [{ "title": "a" }, { "title": "b" }] });
        let e = content(
            "trip",
            &days(),
            &json!([{ "op": "set", "path": "/days/9/title", "value": "x" }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("/days/9"), "{e}");

        let e = content(
            "trip",
            &json!({ "a": 1 }),
            &json!([{ "op": "remove", "path": "/b" }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("not there to remove"), "{e}");

        let e = content(
            "trip",
            &json!({ "obj": { "k": 1 } }),
            &json!([{ "op": "insert", "path": "/obj/0", "value": 1 }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("set adds a field"), "{e}");

        let e = content(
            "trip",
            &json!({ "list": ["a"] }),
            &json!([{ "op": "set", "path": "/list/-", "value": "b" }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("for insert"), "{e}");

        let e = content(
            "trip",
            &json!({ "list": ["a"] }),
            &json!([{ "op": "set", "path": "list/0", "value": "b" }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("starting with /"), "{e}");

        let e = content(
            "trip",
            &json!({ "list": ["a"] }),
            &json!([{ "op": "set", "path": "", "value": "b" }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("artifact_update"), "{e}");
    }

    #[test]
    fn a_document_is_changed_by_its_text_and_a_trip_by_place() {
        let doc = json!({ "markdown": "pack:\n- [ ] passport" });
        let edited = content(
            "document",
            &doc,
            &json!([{ "old": "passport", "new": "Passport" }]),
        )
        .unwrap();
        assert_eq!(edited, json!({ "markdown": "pack:\n- [ ] Passport" }));

        let e = content(
            "document",
            &doc,
            &json!([{ "op": "set", "path": "/markdown", "value": "x" }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("by its text"), "{e}");

        let e = content(
            "trip",
            &json!({ "a": 1 }),
            &json!([{ "old": "a", "new": "b" }]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("by place"), "{e}");
    }
}
