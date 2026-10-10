//! Menu presentation configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::HashSet;

/// The menu entries that can be moved. Home and My profile are not
/// among them: Home is the main navigation's first entry and My profile
/// the avatar menu's. Settings can move but not hide — it is the way
/// back to this.
pub const MOVABLE: &[&str] = &["groceries", "chat", "media", "settings"];

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
/// Two menus: the avatar menu in the top corner, and the main
/// navigation — the bar along the bottom of a phone, the links in a
/// desktop's header. Like `[rooms]`, presentation for the whole house.
/// An entry neither list names goes to the main navigation, after the
/// arranged ones — a page added in a later version has to show up
/// somewhere. Only `hidden` hides.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuConfig {
    /// The main navigation after Home, in order.
    #[serde(default)]
    pub order: Vec<String>,
    /// The avatar menu after My profile, in order. An entry here is not
    /// in the main navigation, whatever `order` says.
    #[serde(default = "default_avatar")]
    pub avatar: Vec<String>,
    /// The movable entries left out of the menu. Their pages still work.
    #[serde(default)]
    pub hidden: Vec<String>,
    /// When the Media entry shows.
    #[serde(default)]
    pub media: MediaShown,
}

fn default_avatar() -> Vec<String> {
    vec!["settings".into()]
}

impl Default for MenuConfig {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            avatar: default_avatar(),
            hidden: Vec::new(),
            media: MediaShown::default(),
        }
    }
}

impl MenuConfig {
    pub fn validate(&self) -> Result<()> {
        for list in [&self.order, &self.avatar] {
            let mut seen = HashSet::new();
            for entry in list {
                known(entry)?;
                if !seen.insert(entry) {
                    return Err(invalid(format!("{entry:?} is listed twice")));
                }
            }
        }
        for entry in &self.hidden {
            known(entry)?;
            // The way back to the menu's settings cannot be put away.
            if entry == "settings" {
                return Err(invalid("Settings cannot be hidden".into()));
            }
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
            avatar: default_avatar(),
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
    fn settings_start_in_the_avatar_menu() {
        let menu: MenuConfig = toml::from_str("").unwrap();
        assert_eq!(menu.avatar, ["settings"]);
        // Moved to the main navigation, the avatar menu is written empty.
        let menu: MenuConfig = toml::from_str("avatar = []").unwrap();
        assert!(menu.avatar.is_empty());
    }

    #[test]
    fn settings_cannot_be_hidden() {
        let err = menu(&[], &["settings"]).validate().unwrap_err();
        assert!(err.to_string().contains("Settings"), "{err}");
    }

    #[test]
    fn rejects_an_entry_listed_twice() {
        let err = menu(&["chat", "chat"], &[]).validate().unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }
}
