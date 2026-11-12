//! A fake browser for the tests: a driver handing out fake tabs, and
//! tabs over a plain state a test can change between the hold and the
//! answer — the page moving on, the element going, something covering
//! it.

use niles_browser::{
    Aim, Asks, Checkout, Choice, Driver, Element, Filled, Kind, NodeId, Point, Reading, Rect,
    Reference, Result, Scroll, Tab,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use url::Url;

/// Gives each `session()` a new fake tab, and remembers them so a test
/// can reach the one a tool used.
pub(crate) struct FakeDriver {
    pub(crate) tabs: Mutex<Vec<Arc<FakeTab>>>,
    pub(crate) closed: AtomicBool,
}

impl Default for FakeDriver {
    fn default() -> Self {
        Self {
            tabs: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl Driver for FakeDriver {
    async fn session(&self) -> Result<Arc<dyn Tab>> {
        let tab = Arc::new(FakeTab::new());
        self.tabs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tab.clone());
        Ok(tab)
    }

    async fn restart(&self) -> Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// What a fake tab pretends the page is.
#[derive(Default)]
pub(crate) struct FakeState {
    pub(crate) address: String,
    pub(crate) elements: HashMap<u32, Element>,
    pub(crate) covered: HashSet<i64>,
    /// What was done, in order: "click", "type {node} {text}", and so on.
    pub(crate) done: Vec<String>,
    /// What was opened.
    pub(crate) opened: Vec<String>,
    pub(crate) closed: bool,
    /// A real PNG, so the picture pipeline is exercised too.
    pub(crate) png: Vec<u8>,
    /// Extra text appended to the reading after the element lines.
    pub(crate) text: String,
    /// When set, one read fails with this fixed reason.
    pub(crate) fail_read: Option<String>,
    /// What `asks` answers now.
    pub(crate) asks: Asks,
    /// Queued pages: `(address, asks)` a submit moves on to, in order.
    pub(crate) then: Vec<(String, Asks)>,
    /// When true, `stay_on` is recorded but not enforced (a leaky guard).
    pub(crate) leaky: bool,
    /// What `fill_login` and `fill_code` typed: (address, field, value).
    pub(crate) typed: Vec<(String, String, String)>,
    /// What `profile` answers.
    pub(crate) profile: Option<String>,
    /// What `restore` was given, in order.
    pub(crate) restored: Vec<String>,
    /// What the reading reports.
    pub(crate) captcha: bool,
    pub(crate) asks_code: bool,
    /// What `checkout` answers; its `card_details` is the reading's too.
    pub(crate) checkout: Checkout,
}

/// What `asks` answers for the state: the standing ask. A submit moves
/// the page on to the next queued `(address, asks)`, which becomes the
/// standing one.
fn current(state: &FakeState) -> Asks {
    state.asks
}

pub(crate) struct FakeTab {
    pub(crate) state: Mutex<FakeState>,
}

impl Default for FakeTab {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeTab {
    pub(crate) fn new() -> Self {
        let image = image::RgbImage::from_pixel(64, 48, image::Rgb([0xE8, 0xE8, 0xE8]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("encoding a plain RGB PNG cannot fail");
        Self {
            state: Mutex::new(FakeState {
                png: png.into_inner(),
                ..FakeState::default()
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait::async_trait]
impl Tab for FakeTab {
    async fn open(&self, url: &Url) -> Result<()> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        state.opened.push(url.to_string());
        state.address = url.to_string();
        Ok(())
    }

    async fn read(&self) -> Result<Reading> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        if let Some(reason) = state.fail_read.take() {
            return Err(niles_browser::Error::Answer {
                method: "__niles.read",
                reason,
            });
        }
        let mut text = state
            .elements
            .values()
            .map(|e| format!("[{}] {} \"{}\"", e.reference.0, e.kind.word(), e.label))
            .collect::<Vec<_>>()
            .join("\n");
        if !state.text.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&state.text);
        }
        Ok(Reading {
            address: state.address.clone(),
            title: "A fake page".into(),
            text,
            captcha: state.captcha,
            asks_code: state.asks_code,
            card_details: state.checkout.card_details,
            more_above: false,
            more_below: false,
        })
    }

    async fn scroll(&self, _way: Scroll) -> Result<()> {
        match self.state().closed {
            true => Err(niles_browser::Error::Closed),
            false => Ok(()),
        }
    }

    async fn checkout(&self) -> Result<Checkout> {
        let state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        Ok(state.checkout.clone())
    }

    async fn address(&self) -> Result<String> {
        let state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        Ok(state.address.clone())
    }

    async fn element(&self, reference: Reference) -> Result<Option<Element>> {
        let state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        Ok(state.elements.get(&reference.0).cloned())
    }

    async fn again(&self, element: &Element) -> Result<Option<Element>> {
        let state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        Ok(state
            .elements
            .values()
            .find(|e| e.node == element.node)
            .cloned())
    }

    async fn aim(&self, element: &Element) -> Result<Aim> {
        let state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        let outline = Rect {
            x: 4.0,
            y: 4.0,
            width: 40.0,
            height: 20.0,
        };
        if state.covered.contains(&element.node.0) {
            return Ok(Aim::Covered { outline });
        }
        Ok(Aim::Clear {
            at: Point {
                x: outline.x + outline.width / 2.0,
                y: outline.y + outline.height / 2.0,
            },
            outline,
        })
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        let state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        Ok(state.png.clone())
    }

    async fn click(&self, _at: Point) -> Result<()> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        state.done.push("click".into());
        Ok(())
    }

    async fn type_into(&self, element: &Element, text: &str) -> Result<()> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        state.done.push(format!("type {} {text}", element.node.0));
        Ok(())
    }

    async fn choose(&self, element: &Element, value: &str) -> Result<()> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        state
            .done
            .push(format!("choose {} {value}", element.node.0));
        Ok(())
    }

    async fn submit(&self, element: &Element) -> Result<()> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        state.done.push(format!("submit {}", element.node.0));
        Ok(())
    }

    async fn asks(&self, _origins: &[String]) -> Result<Asks> {
        let state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        Ok(current(&state))
    }

    async fn fill_login(
        &self,
        origins: &[String],
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Filled> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        let address = state.address.clone();
        state.done.push(format!("fill_login {address} {origins:?}"));
        let asks = current(&state);
        let filled = match asks {
            Asks::Password { .. } => Filled {
                username: username.is_some(),
                password: password.is_some(),
            },
            Asks::Username => Filled {
                username: username.is_some(),
                password: false,
            },
            _ => Filled::default(),
        };
        if filled.username
            && let Some(username) = username
        {
            state
                .typed
                .push((address.clone(), "username".into(), username.into()));
        }
        if filled.password
            && let Some(password) = password
        {
            state
                .typed
                .push((address.clone(), "password".into(), password.into()));
        }
        if filled.username || filled.password {
            if !state.then.is_empty() {
                // The submit moved the page on.
                let (next, asks) = state.then.remove(0);
                state.address = next;
                state.asks = asks;
            } else if filled.password {
                // Signed in: the page asks for nothing more.
                state.asks = Asks::Nothing;
            }
        }
        Ok(filled)
    }

    async fn fill_code(&self, _origins: &[String], code: &str) -> Result<bool> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        if current(&state) != Asks::Code {
            return Ok(false);
        }
        let address = state.address.clone();
        state.typed.push((address, "code".into(), code.into()));
        if !state.then.is_empty() {
            let (next, asks) = state.then.remove(0);
            state.address = next;
            state.asks = asks;
        } else {
            state.asks = Asks::Nothing;
        }
        Ok(true)
    }

    async fn stay_on(&self, origins: &[String]) -> Result<()> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        let _ = state.leaky;
        state.done.push(format!("stay_on {origins:?}"));
        Ok(())
    }

    async fn profile(&self, origins: &[String]) -> Result<String> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        state.done.push(format!("profile {origins:?}"));
        Ok(state.profile.clone().unwrap_or_default())
    }

    async fn restore(&self, _origins: &[String], profile: &str) -> Result<()> {
        let mut state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        state.restored.push(profile.to_string());
        // The restored profile signed the page in: it asks for nothing.
        state.asks = Asks::Nothing;
        Ok(())
    }

    async fn close(&self) {
        self.state().closed = true;
    }
}

/// Set the fake's page to an address asking `asks`: the shape of a
/// sign-in page.
pub(crate) fn login_page(tab: &FakeTab, address: &str, asks: Asks) {
    let mut state = tab.state.lock().unwrap_or_else(|e| e.into_inner());
    state.address = address.to_string();
    state.asks = asks;
}

/// A visible, enabled button labelled `label`.
pub(crate) fn button(reference: u32, node: i64, label: &str) -> Element {
    element(reference, node, Kind::Button, label)
}

/// A visible, enabled text field labelled `label`.
pub(crate) fn textbox(reference: u32, node: i64, label: &str) -> Element {
    element(reference, node, Kind::TextBox, label)
}

/// A password field.
pub(crate) fn password(reference: u32, node: i64, label: &str) -> Element {
    element(reference, node, Kind::Password, label)
}

/// A drop-down list with `options` as (label, value) pairs.
pub(crate) fn select(reference: u32, node: i64, label: &str, options: &[(&str, &str)]) -> Element {
    let mut element = element(reference, node, Kind::Select, label);
    element.in_form = true;
    element.options = options
        .iter()
        .map(|(label, value)| Choice {
            label: label.to_string(),
            value: value.to_string(),
        })
        .collect();
    element
}

/// A visible, enabled element of `kind` labelled `label`.
fn element(reference: u32, node: i64, kind: Kind, label: &str) -> Element {
    Element {
        node: NodeId(node),
        reference: Reference(reference),
        kind,
        label: label.into(),
        link: None,
        in_form: false,
        disabled: false,
        visible: true,
        options: Vec::new(),
    }
}
