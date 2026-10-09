//! Menu presentation configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::HashSet;

/// The menu entries that can be moved and hidden. Home and Me are not
/// among them: Home is where the app opens and Me is the way to the
/// settings, so hiding either would leave no way back to this one.
pub const MOVABLE: &[&str] = &["groceries", "chat", "media"];

/// When the Media entry is in the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MediaShown {
    /// While something plays, or the TV is on: a page for the volume is
    /// only worth a place in the menu while there is a volume to set.
    #[default]
    Playing,
    Always,
    Never,
}

/// `[menu]` section of the config file.
///
/// Like `[rooms]`, presentation for the whole house: the same menu on a
/// phone and a laptop. An entry `order` does not name is not hidden — a
/// page added in a later version has to show up somewhere, and after the
/// arranged ones is where it does not look lost. Only `hidden` hides.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MenuConfig {
    /// The movable entries, in the order they sit between Home and Me.
    #[serde(default)]
    pub order: Vec<String>,
    /// The movable entries left out of the menu. Their pages still work.
    #[serde(default)]
    pub hidden: Vec<String>,
    /// When the Media entry shows.
    #[serde(default)]
    pub media: MediaShown,
}

impl MenuConfig {
    pub fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for entry in &self.order {
            known(entry)?;
            if !seen.insert(entry) {
                return Err(invalid(format!("{entry:?} is listed twice")));
            }
        }
        for entry in &self.hidden {
            known(entry)?;
        }
        Ok(())
    }
}

fn known(entry: &str) -> Result<()> {
    if MOVABLE.contains(&entry) {
        Ok(())
    } else {
        Err(invalid(format!(
            "{entry:?} is not a menu entry that can be moved or hidden (expected one of {})",
            MOVABLE.join(", ")
        )))
    }
}

fn invalid(reason: String) -> Error {
    Error::InvalidSection {
        section: "menu",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(order: &[&str], hidden: &[&str]) -> MenuConfig {
        MenuConfig {
            order: order.iter().map(|s| s.to_string()).collect(),
            hidden: hidden.iter().map(|s| s.to_string()).collect(),
            media: MediaShown::Playing,
        }
    }

    #[test]
    fn nothing_arranged_is_fine() {
        assert!(MenuConfig::default().validate().is_ok());
    }

    #[test]
    fn accepts_the_movable_entries_in_any_order() {
        assert!(
            menu(&["chat", "groceries"], &["groceries"])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn home_and_me_stay_put() {
        let err = menu(&["me", "chat"], &[]).validate().unwrap_err();
        assert!(err.to_string().contains("menu"), "{err}");
        assert!(menu(&[], &["home"]).validate().is_err());
    }

    #[test]
    fn reads_from_the_config_file() {
        let menu: MenuConfig = toml::from_str(
            "order = [\"chat\", \"groceries\"]
hidden = [\"groceries\"]",
        )
        .unwrap();
        assert_eq!(menu.order, ["chat", "groceries"]);
        assert_eq!(menu.hidden, ["groceries"]);
    }

    #[test]
    fn media_shows_while_something_plays_unless_told_otherwise() {
        let menu: MenuConfig = toml::from_str("").unwrap();
        assert_eq!(menu.media, MediaShown::Playing);
        let menu: MenuConfig = toml::from_str("media = \"always\"").unwrap();
        assert_eq!(menu.media, MediaShown::Always);
        assert!(toml::from_str::<MenuConfig>("media = \"sometimes\"").is_err());
    }

    #[test]
    fn rejects_an_entry_listed_twice() {
        let err = menu(&["chat", "chat"], &[]).validate().unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }
}
