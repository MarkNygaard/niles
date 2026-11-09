//! Filling a saved login or a code, and the guard that holds a
//! signed-in session's page on its login's origins.
//!
//! The fill script runs in an isolated world in the field's own frame,
//! so it checks the frame's `location.origin` as it sets each value:
//! the check and the set are one synchronous call, and a frame on
//! another origin — which runs in another process — never receives
//! anything. The guard fails main-frame navigations off the origins
//! before any request is sent, redirects and allowed clicks included;
//! see ARCHITECTURE.md's "Signing in".

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::cdp::{Connection, Event};
use crate::error::{Error, Result};
use crate::session::{PAGE_JS, Session};
use crate::tab::{Asks, Filled, on_sites};

/// How long a fill waits after setting the values before submitting:
/// frameworks update their state on the input event.
const SUBMIT_AFTER: Duration = Duration::from_millis(300);

/// One frame of the page: its CDP id and its security origin, `""`
/// when the frame doesn't report one.
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    pub(crate) id: String,
    pub(crate) origin: String,
}

impl Session {
    /// Every frame of the page, depth-first with the main frame first.
    pub(crate) async fn frames(&self) -> Result<Vec<Frame>> {
        fn walk(node: &Value, out: &mut Vec<Frame>) {
            if let Some(frame) = node.get("frame") {
                out.push(Frame {
                    id: frame
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    origin: frame
                        .get("securityOrigin")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                });
            }
            if let Some(children) = node.get("childFrames").and_then(Value::as_array) {
                for child in children {
                    walk(child, out);
                }
            }
        }
        let tree = self.on("Page.getFrameTree", Value::Null).await?;
        let mut out = Vec::new();
        // The tree's root hangs under `frameTree`; walking the raw
        // answer would visit nothing.
        if let Some(root) = tree.get("frameTree") {
            walk(root, &mut out);
        }
        Ok(out)
    }

    /// The isolated world's execution context for `frame`, `None` when
    /// the frame is unreachable (another process, or gone). The main
    /// frame's cached world is used, so `read()` sees `filled` marks.
    pub(crate) async fn world_for(&self, frame: &Frame) -> Result<Option<i64>> {
        if frame.id == self.frame {
            return self.world().await.map(Some);
        }
        match self
            .on(
                "Page.createIsolatedWorld",
                json!({
                    "frameId": frame.id,
                    "worldName": "niles",
                    "grantUniveralAccess": false
                }),
            )
            .await
        {
            Ok(answer) => {
                let context = answer
                    .get("executionContextId")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| Error::Answer {
                        method: "Page.createIsolatedWorld",
                        reason: "no executionContextId".into(),
                    })?;
                self.evaluate_in(context, PAGE_JS.to_string(), true).await?;
                Ok(Some(context))
            }
            Err(Error::Protocol { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// One `__niles` function call in a known world. Arguments may
    /// carry a saved value, so an exception's text — which could quote
    /// them — is never read: the reason is fixed.
    pub(crate) async fn call_in(
        &self,
        context: i64,
        function: &'static str,
        args: Vec<Value>,
    ) -> Result<Value> {
        let answer = self
            .on(
                "Runtime.callFunctionOn",
                json!({
                    "functionDeclaration":
                        format!("function(...a){{ return __niles.{function}(...a) }}"),
                    "executionContextId": context,
                    "arguments": args.into_iter().map(|v| json!({"value": v})).collect::<Vec<_>>(),
                    "returnByValue": true
                }),
            )
            .await?;
        if answer.get("exceptionDetails").is_some() {
            return Err(Error::Answer {
                method: "Runtime.callFunctionOn",
                reason: "the page script failed".into(),
            });
        }
        Ok(answer
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// The frames on `origins` with their worlds: the main frame plus
    /// every child frame whose origin is one of them and whose world
    /// Niles reaches. `None` when the top page isn't on the origins.
    pub(crate) async fn on_frames(&self, origins: &[String]) -> Result<Option<Vec<(Frame, i64)>>> {
        let main = self.world().await?;
        let top = self.call_in(main, "form", vec![json!(origins)]).await?;
        if top.get("on").and_then(Value::as_bool) != Some(true) {
            return Ok(None);
        }
        let mut out = vec![(
            Frame {
                id: self.frame.clone(),
                origin: String::new(),
            },
            main,
        )];
        for frame in self.frames().await?.iter().skip(1) {
            if !origins.contains(&frame.origin) {
                continue;
            }
            if let Some(context) = self.world_for(frame).await? {
                out.push((frame.clone(), context));
            }
        }
        Ok(Some(out))
    }

    /// What the page asks for to sign in, over its frames on `origins`.
    pub(super) async fn asks_in(&self, origins: &[String]) -> Result<Asks> {
        let main = self.world().await?;
        let top = self.call_in(main, "form", vec![json!(origins)]).await?;
        if top.get("captcha").and_then(Value::as_bool) == Some(true) {
            return Ok(Asks::Captcha);
        }
        if top.get("on").and_then(Value::as_bool) != Some(true) {
            return Ok(Asks::Nothing);
        }
        let Some(frames) = self.on_frames(origins).await? else {
            return Ok(Asks::Nothing);
        };
        let mut username = false;
        let mut code = false;
        for (_, context) in frames {
            let answer = match self.call_in(context, "form", vec![json!(origins)]).await {
                Ok(answer) => answer,
                Err(Error::Protocol { .. }) => continue,
                Err(e) => return Err(e),
            };
            if answer.get("password").and_then(Value::as_bool) == Some(true) {
                return Ok(Asks::Password {
                    username: answer.get("username").and_then(Value::as_bool) == Some(true),
                });
            }
            if answer.get("username").and_then(Value::as_bool) == Some(true) {
                username = true;
            }
            if answer.get("code").and_then(Value::as_bool) == Some(true) {
                code = true;
            }
        }
        if username {
            return Ok(Asks::Username);
        }
        if code {
            return Ok(Asks::Code);
        }
        Ok(Asks::Nothing)
    }

    /// Fill the login fields in the first frame on `origins` that has
    /// them, then submit. Each value is passed as a call argument and
    /// never appears in script source.
    pub(super) async fn fill_login_in(
        &self,
        origins: &[String],
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Filled> {
        let Some(frames) = self.on_frames(origins).await? else {
            return Ok(Filled::default());
        };
        for (_, context) in frames {
            let args = vec![
                json!(origins),
                username.map_or(Value::Null, |u| json!(u)),
                password.map_or(Value::Null, |p| json!(p)),
            ];
            let answer = match self.call_in(context, "fill", args).await {
                Ok(answer) => answer,
                Err(Error::Protocol { .. }) => continue,
                Err(e) => return Err(e),
            };
            let filled = Filled {
                username: answer.get("username").and_then(Value::as_bool) == Some(true),
                password: answer.get("password").and_then(Value::as_bool) == Some(true),
            };
            if filled.username || filled.password {
                self.submit_in(context, origins).await?;
                return Ok(filled);
            }
        }
        Ok(Filled::default())
    }

    /// Type the code into the first frame on `origins` whose page asks
    /// for one, then submit.
    pub(super) async fn fill_code_in(&self, origins: &[String], code: &str) -> Result<bool> {
        let Some(frames) = self.on_frames(origins).await? else {
            return Ok(false);
        };
        for (_, context) in frames {
            let answer = match self
                .call_in(context, "code", vec![json!(origins), json!(code)])
                .await
            {
                Ok(answer) => answer,
                Err(Error::Protocol { .. }) => continue,
                Err(e) => return Err(e),
            };
            if answer.as_bool() == Some(true) {
                self.submit_in(context, origins).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// How the fill is submitted: the form's submit button when there
    /// is one, else `requestSubmit`, else Enter on the filled field.
    async fn submit_in(&self, context: i64, origins: &[String]) -> Result<()> {
        // Frameworks update their state on the input event; give them
        // a beat before the submit.
        tokio::time::sleep(SUBMIT_AFTER).await;
        let rx = self.conn.subscribe();
        let answer = match self.call_in(context, "submit", vec![json!(origins)]).await {
            Ok(answer) => answer,
            // The page moved on: nothing to submit any more.
            Err(Error::Protocol { .. }) => return Ok(()),
            Err(e) => return Err(e),
        };
        match answer.as_str() {
            Some("enter") => {
                for kind in ["keyDown", "keyUp"] {
                    let mut event = json!({
                        "type": kind,
                        "key": "Enter",
                        "code": "Enter",
                        "windowsVirtualKeyCode": 13
                    });
                    if kind == "keyDown" {
                        event["text"] = json!("\r");
                    }
                    self.on("Input.dispatchKeyEvent", event).await?;
                }
                Ok(())
            }
            Some("no") => Ok(()),
            _ => self.settle(rx).await,
        }
    }

    /// From now on the page's own navigations stay on `origins`, and
    /// its history is forgotten so the back-forward cache can't bring
    /// an earlier page of another site back.
    pub(super) async fn stay_on_in(&self, origins: &[String]) -> Result<()> {
        self.on("Page.resetNavigationHistory", Value::Null).await?;
        // Subscribe before enabling, so no paused request is missed and
        // none hangs.
        let events = self.conn.subscribe();
        let task = tokio::spawn(guard_loop(
            Arc::clone(&self.conn),
            self.session.clone(),
            self.frame.clone(),
            origins.to_vec(),
            events,
        ));
        self.swap_guard(task);
        self.on(
            "Fetch.enable",
            json!({
                "patterns": [{
                    "urlPattern": "*",
                    "resourceType": "Document",
                    "requestStage": "Request"
                }]
            }),
        )
        .await?;
        Ok(())
    }
    /// Store the running guard, aborting the one a previous binding
    /// left. Synchronous so the std guard never lives across an await.
    fn swap_guard(&self, task: tokio::task::JoinHandle<()>) {
        let mut guard = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = guard.replace(task) {
            previous.abort();
        }
    }
}

/// Fails main-frame Document requests off `origins` with
/// `BlockedByClient`; subframes and subresources pass untouched.
/// `on_sites` blocks a URL that won't parse, and every paused request
/// is answered, so none hangs.
async fn guard_loop(
    conn: Arc<Connection>,
    session: String,
    frame: String,
    origins: Vec<String>,
    mut events: broadcast::Receiver<Event>,
) {
    loop {
        match events.recv().await {
            Ok(event) => {
                if event.session.as_deref() != Some(session.as_str())
                    || event.method != "Fetch.requestPaused"
                {
                    continue;
                }
                let params = &event.params;
                let main = params.get("frameId").and_then(Value::as_str) == Some(frame.as_str());
                let allowed = !main
                    || params
                        .get("request")
                        .and_then(|r| r.get("url"))
                        .and_then(Value::as_str)
                        .is_some_and(|url| on_sites(url, &origins));
                let request_id = params.get("requestId").cloned().unwrap_or(Value::Null);
                let (method, call) = if allowed {
                    ("Fetch.continueRequest", json!({ "requestId": request_id }))
                } else {
                    tracing::info!(
                        "[browser] a signed-in session's page was kept on its login's sites"
                    );
                    (
                        "Fetch.failRequest",
                        json!({ "requestId": request_id, "errorReason": "BlockedByClient" }),
                    )
                };
                let _ = conn.call(method, call, Some(&session)).await;
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_call_source_holds_only_the_functions_name() {
        let source = format!(
            "function(...a){{ return __niles.{function}(...a) }}",
            function = "fill"
        );
        assert_eq!(source, "function(...a){ return __niles.fill(...a) }");
        // The name is a &'static str chosen by this crate; a saved
        // value can only ever be an argument, never part of the source.
        assert!(!source.contains("password"));
    }
}
