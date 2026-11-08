//! A fake browser for the tests: a driver handing out fake tabs, and
//! tabs over a plain state a test can change between the hold and the
//! answer — the page moving on, the element going, something covering
//! it.

use niles_browser::{
    Aim, Choice, Driver, Element, Kind, NodeId, Point, Reading, Rect, Reference, Result, Scroll,
    Tab,
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
        let state = self.state();
        if state.closed {
            return Err(niles_browser::Error::Closed);
        }
        let text = state
            .elements
            .values()
            .map(|e| format!("[{}] {} \"{}\"", e.reference.0, e.kind.word(), e.label))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Reading {
            address: state.address.clone(),
            title: "A fake page".into(),
            text,
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

    async fn close(&self) {
        self.state().closed = true;
    }
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
