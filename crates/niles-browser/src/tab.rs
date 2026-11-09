//! The seam between Niles and a browser, and the types the page script
//! and the tools pass through it.
//!
//! [`Driver`] is a browser, [`Tab`] is one of its sessions' page. They
//! live here beside [`crate::Browser`] and [`crate::Session`], so
//! `niles-bin` can test against fakes.

use serde::Deserialize;
use url::Url;

use crate::Result;

/// Chromium's backend node id, which the page can't read or move. The
/// held-step check keys on it, so a page can't swap an element for a
/// look-alike between the card and the click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub i64);

/// The `[12]` the model names when it means an element. Stable for an
/// element across reads of one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reference(pub u32);

impl Reference {
    /// Parses a positive integer or a numeric string, from the tool's
    /// JSON arguments. Zero, negatives and junk are `None`.
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        let number = match value {
            serde_json::Value::Number(n) => n.as_u64()?,
            serde_json::Value::String(s) => s.trim().parse::<u64>().ok()?,
            _ => return None,
        };
        u32::try_from(number).ok().filter(|n| *n > 0).map(Reference)
    }
}

/// What an element is, in the script's one lowercase word.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum Kind {
    Link,
    Button,
    TextBox,
    Password,
    Checkbox,
    Radio,
    Select,
    File,
    Other,
}

impl Kind {
    /// The word the card text uses.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Link => "link",
            Kind::Button => "button",
            Kind::TextBox => "text field",
            Kind::Password => "password field",
            Kind::Checkbox => "checkbox",
            Kind::Radio => "radio button",
            Kind::Select => "drop-down list",
            Kind::File => "file field",
            Kind::Other => "element",
        }
    }
}

impl TryFrom<String> for Kind {
    type Error = String;

    fn try_from(word: String) -> std::result::Result<Self, Self::Error> {
        Ok(match word.as_str() {
            "link" => Kind::Link,
            "button" => Kind::Button,
            "textbox" => Kind::TextBox,
            "password" => Kind::Password,
            "checkbox" => Kind::Checkbox,
            "radio" => Kind::Radio,
            "select" => Kind::Select,
            "file" => Kind::File,
            _ => Kind::Other,
        })
    }
}

/// One option of a drop-down list, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Choice {
    pub label: String,
    pub value: String,
}

/// An element the page script resolved: what it is, how it's labelled,
/// and whether it can be acted on.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub node: NodeId,
    pub reference: Reference,
    pub kind: Kind,
    pub label: String,
    pub link: Option<String>,
    pub in_form: bool,
    pub disabled: bool,
    pub visible: bool,
    pub options: Vec<Choice>,
}

impl Element {
    /// Whether `now` is still what this element was when its card was
    /// made: same node, kind, label, link and options. Address and
    /// enabled state are checked separately.
    pub fn same_as(&self, now: &Element) -> bool {
        self.node == now.node
            && self.kind == now.kind
            && self.label == now.label
            && self.link == now.link
            && self.options == now.options
    }
}

/// A page read as text for the model.
#[derive(Debug, Clone, Deserialize)]
pub struct Reading {
    pub address: String,
    pub title: String,
    pub text: String,
    /// The page shows a CAPTCHA, which Niles never solves.
    #[serde(default)]
    pub captcha: bool,
    /// The page asks for a code the site sent.
    #[serde(default)]
    pub asks_code: bool,
    pub more_above: bool,
    pub more_below: bool,
}

/// A point in CSS pixels, the screenshot's pixels at device scale 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// A rectangle in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// The point at the rectangle's centre.
    pub(crate) fn centre(&self) -> Point {
        Point {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        }
    }
}

/// Where an element would be acted on, or why it can't be.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum Aim {
    /// The point a click lands on, and the element's visible outline.
    Clear { at: Point, outline: Rect },
    /// Something else is on top of the element at its point.
    Covered { outline: Rect },
    /// The element isn't on screen at all.
    Hidden,
}

impl Aim {
    /// The outline to draw on the picture, when there is something to
    /// draw around.
    pub fn outline(&self) -> Option<Rect> {
        match self {
            Aim::Clear { outline, .. } | Aim::Covered { outline } => Some(*outline),
            Aim::Hidden => None,
        }
    }
}

/// How to scroll the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    Down,
    Up,
    Top,
    Bottom,
}

/// What a page asks for to sign in, in its frames on the login's
/// origins.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Asks {
    /// The page wants a person to show they aren't a robot.
    Captcha,
    /// A password field, with a username field beside it or not.
    Password { username: bool },
    /// Only a username field: the password comes on the next page.
    Username,
    /// A code the site sent.
    Code,
    /// No sign-in form the script may fill.
    #[default]
    Nothing,
}

/// Which fields a fill set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Filled {
    pub username: bool,
    pub password: bool,
}

/// Whether `address`'s origin, in its ASCII form, is one of `origins`.
/// False when the address doesn't parse.
pub fn on_sites(address: &str, origins: &[String]) -> bool {
    let Ok(url) = Url::parse(address) else {
        return false;
    };
    let origin = url.origin();
    origin.is_tuple() && origins.contains(&origin.ascii_serialization())
}

/// A browser: what opens sessions and can be restarted by its supervisor.
#[async_trait::async_trait]
pub trait Driver: Send + Sync {
    /// A new empty browser context with one page, every connection
    /// through Niles's proxy.
    async fn session(&self) -> Result<std::sync::Arc<dyn Tab>>;

    /// Close the browser, for its supervisor to start again.
    async fn restart(&self) -> Result<()>;

    /// Whether the connection to the browser is gone.
    fn closed(&self) -> bool;
}

/// One session's page: reading, aiming, acting.
#[async_trait::async_trait]
pub trait Tab: Send + Sync {
    /// Navigate the page to `url`.
    async fn open(&self, url: &Url) -> Result<()>;

    /// The page as text with `[n]` references, from the top of the view.
    async fn read(&self) -> Result<Reading>;

    /// Scroll the page.
    async fn scroll(&self, way: Scroll) -> Result<()>;

    /// The page's address as `location.href` has it.
    async fn address(&self) -> Result<String>;

    /// The element the model's `[n]` names. `None` means no such
    /// reference on this page now.
    async fn element(&self, reference: Reference) -> Result<Option<Element>>;

    /// The same node as it is now, `None` once it is gone.
    async fn again(&self, element: &Element) -> Result<Option<Element>>;

    /// Where the element would be acted on, scrolling it into view first.
    async fn aim(&self, element: &Element) -> Result<Aim>;

    /// A PNG of the view, for a card's picture.
    async fn screenshot(&self) -> Result<Vec<u8>>;

    /// Click a point the page script aimed at.
    async fn click(&self, at: Point) -> Result<()>;

    /// Replace what the field holds with `text`. Never used for a
    /// password field.
    async fn type_into(&self, element: &Element, text: &str) -> Result<()>;

    /// Choose an option of a drop-down list by its value.
    async fn choose(&self, element: &Element, value: &str) -> Result<()>;

    /// Submit the form the element belongs to.
    async fn submit(&self, element: &Element) -> Result<()>;

    /// What the page asks for to sign in, in its frames on `origins`.
    async fn asks(&self, origins: &[String]) -> Result<Asks>;

    /// Fill the login fields in a frame on `origins`, each frame's
    /// script checking its own origin as it sets the value; submits and
    /// waits out the page.
    async fn fill_login(
        &self,
        origins: &[String],
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Filled>;

    /// Type the sign-in code into the page's code fields, on `origins`.
    async fn fill_code(&self, origins: &[String], code: &str) -> Result<bool>;

    /// From now on the page's own navigations stay on `origins`;
    /// history is forgotten.
    async fn stay_on(&self, origins: &[String]) -> Result<()>;

    /// The cookies of `origins`' hosts and the storage of their frames,
    /// as opaque JSON.
    async fn profile(&self, origins: &[String]) -> Result<String>;

    /// Put a profile back and open the page again.
    async fn restore(&self, origins: &[String], profile: &str) -> Result<()>;

    /// End the session, releasing its browser context.
    async fn close(&self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn element() -> Element {
        Element {
            node: NodeId(7),
            reference: Reference(3),
            kind: Kind::Button,
            label: "Add to basket".into(),
            link: None,
            in_form: true,
            disabled: false,
            visible: true,
            options: vec![],
        }
    }

    #[test]
    fn reference_parses_a_number_or_a_numeric_string() {
        assert_eq!(Reference::parse(&json!(12)), Some(Reference(12)));
        assert_eq!(Reference::parse(&json!(" 12 ")), Some(Reference(12)));
    }

    #[test]
    fn reference_refuses_zero_negatives_and_junk() {
        assert_eq!(Reference::parse(&json!(0)), None);
        assert_eq!(Reference::parse(&json!(-3)), None);
        assert_eq!(Reference::parse(&json!("12a")), None);
        assert_eq!(Reference::parse(&json!(1.5)), None);
        assert_eq!(Reference::parse(&json!(true)), None);
        assert_eq!(Reference::parse(&Value::Null), None);
    }

    #[test]
    fn unknown_words_map_to_other() {
        assert_eq!(Kind::try_from("textbox".to_string()), Ok(Kind::TextBox));
        assert_eq!(Kind::try_from("wibble".to_string()), Ok(Kind::Other));
        assert_eq!(Kind::try_from("".to_string()), Ok(Kind::Other));
    }

    #[test]
    fn same_as_compares_the_page_not_the_reference() {
        let mut now = element();
        assert!(element().same_as(&now));
        now.label = "Buy".into();
        assert!(!element().same_as(&now));
        let mut now = element();
        now.link = Some("https://shop.example/buy".into());
        assert!(!element().same_as(&now));
        let mut now = element();
        now.options = vec![Choice {
            label: "2 m".into(),
            value: "2".into(),
        }];
        assert!(!element().same_as(&now));
    }

    #[test]
    fn on_sites_matches_the_ascii_origin() {
        let origins = vec!["https://shop.example".to_string()];
        assert!(on_sites("https://shop.example/login", &origins));
        // A subdomain of another site that merely ends the same isn't
        // one of the login's origins.
        assert!(!on_sites("https://shop.example.evil.example/", &origins));
        // Only https: the login's origins are https and so is the page
        // script's check.
        assert!(!on_sites("http://shop.example/", &origins));
        assert!(on_sites("blob:https://shop.example/x", &origins));
        // An IDN page answers against its punycode origin.
        let idn = vec!["https://xn--bcher-kva.example".to_string()];
        assert!(on_sites("https://bücher.example/", &idn));
        // Junk never parses onto a login's origin.
        assert!(!on_sites("not a url", &origins));
        assert!(!on_sites("", &origins));
    }
}
