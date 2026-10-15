//! Menu presentation configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

/// The menu entries that can be moved. Home and My profile are not
/// among them: Home is the main navigation's first entry and My profile
/// the avatar menu's. Settings can move but not hide — it is the way
/// back to this.
pub const MOVABLE: &[&str] = &[
    "groceries",
    "chat",
    "media",
    "settings",
    "automations",
    "skills",
];

/// Where a plugin's widget goes, as `[menu.widgets.<plugin>]` writes it.
pub const WIDGET_ON_FRONT_PAGE: &str = "front";
pub const WIDGET_HIDDEN: &str = "hidden";
/// Before a room id: `"room:kitchen"`.
pub const WIDGET_IN_ROOM: &str = "room:";

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
///
/// A plugin's page is an entry like the built-in ones, as
/// `<plugin-id>.<page-id>` (`spotprice.electricity`). This section cannot
/// know which plugins are installed, so any entry of that shape is
/// accepted; the app shows only those of installed plugins, and removing
/// a plugin takes its entries out ([`MenuConfig::forget_plugin`]).
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
    /// Where plugins' widgets go, when not where their manifests start
    /// them: by plugin id, then widget id, `"front"` for the front page,
    /// `"room:<id>"` for a room's card, or `"hidden"`.
    #[serde(default)]
    pub widgets: BTreeMap<String, BTreeMap<String, String>>,
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
            widgets: BTreeMap::new(),
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
        for (plugin, widgets) in &self.widgets {
            if !is_plugin_id(plugin) {
                return Err(invalid(format!(
                    "widgets: {plugin:?} is not a plugin id like spotprice"
                )));
            }
            for (widget, place) in widgets {
                if !is_identifier(widget) {
                    return Err(invalid(format!(
                        "widgets.{plugin}: {widget:?} is not a widget id like price_now"
                    )));
                }
                let room = place.strip_prefix(WIDGET_IN_ROOM);
                let fits = place == WIDGET_ON_FRONT_PAGE
                    || place == WIDGET_HIDDEN
                    || room.is_some_and(is_identifier);
                if !fits {
                    return Err(invalid(format!(
                        "widgets.{plugin}.{widget} is {place:?}; it is \"front\", \"hidden\" or \
                         \"room:<room id>\""
                    )));
                }
            }
        }
        Ok(())
    }

    /// What removing plugin `id` takes out of this section: the lists
    /// without its pages, for those that had any, as a patch for
    /// `[menu]`, and whether it had widgets placed (which the caller
    /// drops by resetting `menu.widgets.<id>`).
    pub fn forget_plugin(&self, id: &str) -> (toml::Table, bool) {
        let prefix = format!("{id}.");
        let mut patch = toml::Table::new();
        for (key, list) in [
            ("order", &self.order),
            ("avatar", &self.avatar),
            ("hidden", &self.hidden),
        ] {
            if list.iter().any(|e| e.starts_with(&prefix)) {
                let kept = list
                    .iter()
                    .filter(|e| !e.starts_with(&prefix))
                    .map(|e| toml::Value::String(e.clone()))
                    .collect();
                patch.insert(key.into(), toml::Value::Array(kept));
            }
        }
        (patch, self.widgets.contains_key(id))
    }
}

fn known(entry: &str) -> Result<()> {
    if MOVABLE.contains(&entry) || is_plugin_entry(entry) {
        Ok(())
    } else {
        Err(invalid(format!(
            "{entry:?} is not a menu entry that can be moved or hidden (expected one of {}, \
             or a plugin's page as <plugin-id>.<page-id>)",
            MOVABLE.join(", ")
        )))
    }
}

/// `spotprice.electricity`: a plugin's id, a dot, and its page's id.
fn is_plugin_entry(entry: &str) -> bool {
    entry
        .split_once('.')
        .is_some_and(|(plugin, page)| is_plugin_id(plugin) && is_identifier(page))
}

/// The manifest's rule for a plugin id: `[a-z0-9_]`, 2 to 32
/// characters, starting with a letter.
fn is_plugin_id(id: &str) -> bool {
    (2..=32).contains(&id.len())
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && is_identifier(id)
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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
            widgets: BTreeMap::new(),
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
    fn a_plugins_page_is_an_entry_like_the_others() {
        assert!(
            menu(
                &["chat", "spotprice.electricity"],
                &["spotprice.electricity"]
            )
            .validate()
            .is_ok()
        );
        for wrong in [
            "spotprice",
            "Spot.prices",
            "x.page",
            "spotprice.",
            ".page",
            "a.b.c",
        ] {
            assert!(menu(&[wrong], &[]).validate().is_err(), "{wrong}");
        }
    }

    #[test]
    fn a_widget_goes_to_the_front_page_a_room_or_nowhere() {
        let read = |text: &str| toml::from_str::<MenuConfig>(text).unwrap().validate();
        assert!(
            read(
                "[widgets.spotprice]\nprice_now = \"front\"\nchart = \"room:kitchen\"\nold = \"hidden\""
            )
            .is_ok()
        );
        assert!(read("[widgets.spotprice]\nprice_now = \"kitchen\"").is_err());
        assert!(read("[widgets.spotprice]\nprice_now = \"room:Kitchen\"").is_err());
        assert!(read("[widgets.Spot]\nprice_now = \"front\"").is_err());
    }

    #[test]
    fn forgetting_a_plugin_takes_out_its_pages_and_only_its() {
        let mut config = menu(
            &["chat", "spotprice.electricity", "spotprice2.page"],
            &["spotprice.history"],
        );
        config.widgets.insert(
            "spotprice".into(),
            BTreeMap::from([("now".into(), "front".into())]),
        );
        let (patch, widgets) = config.forget_plugin("spotprice");
        assert_eq!(
            patch["order"],
            toml::Value::Array(vec!["chat".into(), "spotprice2.page".into()])
        );
        assert_eq!(patch["hidden"], toml::Value::Array(Vec::new()));
        assert!(
            !patch.contains_key("avatar"),
            "untouched lists are not written"
        );
        assert!(widgets);

        let (patch, widgets) = menu(&["chat"], &[]).forget_plugin("spotprice");
        assert!(patch.is_empty());
        assert!(!widgets);
    }

    #[test]
    fn rejects_an_entry_listed_twice() {
        let err = menu(&["chat", "chat"], &[]).validate().unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }
}
