//! A session: one browser context's page, read through the isolated
//! world, acted on with the checks Niles means.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;
use url::Url;

use crate::cdp::{Connection, Event};
use crate::error::{Error, Result};
use crate::tab::{
    Aim, Choice, Element, Kind, NodeId, Point, Reading, Rect, Reference, Scroll, Tab,
};

/// Niles's page script, installed in an isolated world the page can't
/// see (`include_str!` keeps it out of Rust string quoting).
const PAGE_JS: &str = include_str!("page.js");

/// How long acting waits for the page to load after it changed.
const SETTLE_LOAD: Duration = Duration::from_secs(15);

/// How long a fresh navigation may take before acting proceeds anyway.
const OPEN_LOAD: Duration = Duration::from_secs(15);

/// A page read settles this long for lazy content after a scroll.
const SCROLL_SETTLE: Duration = Duration::from_millis(400);

/// One session's page. Constructed through [`crate::Browser`].
pub struct Session {
    conn: Arc<Connection>,
    context: String,
    target: String,
    session: String,
    frame: String,
    /// The loader id and execution context id of the isolated world,
    /// cached while the document lives.
    world: Mutex<Option<(String, i64)>>,
    dialogs: JoinHandle<()>,
}

/// The script's `info(el)` answer.
#[derive(Debug, Deserialize)]
struct Info {
    kind: Kind,
    label: String,
    link: Option<String>,
    in_form: bool,
    disabled: bool,
    visible: bool,
    connected: bool,
    #[serde(default)]
    options: Vec<Choice>,
}

impl Info {
    /// The element the info describes, under the reference the model
    /// named it by.
    fn element(self, node: i64, reference: Reference) -> Element {
        Element {
            node: NodeId(node),
            reference,
            kind: self.kind,
            label: self.label,
            link: self.link,
            in_form: self.in_form,
            disabled: self.disabled,
            visible: self.visible,
            options: self.options,
        }
    }
}

impl Session {
    /// Attaches to the session's page: viewport fixed so screenshot
    /// pixels equal CSS pixels, dialogs answered, the frame noted.
    pub(crate) async fn start(
        conn: Arc<Connection>,
        context: String,
        target: String,
        session: String,
    ) -> Result<Arc<Session>> {
        let frame = Self::frame_id(&conn, &session).await?;
        let dialogs = tokio::spawn(dialog_loop(Arc::clone(&conn), session.clone()));
        Ok(Arc::new(Session {
            conn,
            context,
            target,
            session,
            frame,
            world: Mutex::new(None),
            dialogs,
        }))
    }

    /// The main frame's id of the attached page.
    async fn frame_id(conn: &Connection, session: &str) -> Result<String> {
        conn.call("Page.enable", Value::Null, Some(session)).await?;
        conn.call(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": 1280,
                "height": 900,
                "deviceScaleFactor": 1,
                "mobile": false
            }),
            Some(session),
        )
        .await?;
        conn.call("Page.getFrameTree", Value::Null, Some(session))
            .await?
            .get("frameTree")
            .and_then(|t| t.get("frame"))
            .and_then(|f| f.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Answer {
                method: "Page.getFrameTree",
                reason: "no frame id".into(),
            })
    }

    /// One CDP call on this session.
    async fn on(&self, method: &'static str, params: Value) -> Result<Value> {
        self.conn.call(method, params, Some(&self.session)).await
    }

    /// The isolated world's execution context id, making the world when
    /// the document is new.
    async fn world(&self) -> Result<i64> {
        let loader = self
            .on("Page.getFrameTree", Value::Null)
            .await?
            .get("frameTree")
            .and_then(|t| t.get("frame"))
            .and_then(|f| f.get("loaderId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Answer {
                method: "Page.getFrameTree",
                reason: "no loader id".into(),
            })?;
        let mut cached = self.world.lock().await;
        if let Some((seen, context)) = cached.as_ref()
            && seen == &loader
        {
            return Ok(*context);
        }
        let context = self
            .on(
                "Page.createIsolatedWorld",
                json!({
                    "frameId": self.frame,
                    "worldName": "niles",
                    "grantUniveralAccess": false
                }),
            )
            .await?
            .get("executionContextId")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Answer {
                method: "Page.createIsolatedWorld",
                reason: "no executionContextId".into(),
            })?;
        // Install the page script. Its value is nothing; an exception
        // would mean the script broke, which is an Answer.
        self.evaluate_in(context, PAGE_JS.to_string(), true).await?;
        *cached = Some((loader, context));
        Ok(context)
    }

    /// Runs `expression` in the isolated world.
    async fn evaluate(&self, expression: String, by_value: bool) -> Result<Value> {
        let context = self.world().await?;
        self.evaluate_in(context, expression, by_value).await
    }

    /// Runs `expression` in a known context id.
    async fn evaluate_in(&self, context: i64, expression: String, by_value: bool) -> Result<Value> {
        let answer = self
            .on(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "contextId": context,
                    "returnByValue": by_value,
                    "timeout": 10000
                }),
            )
            .await?;
        if let Some(details) = answer.get("exceptionDetails") {
            // The exception text may quote page content; it is never
            // logged, only returned to the caller as an error.
            let reason = details
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(Value::as_str)
                .or_else(|| details.get("text").and_then(Value::as_str))
                .unwrap_or("an exception in the page script")
                .to_string();
            return Err(Error::Answer {
                method: "Runtime.evaluate",
                reason,
            });
        }
        answer.get("result").cloned().ok_or_else(|| Error::Answer {
            method: "Runtime.evaluate",
            reason: "no result".into(),
        })
    }

    /// `__niles.info(this)` on a remote object, by its object id.
    async fn info_of(&self, object: &str) -> Result<Info> {
        let answer = self
            .on(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object,
                    "functionDeclaration": "function(){ return __niles.info(this) }",
                    "returnByValue": true
                }),
            )
            .await?;
        serde_json::from_value(
            answer
                .get("result")
                .and_then(|r| r.get("value"))
                .cloned()
                .ok_or_else(|| Error::Answer {
                    method: "Runtime.callFunctionOn",
                    reason: "no info value".into(),
                })?,
        )
        .map_err(|_| Error::Answer {
            method: "Runtime.callFunctionOn",
            reason: "info wasn't the expected shape".into(),
        })
    }

    /// The element the model's reference names, or `None` when no such
    /// reference is on this page now.
    async fn find(&self, reference: Reference) -> Result<Option<(i64, Info)>> {
        let object = self
            .evaluate(format!("__niles.node({})", reference.0), false)
            .await?;
        let Some(object_id) = object.get("objectId").and_then(Value::as_str) else {
            return Ok(None);
        };
        if object.get("subtype").and_then(Value::as_str) == Some("null") {
            return Ok(None);
        }
        let info = self.info_of(object_id).await?;
        let node = self
            .on("DOM.describeNode", json!({"objectId": object_id}))
            .await?
            .get("node")
            .and_then(|n| n.get("backendNodeId"))
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Answer {
                method: "DOM.describeNode",
                reason: "no backendNodeId".into(),
            })?;
        let _ = self
            .on("Runtime.releaseObject", json!({"objectId": object_id}))
            .await;
        Ok(Some((node, info)))
    }
}

/// Answers dialogs: alerts and leave-page prompts are accepted, confirm
/// and prompt dismissed, because nobody allowed confirming.
async fn dialog_loop(conn: Arc<Connection>, session: String) {
    let mut events = conn.subscribe();
    loop {
        match events.recv().await {
            Ok(event) => {
                if event.session.as_deref() != Some(session.as_str())
                    || event.method != "Page.javascriptDialogOpening"
                {
                    continue;
                }
                let kind = event
                    .params
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let _ = conn
                    .call(
                        "Page.handleJavaScriptDialog",
                        json!({"accept": kind == "alert" || kind == "beforeunload"}),
                        Some(&session),
                    )
                    .await;
            }
            // Closed: the broadcast is dropped, so is this task.
            Err(broadcast::error::RecvError::Closed) => return,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
        }
    }
}

#[async_trait::async_trait]
impl Tab for Session {
    async fn read(&self) -> Result<Reading> {
        let answer = self
            .evaluate("__niles.read(12000)".to_string(), true)
            .await?;
        serde_json::from_value(answer.get("value").cloned().unwrap_or(Value::Null)).map_err(|_| {
            Error::Answer {
                method: "__niles.read",
                reason: "the reading wasn't the expected shape".into(),
            }
        })
    }

    async fn scroll(&self, way: Scroll) -> Result<()> {
        match way {
            Scroll::Down | Scroll::Up => {
                // A wheel scrolls a page whose content scrolls inside a
                // box too, which setting scrollTop would miss.
                let delta = if way == Scroll::Down { 765 } else { -765 };
                self.on(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": "mouseWheel",
                        "x": 640,
                        "y": 450,
                        "deltaX": 0,
                        "deltaY": delta
                    }),
                )
                .await?;
            }
            Scroll::Top | Scroll::Bottom => {
                let target = if way == Scroll::Top { "0" } else { "1e9" };
                self.evaluate(
                    format!("document.scrollingElement?.scrollTo(0, {target})"),
                    true,
                )
                .await?;
            }
        }
        // Lazy content may take a moment after the scroll.
        tokio::time::sleep(SCROLL_SETTLE).await;
        Ok(())
    }

    async fn address(&self) -> Result<String> {
        let answer = self.evaluate("location.href".to_string(), true).await?;
        answer
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Answer {
                method: "Runtime.evaluate",
                reason: "no address string".into(),
            })
    }

    async fn element(&self, reference: Reference) -> Result<Option<Element>> {
        let Some((node, info)) = self.find(reference).await? else {
            return Ok(None);
        };
        Ok(Some(info.element(node, reference)))
    }

    async fn again(&self, element: &Element) -> Result<Option<Element>> {
        // The document may have changed: resolving the node by id fails
        // when it has, and that means gone.
        let object = match self
            .on(
                "DOM.resolveNode",
                json!({
                    "backendNodeId": element.node.0,
                    "executionContextId": self.world().await?
                }),
            )
            .await
        {
            Ok(object) => object,
            Err(Error::Protocol { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let Some(object_id) = object
            .get("object")
            .and_then(|o| o.get("objectId"))
            .and_then(Value::as_str)
        else {
            return Ok(None);
        };
        let info = self.info_of(object_id).await?;
        if !info.connected {
            return Ok(None);
        }
        Ok(Some(info.element(element.node.0, element.reference)))
    }

    async fn aim(&self, element: &Element) -> Result<Aim> {
        // Scroll to it first: a covered verdict means a real overlay,
        // not an off-screen element.
        if self
            .on(
                "DOM.scrollIntoViewIfNeeded",
                json!({"backendNodeId": element.node.0}),
            )
            .await
            .is_err()
        {
            return Ok(Aim::Hidden);
        }
        let quads = match self
            .on(
                "DOM.getContentQuads",
                json!({"backendNodeId": element.node.0}),
            )
            .await
        {
            Ok(answer) => answer,
            Err(_) => return Ok(Aim::Hidden),
        };
        let Some(quads) = quads.get("quads").and_then(Value::as_array) else {
            return Ok(Aim::Hidden);
        };
        let viewport = self
            .on("Page.getLayoutMetrics", Value::Null)
            .await?
            .get("cssLayoutViewport")
            .cloned()
            .ok_or_else(|| Error::Answer {
                method: "Page.getLayoutMetrics",
                reason: "no cssLayoutViewport".into(),
            })?;
        let (vw, vh) = (
            viewport
                .get("clientWidth")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            viewport
                .get("clientHeight")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
        );
        let Some(outline) = visible_part(quads, vw, vh) else {
            return Ok(Aim::Hidden);
        };
        let at = outline.centre();
        let object = self
            .on(
                "DOM.resolveNode",
                json!({
                    "backendNodeId": element.node.0,
                    "executionContextId": self.world().await?
                }),
            )
            .await?;
        let Some(object_id) = object
            .get("object")
            .and_then(|o| o.get("objectId"))
            .and_then(Value::as_str)
        else {
            return Ok(Aim::Hidden);
        };
        // The isolated world's own hit test: the page can't see or
        // override this elementFromPoint.
        let hit = self
            .on(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": "function(x,y){ return __niles.hit(this,x,y) }",
                    "arguments": [{"value": at.x}, {"value": at.y}],
                    "returnByValue": true
                }),
            )
            .await?;
        match hit
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(Value::as_bool)
        {
            Some(true) => Ok(Aim::Clear { at, outline }),
            _ => Ok(Aim::Covered { outline }),
        }
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        let answer = self
            .on("Page.captureScreenshot", json!({"format": "png"}))
            .await?;
        let data = answer
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Answer {
                method: "Page.captureScreenshot",
                reason: "no data".into(),
            })?;
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data).map_err(|_| {
            Error::Answer {
                method: "Page.captureScreenshot",
                reason: "data wasn't base64".into(),
            }
        })
    }

    async fn click(&self, at: Point) -> Result<()> {
        // Subscribe first: the events the click causes must not be
        // missed between the dispatch and the wait.
        let rx = self.conn.subscribe();
        for (kind, buttons) in [
            ("mouseMoved", None),
            ("mousePressed", Some(1)),
            ("mouseReleased", Some(0)),
        ] {
            let mut event = json!({
                "type": kind,
                "x": at.x,
                "y": at.y,
                "button": "left"
            });
            if let Some(buttons) = buttons {
                event["buttons"] = json!(buttons);
                event["clickCount"] = json!(1);
            }
            self.on("Input.dispatchMouseEvent", event).await?;
        }
        self.settle(rx).await
    }

    async fn type_into(&self, element: &Element, text: &str) -> Result<()> {
        let rx = self.conn.subscribe();
        self.on("DOM.focus", json!({"backendNodeId": element.node.0}))
            .await?;
        let object_id = self.resolve(element).await?;
        // Replace what the field holds: select its content first, with
        // the contenteditable case falling back to a Range.
        self.call_on(
            &object_id,
            "function(){ if (this.select) { this.select(); } else if (this.isContentEditable) { const r = document.createRange(); r.selectNodeContents(this); const s = getSelection(); s.removeAllRanges(); s.addRange(r); } }",
            &[],
        )
        .await?;
        self.on("Input.insertText", json!({"text": text})).await?;
        self.call_on(
            &object_id,
            "function(){ this.dispatchEvent(new Event('change', {bubbles: true})); }",
            &[],
        )
        .await?;
        self.settle(rx).await
    }

    async fn choose(&self, element: &Element, value: &str) -> Result<()> {
        let rx = self.conn.subscribe();
        let object_id = self.resolve(element).await?;
        let changed = self
            .call_on(
                &object_id,
                "function(v){ this.value = v; this.dispatchEvent(new Event('input',{bubbles:true})); this.dispatchEvent(new Event('change',{bubbles:true})); return this.value === v; }",
                &[json!({ "value": value })],
            )
            .await?;
        if changed != Some(true) {
            return Err(Error::Answer {
                method: "Runtime.callFunctionOn",
                reason: "the option didn't take".into(),
            });
        }
        self.settle(rx).await
    }

    async fn submit(&self, element: &Element) -> Result<()> {
        let rx = self.conn.subscribe();
        let object_id = self.resolve(element).await?;
        let submitted = self
            .call_on(
                &object_id,
                "function(){ const form = this instanceof HTMLFormElement ? this : (this.form ?? this.closest('form')); if (!form) return false; const submitter = this instanceof HTMLInputElement && this.form === form && (this.type === 'submit' || this.type === 'image') ? this : undefined; form.requestSubmit(submitter); return true; }",
                &[],
            )
            .await?;
        if submitted != Some(true) {
            return Err(Error::Answer {
                method: "Runtime.callFunctionOn",
                reason: "no form to submit".into(),
            });
        }
        self.settle(rx).await
    }

    async fn open(&self, url: &Url) -> Result<()> {
        let mut rx = self.conn.subscribe();
        let answer = self
            .on("Page.navigate", json!({"url": url.as_str()}))
            .await?;
        if let Some(reason) = answer.get("errorText").and_then(Value::as_str) {
            return Err(Error::Navigation {
                reason: reason.to_string(),
            });
        }
        if answer.get("loaderId").is_none() {
            // A same-document navigation: nothing to wait for.
            return Ok(());
        }
        // Proceed on timeout: a slow page is still the page.
        wait_frame(
            &mut rx,
            &self.session,
            &self.frame,
            "Page.loadEventFired",
            OPEN_LOAD,
        )
        .await;
        Ok(())
    }

    async fn close(&self) {
        self.dialogs.abort();
        let _ = self
            .on(
                "Target.disposeBrowserContext",
                json!({"browserContextId": self.context}),
            )
            .await;
    }
}

/// The part of the element's quads that lies inside the viewport: the
/// outline the picture draws and the click's centre. `None` when the
/// element shows nothing on screen.
fn visible_part(quads: &[Value], vw: f64, vh: f64) -> Option<Rect> {
    for quad in quads {
        let Some(numbers) = quad.as_array() else {
            continue;
        };
        let nums: Vec<f64> = numbers.iter().filter_map(Value::as_f64).collect();
        if nums.len() < 8 {
            continue;
        }
        let xs = [nums[0], nums[2], nums[4], nums[6]];
        let ys = [nums[1], nums[3], nums[5], nums[7]];
        let (min_x, max_x) = (
            xs.iter().fold(f64::INFINITY, |a, &b| a.min(b)),
            xs.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b)),
        );
        let (min_y, max_y) = (
            ys.iter().fold(f64::INFINITY, |a, &b| a.min(b)),
            ys.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b)),
        );
        let x = min_x.max(0.0);
        let y = min_y.max(0.0);
        let width = max_x.min(vw) - x;
        let height = max_y.min(vh) - y;
        if width >= 1.0 && height >= 1.0 {
            return Some(Rect {
                x,
                y,
                width,
                height,
            });
        }
    }
    None
}

impl Session {
    /// The remote object id of the element, resolved in the isolated
    /// world.
    async fn resolve(&self, element: &Element) -> Result<String> {
        self.on(
            "DOM.resolveNode",
            json!({
                "backendNodeId": element.node.0,
                "executionContextId": self.world().await?
            }),
        )
        .await?
        .get("object")
        .and_then(|o| o.get("objectId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Answer {
            method: "DOM.resolveNode",
            reason: "no object id".into(),
        })
    }

    /// A `true`/`false` answer from a function run on a remote object.
    async fn call_on(
        &self,
        object_id: &str,
        function: &str,
        arguments: &[Value],
    ) -> Result<Option<bool>> {
        let answer = self
            .on(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": function,
                    "arguments": arguments,
                    "returnByValue": true
                }),
            )
            .await?;
        Ok(answer
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(Value::as_bool))
    }

    /// Waits out the page an action started: its load, then any popup
    /// a step opened in a new tab becomes the session's one page.
    async fn settle(&self, mut rx: broadcast::Receiver<Event>) -> Result<()> {
        let started = tokio::time::timeout(Duration::from_secs(1), self.wait_start(&mut rx))
            .await
            .is_ok();
        if started {
            // Either end of the load counts; a timeout proceeds.
            wait_loaded(&mut rx, &self.session, &self.frame, SETTLE_LOAD).await;
        }
        self.fold_popups().await
    }

    /// Waits for the main frame to start loading.
    async fn wait_start(&self, rx: &mut broadcast::Receiver<Event>) {
        loop {
            match rx.recv().await {
                Ok(event)
                    if event.method == "Page.frameStartedLoading"
                        && on_main_frame(&event, &self.session, &self.frame) =>
                {
                    return;
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    /// Pages a step opened in new tabs, closed; the last one with an
    /// https address replaces the session's page, so a checkout's new
    /// tab still lands where Niles can read it.
    async fn fold_popups(&self) -> Result<()> {
        let targets = self
            .on("Target.getTargets", Value::Null)
            .await?
            .get("targetInfos")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut landing: Option<Url> = None;
        for target in targets {
            let Some(id) = target.get("targetId").and_then(Value::as_str) else {
                continue;
            };
            let same_context = target.get("browserContextId").and_then(Value::as_str)
                == Some(self.context.as_str());
            let is_page = target.get("type").and_then(Value::as_str) == Some("page");
            if !same_context || !is_page || id == self.target {
                continue;
            }
            let url = self.popup_url(id).await;
            let _ = self.on("Target.closeTarget", json!({"targetId": id})).await;
            if let Ok(url) = Url::parse(&url)
                && url.scheme() == "https"
            {
                landing = Some(url);
            }
        }
        if let Some(url) = landing {
            self.open(&url).await?;
        }
        Ok(())
    }

    /// Where a popup points, once it has a real address: a fresh tab
    /// sits at `about:blank` for a moment.
    async fn popup_url(&self, target_id: &str) -> String {
        for _ in 0..10 {
            let info = self
                .on("Target.getTargetInfo", json!({"targetId": target_id}))
                .await;
            let url = info.ok().and_then(|i| {
                i.get("targetInfo")
                    .and_then(|t| t.get("url"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            match url {
                Some(url) if url != "about:blank" => return url,
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
        "about:blank".to_string()
    }
}

/// Whether `event` is on `session`'s main frame: the frame id must
/// match when the event carries one. `Page.loadEventFired` among
/// others has no frame id in newer protocols, and a session holds one
/// page, so no frame id is as good as the main frame's.
fn on_main_frame(event: &Event, session: &str, frame: &str) -> bool {
    if event.session.as_deref() != Some(session) {
        return false;
    }
    match event.params.get("frameId").and_then(Value::as_str) {
        Some(id) => id == frame,
        None => true,
    }
}

/// Waits up to `for_` for the main frame's load to finish, by its fired
/// or stopped event. Proceeds silently on timeout; a page may be slow.
async fn wait_loaded(
    rx: &mut broadcast::Receiver<Event>,
    session: &str,
    frame: &str,
    for_: Duration,
) {
    let deadline = tokio::time::Instant::now() + for_;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return;
        }
        let Ok(Ok(event)) = tokio::time::timeout(left, rx.recv()).await else {
            return;
        };
        let load_end =
            event.method == "Page.loadEventFired" || event.method == "Page.frameStoppedLoading";
        if load_end && on_main_frame(&event, session, frame) {
            return;
        }
    }
}

/// Waits up to `for_` for an event of `method` on this session's main
/// frame. Proceeds silently on timeout; a page may be slow.
async fn wait_frame(
    rx: &mut broadcast::Receiver<Event>,
    session: &str,
    frame: &str,
    method: &str,
    for_: Duration,
) {
    let deadline = tokio::time::Instant::now() + for_;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return;
        }
        let got = tokio::time::timeout(left, rx.recv()).await;
        let Ok(Ok(event)) = got else {
            return;
        };
        if event.method == method && on_main_frame(&event, session, frame) {
            return;
        }
    }
}
