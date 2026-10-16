//! The kinds of artifact, and what each one's content is.
//!
//! A kind is one entry in [`KINDS`], with its twin in the app's
//! `web/src/lib/artifacts.tsx`: a new kind is a new entry in each and a
//! view for it in the app. Everything else — storage, versions, jobs,
//! cards — is the same for every kind.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct Kind {
    pub id: &'static str,
    /// For people: "Document".
    pub label: &'static str,
    /// For the model: what `content` is, in a sentence or two.
    pub content_help: &'static str,
    /// What a job for this kind is doing, before the title: "Writing".
    pub doing: &'static str,
    validate: fn(&Value) -> Result<Value>,
    preview: fn(&Value) -> Preview,
}

impl Kind {
    pub(crate) const fn new(
        id: &'static str,
        label: &'static str,
        content_help: &'static str,
        doing: &'static str,
        validate: fn(&Value) -> Result<Value>,
        preview: fn(&Value) -> Preview,
    ) -> Self {
        Self {
            id,
            label,
            content_help,
            doing,
            validate,
            preview,
        }
    }

    /// The content as it is to be kept, or why it cannot be.
    pub fn validate(&self, content: &Value) -> Result<Value> {
        (self.validate)(content)
    }

    pub fn preview(&self, content: &Value) -> Preview {
        (self.preview)(content)
    }
}

pub const KINDS: &[Kind] = &[crate::document::KIND, crate::training_plan::KIND];

pub fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

/// The kind named `id`, or an error listing the ones there are.
pub fn require(id: &str) -> Result<&'static Kind> {
    kind(id).ok_or_else(|| Error::Invalid {
        kind: "kind",
        reason: format!(
            "there is no kind {id:?}; the kinds are {}",
            KINDS.iter().map(|k| k.id).collect::<Vec<_>>().join(", ")
        ),
    })
}

/// `content` checked for kind `id`.
pub fn validate(id: &str, content: &Value) -> Result<Value> {
    require(id)?.validate(content)
}

/// The preview of `content`, empty for a kind this Niles does not know.
pub fn preview(id: &str, content: &Value) -> Preview {
    kind(id).map(|k| k.preview(content)).unwrap_or_default()
}

/// The first few lines of an artifact, for a card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub lines: Vec<PreviewLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewLine {
    pub text: String,
    pub style: Style,
    /// For a checklist item: whether it is ticked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Heading,
    Bullet,
    Number,
    Check,
    Text,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unknown_kind_is_refused_with_the_ones_there_are() {
        let e = validate("spreadsheet", &json!({})).unwrap_err().to_string();
        assert!(e.contains("document"), "{e}");
    }

    #[test]
    fn an_unknown_kind_has_an_empty_preview_rather_than_an_error() {
        assert_eq!(preview("trip", &json!({})), Preview::default());
    }

    #[test]
    fn every_kind_has_its_own_id() {
        let mut ids: Vec<&str> = KINDS.iter().map(|k| k.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), KINDS.len());
    }
}
