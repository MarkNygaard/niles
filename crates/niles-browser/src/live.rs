//! The live view: the page as pictures while somebody watches, and the
//! input of a person who took the browser over.
//!
//! Frames are Chromium's screencast, JPEG, paced by when each is
//! acknowledged: the browser sends no new frame while two wait. Input
//! is the person's own, never the model's, and is checked here so a
//! stray value can't reach the browser.

use serde::Deserialize;

/// The page's size in CSS pixels, fixed for every session so a
/// screenshot's pixels are the page's.
pub const VIEW_WIDTH: u32 = 1280;
pub const VIEW_HEIGHT: u32 = 900;

/// The longest text one input may insert.
pub const MAX_TEXT: usize = 1000;

/// One picture of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub jpeg: std::sync::Arc<[u8]>,
    /// The page's size it shows, in CSS pixels.
    pub width: u32,
    pub height: u32,
}

/// Which mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Middle,
    Right,
}

impl Button {
    fn name(self) -> &'static str {
        match self {
            Button::Left => "left",
            Button::Middle => "middle",
            Button::Right => "right",
        }
    }

    /// CDP's `buttons` bit while this one is held.
    fn bit(self) -> u8 {
        match self {
            Button::Left => 1,
            Button::Right => 2,
            Button::Middle => 4,
        }
    }
}

/// A button or key going down or coming up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Press {
    Down,
    Up,
}

/// A key with a name, which a page reads by `key` and `keyCode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Enter,
    Backspace,
    Tab,
    Escape,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
}

const NAMED: [(Named, &str, i64); 13] = [
    (Named::Enter, "Enter", 13),
    (Named::Backspace, "Backspace", 8),
    (Named::Tab, "Tab", 9),
    (Named::Escape, "Escape", 27),
    (Named::Delete, "Delete", 46),
    (Named::ArrowUp, "ArrowUp", 38),
    (Named::ArrowDown, "ArrowDown", 40),
    (Named::ArrowLeft, "ArrowLeft", 37),
    (Named::ArrowRight, "ArrowRight", 39),
    (Named::Home, "Home", 36),
    (Named::End, "End", 35),
    (Named::PageUp, "PageUp", 33),
    (Named::PageDown, "PageDown", 34),
];

/// A key: one with a name, or one that types a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Named(Named),
    Char(char),
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let given = String::deserialize(d)?;
        if let Some((named, _, _)) = NAMED.iter().find(|(_, name, _)| *name == given) {
            return Ok(Key::Named(*named));
        }
        let mut chars = given.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if !c.is_control() => Ok(Key::Char(c)),
            _ => Err(serde::de::Error::custom("not a key Niles passes on")),
        }
    }
}

impl Key {
    /// `key`, `code` and the Windows key code CDP wants for it.
    fn cdp(self) -> (String, String, i64) {
        match self {
            Key::Named(named) => {
                let (_, name, code) = NAMED
                    .iter()
                    .find(|(n, _, _)| *n == named)
                    .expect("every named key is in the table");
                ((*name).to_string(), (*name).to_string(), *code)
            }
            Key::Char(c) => {
                let upper = c.to_ascii_uppercase();
                let (code, vk) = if upper.is_ascii_alphabetic() {
                    (format!("Key{upper}"), i64::from(u32::from(upper)))
                } else if c.is_ascii_digit() {
                    (format!("Digit{c}"), i64::from(u32::from(c)))
                } else if c == ' ' {
                    ("Space".to_string(), 32)
                } else {
                    (String::new(), 0)
                };
                (c.to_string(), code, vk)
            }
        }
    }
}

/// Alt, Ctrl, Meta and Shift, as CDP's bit mask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Modifiers(pub u8);

impl Modifiers {
    const CTRL: u8 = 2;
    const META: u8 = 4;

    /// Whether a character key would type its character: not under
    /// Ctrl or Meta, which make it a shortcut.
    fn types(self) -> bool {
        self.0 & (Self::CTRL | Self::META) == 0
    }
}

/// One thing the person did in the live view.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Input {
    /// The pointer moved.
    Move { x: f64, y: f64 },
    /// A button went down or up.
    Press {
        x: f64,
        y: f64,
        press: Press,
        button: Button,
        #[serde(default = "one")]
        clicks: u8,
    },
    /// The wheel, or a finger's drag on a phone.
    Wheel { x: f64, y: f64, dx: f64, dy: f64 },
    /// A key went down or up.
    Key {
        press: Press,
        key: Key,
        #[serde(default)]
        modifiers: Modifiers,
    },
    /// Text typed in one go, from a phone's own keyboard.
    Text { text: String },
}

fn one() -> u8 {
    1
}

impl Input {
    /// Whether it is one Niles passes on: a point on the page, a wheel
    /// of a sensible size, at most three clicks, text of a sensible
    /// length.
    pub fn checked(&self) -> bool {
        let on_page = |x: f64, y: f64| {
            x.is_finite()
                && y.is_finite()
                && (0.0..=f64::from(VIEW_WIDTH)).contains(&x)
                && (0.0..=f64::from(VIEW_HEIGHT)).contains(&y)
        };
        match self {
            Input::Move { x, y } => on_page(*x, *y),
            Input::Press { x, y, clicks, .. } => on_page(*x, *y) && (1..=3).contains(clicks),
            Input::Wheel { x, y, dx, dy } => {
                on_page(*x, *y)
                    && [dx, dy]
                        .iter()
                        .all(|d| d.is_finite() && d.abs() <= f64::from(VIEW_HEIGHT) * 4.0)
            }
            Input::Key { .. } => true,
            Input::Text { text } => {
                !text.is_empty()
                    && text.chars().count() <= MAX_TEXT
                    && !text.chars().any(|c| c.is_control() && c != '\n')
            }
        }
    }

    /// The CDP calls it is, in order.
    pub(crate) fn calls(&self) -> Vec<(&'static str, serde_json::Value)> {
        use serde_json::json;
        match self {
            Input::Move { x, y } => vec![(
                "Input.dispatchMouseEvent",
                json!({"type": "mouseMoved", "x": x, "y": y}),
            )],
            Input::Press {
                x,
                y,
                press,
                button,
                clicks,
            } => {
                let (kind, buttons) = match press {
                    Press::Down => ("mousePressed", button.bit()),
                    Press::Up => ("mouseReleased", 0),
                };
                vec![(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": kind,
                        "x": x,
                        "y": y,
                        "button": button.name(),
                        "buttons": buttons,
                        "clickCount": clicks,
                    }),
                )]
            }
            Input::Wheel { x, y, dx, dy } => vec![(
                "Input.dispatchMouseEvent",
                json!({"type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy}),
            )],
            Input::Key {
                press,
                key,
                modifiers,
            } => {
                let (name, code, vk) = key.cdp();
                let mut event = json!({
                    "type": if *press == Press::Down { "rawKeyDown" } else { "keyUp" },
                    "key": name,
                    "code": code,
                    "windowsVirtualKeyCode": vk,
                    "nativeVirtualKeyCode": vk,
                    "modifiers": modifiers.0,
                });
                // A key that types sends its character with the key going
                // down, as a real keyboard's does.
                let text = match key {
                    Key::Char(c) => Some(c.to_string()),
                    Key::Named(Named::Enter) => Some("\r".to_string()),
                    Key::Named(_) => None,
                };
                if *press == Press::Down
                    && modifiers.types()
                    && let Some(text) = text
                {
                    event["type"] = json!("keyDown");
                    event["text"] = json!(text);
                    event["unmodifiedText"] = json!(text);
                }
                vec![("Input.dispatchKeyEvent", event)]
            }
            Input::Text { text } => vec![("Input.insertText", json!({"text": text}))],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(value: serde_json::Value) -> Option<Input> {
        serde_json::from_value(value).ok()
    }

    #[test]
    fn a_click_reads_and_becomes_a_press() {
        let down =
            input(json!({"type": "press", "x": 10, "y": 20, "press": "down", "button": "left"}))
                .expect("reads");
        assert!(down.checked());
        let calls = down.calls();
        assert_eq!(calls[0].0, "Input.dispatchMouseEvent");
        assert_eq!(calls[0].1["type"], "mousePressed");
        assert_eq!(calls[0].1["buttons"], 1);
        assert_eq!(calls[0].1["clickCount"], 1);
    }

    #[test]
    fn a_point_off_the_page_or_not_a_number_is_refused() {
        for (x, y) in [(-1.0, 5.0), (5.0, 901.0), (1281.0, 5.0), (f64::NAN, 5.0)] {
            assert!(!Input::Move { x, y }.checked(), "{x},{y}");
        }
        assert!(
            !Input::Press {
                x: 1.0,
                y: 1.0,
                press: Press::Down,
                button: Button::Left,
                clicks: 9
            }
            .checked()
        );
    }

    #[test]
    fn a_key_is_a_named_one_or_one_character() {
        assert_eq!(
            input(json!({"type": "key", "press": "down", "key": "Enter"})),
            Some(Input::Key {
                press: Press::Down,
                key: Key::Named(Named::Enter),
                modifiers: Modifiers(0)
            })
        );
        assert!(input(json!({"type": "key", "press": "down", "key": "a"})).is_some());
        for refused in ["F12", "ab", "", "\u{7}"] {
            assert!(
                input(json!({"type": "key", "press": "down", "key": refused})).is_none(),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn a_character_key_types_its_character_unless_it_is_a_shortcut() {
        let typed = Input::Key {
            press: Press::Down,
            key: Key::Char('a'),
            modifiers: Modifiers(8),
        }
        .calls();
        assert_eq!(typed[0].1["type"], "keyDown");
        assert_eq!(typed[0].1["text"], "a");
        assert_eq!(typed[0].1["windowsVirtualKeyCode"], 65);
        let shortcut = Input::Key {
            press: Press::Down,
            key: Key::Char('a'),
            modifiers: Modifiers(2),
        }
        .calls();
        assert_eq!(shortcut[0].1["type"], "rawKeyDown");
        assert!(shortcut[0].1.get("text").is_none());
        let backspace = Input::Key {
            press: Press::Down,
            key: Key::Named(Named::Backspace),
            modifiers: Modifiers(0),
        }
        .calls();
        assert_eq!(backspace[0].1["windowsVirtualKeyCode"], 8);
        assert!(backspace[0].1.get("text").is_none());
    }

    #[test]
    fn text_is_inserted_whole_and_kept_to_a_sensible_length() {
        let text = Input::Text {
            text: "hello there".into(),
        };
        assert!(text.checked());
        assert_eq!(text.calls()[0].0, "Input.insertText");
        assert!(
            !Input::Text {
                text: "x".repeat(MAX_TEXT + 1)
            }
            .checked()
        );
        assert!(
            !Input::Text {
                text: String::new()
            }
            .checked()
        );
    }
}
