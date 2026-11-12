//! Real-Chromium tests for signing in, codes, CAPTCHAs, the guard and
//! profiles: the page-script origin checks only run in a real browser.
//! Ignored unless `NILES_TEST_CHROMIUM` names a DevTools address, such
//! as `127.0.0.1:9222`; CI's `browser` job runs them with
//! `cargo test -p niles-browser -- --ignored`.

#![allow(clippy::unwrap_used, reason = "test helpers")]

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};
use url::Url;

use base64::Engine;

use crate::browser::Browser;
use crate::proxy::Proxy;
use crate::session::Session;
use crate::tab::{Asks, Checkout, Filled, Reference, Tab};

/// Where the test Chromium's DevTools listens.
fn devtools() -> SocketAddr {
    let raw =
        std::env::var("NILES_TEST_CHROMIUM").expect("NILES_TEST_CHROMIUM, such as 127.0.0.1:9222");
    SocketAddr::from_str(&raw).expect("an address")
}

/// A session on a browser whose every connection goes through a fresh
/// proxy.
async fn session() -> Arc<Session> {
    let devtools = devtools();
    let proxy = Proxy::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let port = proxy.local_addr().unwrap().port();
    tokio::spawn(proxy.serve());
    let browser = Browser::connect(devtools, port)
        .await
        .expect("connect to Chromium");
    browser.open().await.expect("a session")
}

/// One page the fulfiller answers with.
struct Page {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

impl Page {
    fn html(body: &str) -> Page {
        Page {
            status: 200,
            headers: vec![],
            body: body.to_string(),
        }
    }
}

/// Answers every request to `pages` with its page, a 404 otherwise,
/// and records `(url, body)` of every POST. Must be called before any
/// navigation it should answer.
async fn serve(session: &Session, pages: Vec<(&str, Page)>) -> Arc<Mutex<Vec<(String, String)>>> {
    let pages: Vec<(String, Page)> = pages
        .into_iter()
        .map(|(want, page)| (want.to_string(), page))
        .collect();
    let mut events = session.conn.subscribe();
    let records: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let spawned = Arc::clone(&records);
    let by_session = session.session.clone();
    let conn = Arc::clone(&session.conn);
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event.session.as_deref() != Some(by_session.as_str())
                        || event.method != "Fetch.requestPaused"
                    {
                        continue;
                    }
                    let params = &event.params;
                    let request_id = params.get("requestId").cloned().unwrap_or(Value::Null);
                    let request = params.get("request").cloned().unwrap_or(Value::Null);
                    let url = request
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let method = request
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or("GET")
                        .to_string();
                    let post = request
                        .get("postData")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            let entries = request.get("postDataEntries")?.as_array()?;
                            let joined: String = entries
                                .iter()
                                .filter_map(|entry| {
                                    let bytes = entry.get("bytes")?.as_str()?;
                                    let decoded = base64::engine::general_purpose::STANDARD
                                        .decode(bytes)
                                        .ok()?;
                                    String::from_utf8(decoded).ok()
                                })
                                .collect::<Vec<_>>()
                                .join("");
                            Some(joined)
                        });
                    if method == "POST" {
                        spawned.lock().push((url.clone(), post.unwrap_or_default()));
                    }
                    let found = pages.iter().find(|(want, _)| *want == url);
                    let (status, headers, body) = match found {
                        Some((_, page)) => (page.status, &page.headers, page.body.clone()),
                        None => (404, &vec![], String::new()),
                    };
                    let mut answer_headers =
                        vec![json!({"name": "Content-Type", "value": "text/html; charset=utf-8"})];
                    for (name, value) in headers {
                        answer_headers.push(json!({"name": name, "value": value}));
                    }
                    let _ = conn
                        .call(
                            "Fetch.fulfillRequest",
                            json!({
                                "requestId": request_id,
                                "responseCode": status,
                                "responseHeaders": answer_headers,
                                "body": base64::engine::general_purpose::STANDARD.encode(body),
                            }),
                            Some(&by_session),
                        )
                        .await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
    session
        .on(
            "Fetch.enable",
            json!({"patterns": [{"urlPattern": "https://*", "requestStage": "Request"}]}),
        )
        .await
        .unwrap();
    records
}

/// The value of `selector` in the frame's world, "" when there is no
/// such element.
async fn value_of(session: &Session, frame_index: usize, selector: &str) -> String {
    let frames = session.frames().await.unwrap();
    let frame = frames.get(frame_index).expect("the frame").clone();
    let context = session
        .world_for(&frame)
        .await
        .unwrap()
        .expect("the frame's world");
    let answer = session
        .evaluate_in(
            context,
            format!("(() => {{ const el = document.querySelector({selector:?}); return el ? el.value : '' }})()"),
            true,
        )
        .await
        .unwrap();
    answer
        .get("value")
        .and_then(Value::as_str)
        .unwrap()
        .to_string()
}

/// A username-and-password form.
const LOGIN: &str = r#"<form method="post" action="/session">
  <input id="u" name="u" type="email" autocomplete="username">
  <input id="p" name="p" type="password" autocomplete="current-password">
  <button>Sign in</button>
</form>"#;

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_login_is_filled_and_sent_on_its_own_origin() {
    let session = session().await;
    let records = serve(
        &session,
        vec![
            ("https://shop.example/login", Page::html(LOGIN)),
            ("https://shop.example/session", Page::html("Welcome")),
        ],
    )
    .await;
    session
        .open(&Url::parse("https://shop.example/login").unwrap())
        .await
        .unwrap();
    let shop = vec!["https://shop.example".to_string()];

    assert_eq!(
        Tab::asks(&*session, &shop).await.unwrap(),
        Asks::Password { username: true }
    );
    let filled = session
        .fill_login(&shop, Some("u@example.com"), Some("p4ssw0rd"))
        .await
        .unwrap();
    assert_eq!(
        filled,
        Filled {
            username: true,
            password: true
        }
    );

    let posted = {
        let records = records.lock();
        records
            .iter()
            .find(|(url, _)| url == "https://shop.example/session")
            .expect("the form was posted")
            .clone()
    };
    assert!(posted.1.contains("u=u%40example.com"), "{}", posted.1);
    assert!(posted.1.contains("p=p4ssw0rd"), "{}", posted.1);
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn another_origin_gets_nothing() {
    let session = session().await;
    let records = serve(
        &session,
        vec![("https://evil.example/login", Page::html(LOGIN))],
    )
    .await;
    session
        .open(&Url::parse("https://evil.example/login").unwrap())
        .await
        .unwrap();
    let shop = vec!["https://shop.example".to_string()];

    assert_eq!(Tab::asks(&*session, &shop).await.unwrap(), Asks::Nothing);
    let filled = session
        .fill_login(&shop, Some("u@example.com"), Some("p4ssw0rd"))
        .await
        .unwrap();
    assert_eq!(filled, Filled::default());
    assert_eq!(value_of(&session, 0, "#u").await, "");
    assert_eq!(value_of(&session, 0, "#p").await, "");
    assert!(records.lock().is_empty(), "no POST was recorded");
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_frame_from_another_origin_gets_nothing() {
    let session = session().await;
    let records = serve(
        &session,
        vec![
            (
                "https://shop.example/framed",
                Page::html(r#"<iframe src="https://login.shop.example/form"></iframe>"#),
            ),
            ("https://login.shop.example/form", Page::html(LOGIN)),
        ],
    )
    .await;
    session
        .open(&Url::parse("https://shop.example/framed").unwrap())
        .await
        .unwrap();
    let shop = vec!["https://shop.example".to_string()];

    let filled = session
        .fill_login(&shop, Some("u@example.com"), Some("p4ssw0rd"))
        .await
        .unwrap();
    assert_eq!(filled, Filled::default());
    assert_eq!(value_of(&session, 1, "#u").await, "");
    assert_eq!(value_of(&session, 1, "#p").await, "");

    // Called directly, bypassing the frame filter, the in-page origin
    // check still refuses: the frame's origin isn't one of the login's.
    let frames = session.frames().await.unwrap();
    let context = session
        .world_for(&frames[1])
        .await
        .unwrap()
        .expect("the frame's world");
    let answer = session
        .call_in(
            context,
            "fill",
            vec![
                json!(vec!["https://shop.example"]),
                json!("u@example.com"),
                json!("p4ssw0rd"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(answer.get("username").and_then(Value::as_bool), Some(false));
    assert_eq!(answer.get("password").and_then(Value::as_bool), Some(false));
    assert!(records.lock().is_empty(), "no POST was recorded");
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_frame_on_the_logins_origin_is_filled_beside_a_foreign_one() {
    let session = session().await;
    serve(
        &session,
        vec![
            (
                "https://shop.example/both",
                Page::html(
                    // The submit is handled in the page, as an AJAX
                    // login's is: it must not navigate, or the filled
                    // values would be gone before they could be read.
                    r#"<form method="post" action="/session" onsubmit="return false">
                         <input id="u" name="u" type="email" autocomplete="username">
                         <input id="p" name="p" type="password" autocomplete="current-password">
                         <button>Sign in</button>
                       </form>
                       <iframe src="https://evil.shop.example/f"></iframe>"#,
                ),
            ),
            (
                "https://evil.shop.example/f",
                Page::html(r#"<input id="p" type="password" autocomplete="current-password">"#),
            ),
        ],
    )
    .await;
    session
        .open(&Url::parse("https://shop.example/both").unwrap())
        .await
        .unwrap();
    let shop = vec!["https://shop.example".to_string()];

    let filled = session
        .fill_login(&shop, Some("u@example.com"), Some("p4ssw0rd"))
        .await
        .unwrap();
    assert_eq!(
        filled,
        Filled {
            username: true,
            password: true
        }
    );
    assert_eq!(value_of(&session, 0, "#u").await, "u@example.com");
    assert_eq!(value_of(&session, 1, "#p").await, "");
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_redirect_off_the_origin_gets_nothing() {
    let session = session().await;
    let records = serve(
        &session,
        vec![
            (
                "https://shop.example/go",
                Page {
                    status: 302,
                    headers: vec![("Location", "https://evil.example/login".to_string())],
                    body: String::new(),
                },
            ),
            ("https://evil.example/login", Page::html(LOGIN)),
        ],
    )
    .await;
    session
        .open(&Url::parse("https://shop.example/go").unwrap())
        .await
        .unwrap();
    let address = session.address().await.unwrap();
    assert!(
        address.starts_with("https://evil.example"),
        "the redirect ran, off the login's origins: {address}"
    );
    let shop = vec!["https://shop.example".to_string()];

    let filled = session
        .fill_login(&shop, Some("u@example.com"), Some("p4ssw0rd"))
        .await
        .unwrap();
    assert_eq!(filled, Filled::default());
    assert!(records.lock().is_empty(), "no POST was recorded");
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_form_without_a_form_element_submits_with_enter() {
    let session = session().await;
    let records = serve(
        &session,
        vec![(
            "https://shop.example/enter",
            Page::html(
                r#"<input id="u" type="email" autocomplete="username">
                   <input id="p" type="password" autocomplete="current-password">
                   <script>
                   document.getElementById("p").addEventListener("keydown", (e) => {
                     if (e.key === "Enter") {
                       fetch("https://shop.example/api/login", {
                         method: "POST",
                         headers: {"content-type": "application/x-www-form-urlencoded"},
                         body: "u=" + encodeURIComponent(document.getElementById("u").value)
                             + "&p=" + encodeURIComponent(document.getElementById("p").value)
                       });
                     }
                   });
                   </script>"#,
            ),
        )],
    )
    .await;
    session
        .open(&Url::parse("https://shop.example/enter").unwrap())
        .await
        .unwrap();
    let shop = vec!["https://shop.example".to_string()];

    let filled = session
        .fill_login(&shop, Some("u@example.com"), Some("p4ssw0rd"))
        .await
        .unwrap();
    assert_eq!(
        filled,
        Filled {
            username: true,
            password: true
        }
    );
    // The Enter keydown on the password field posted both values.
    for _ in 0..20 {
        if records
            .lock()
            .iter()
            .any(|(url, _)| url == "https://shop.example/api/login")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let posted = {
        let records = records.lock();
        records
            .iter()
            .find(|(url, _)| url == "https://shop.example/api/login")
            .expect("the Enter posted the values")
            .clone()
    };
    assert!(posted.1.contains("u=u%40example.com"), "{}", posted.1);
    assert!(posted.1.contains("p=p4ssw0rd"), "{}", posted.1);
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_code_reaches_its_origin_only() {
    let session = session().await;
    let shop = vec!["https://shop.example".to_string()];
    // Six one-digit boxes take one digit each; like the single field,
    // the page is the login's own, or the origin check refuses it.
    let boxed: String = (0..6)
        .map(|i| format!(r#"<input name="d" maxlength="1" id="b{i}">"#))
        .collect();
    let records = serve(
        &session,
        vec![
            (
                "https://shop.example/code",
                Page::html(
                    r#"<form method="post" action="/verify">
                         <input autocomplete="one-time-code" name="otp">
                       </form>"#,
                ),
            ),
            (
                "https://evil.example/code",
                Page::html(
                    r#"<form method="post" action="/verify">
                         <input autocomplete="one-time-code" name="otp">
                       </form>"#,
                ),
            ),
            (
                "https://shop.example/boxed",
                Page::html(&format!(
                    r#"<form method="post" action="https://shop.example/verify2">{boxed}</form>"#
                )),
            ),
        ],
    )
    .await;

    session
        .open(&Url::parse("https://shop.example/code").unwrap())
        .await
        .unwrap();
    assert_eq!(Tab::asks(&*session, &shop).await.unwrap(), Asks::Code);
    assert!(session.fill_code(&shop, "493817").await.unwrap());
    let posted = {
        let records_now = records.lock();
        records_now
            .iter()
            .find(|(url, _)| url == "https://shop.example/verify")
            .expect("the code was posted")
            .clone()
    };
    assert!(posted.1.contains("otp=493817"), "{}", posted.1);

    // Six one-digit boxes take one digit each.
    session
        .open(&Url::parse("https://shop.example/boxed").unwrap())
        .await
        .unwrap();
    assert!(session.fill_code(&shop, "493817").await.unwrap());
    for _ in 0..20 {
        if records
            .lock()
            .iter()
            .any(|(url, _)| url == "https://shop.example/verify2")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let posted = {
        let records_now = records.lock();
        records_now
            .iter()
            .find(|(url, _)| url == "https://shop.example/verify2")
            .expect("the split code was posted")
            .clone()
    };
    assert!(posted.1.contains("d=4"), "{}", posted.1);
    assert!(posted.1.contains("d=7"), "{}", posted.1);

    // The same page on another origin gets nothing.
    session
        .open(&Url::parse("https://evil.example/code").unwrap())
        .await
        .unwrap();
    assert!(!session.fill_code(&shop, "493817").await.unwrap());
    Tab::close(&*session).await;
}

/// A data: URL carrying a small HTML page.
fn page(html: &str) -> Url {
    let encoded: String = html
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            byte => format!("%{byte:02X}"),
        })
        .collect();
    Url::parse(&format!("data:text/html,{encoded}")).unwrap()
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_captcha_is_seen_and_its_badge_is_not() {
    let session = session().await;
    let captcha = page(
        r#"<iframe src="https://www.google.com/recaptcha/api2/anchor?k=x&amp;size=normal" width="304" height="78"></iframe>"#,
    );
    session.open(&captcha).await.unwrap();
    let shop = vec!["https://shop.example".to_string()];
    assert!(session.read().await.unwrap().captcha);
    assert_eq!(Tab::asks(&*session, &shop).await.unwrap(), Asks::Captcha);

    // reCAPTCHA v3's badge doesn't count.
    let badge = page(
        r#"<div class="grecaptcha-badge"><iframe src="https://www.google.com/recaptcha/api2/anchor?k=x&amp;size=normal" width="304" height="78"></iframe></div>"#,
    );
    session.open(&badge).await.unwrap();
    assert!(!session.read().await.unwrap().captcha);
    assert_eq!(Tab::asks(&*session, &shop).await.unwrap(), Asks::Nothing);

    // Neither does an invisible widget.
    let invisible = page(
        r#"<iframe src="https://www.google.com/recaptcha/api2/anchor?k=x&amp;size=invisible" width="304" height="78"></iframe>"#,
    );
    session.open(&invisible).await.unwrap();
    assert!(!session.read().await.unwrap().captcha);
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn the_guard_keeps_the_page_on_its_origins() {
    let session = session().await;
    let linked = page(r#"<a id="x" href="https://evil.example/">x</a>"#);
    session.open(&linked).await.unwrap();
    let shop = vec!["https://shop.example".to_string()];

    session.stay_on(&shop).await.unwrap();
    match session
        .open(&Url::parse("https://evil.example/").unwrap())
        .await
    {
        Err(crate::error::Error::Navigation { reason }) => {
            assert!(
                reason.contains("ERR_BLOCKED_BY_CLIENT"),
                "the guard failed it: {reason}"
            );
        }
        other => panic!("expected a blocked navigation, got {other:?}"),
    }
    // The guard let a same-origin request through: it died at the proxy
    // instead, which proves it wasn't the guard.
    match session
        .open(&Url::parse("https://shop.example/").unwrap())
        .await
    {
        Err(crate::error::Error::Navigation { reason }) => {
            assert!(
                reason.contains("ERR_TUNNEL_CONNECTION_FAILED"),
                "the request reached the proxy: {reason}"
            );
        }
        other => panic!("expected the proxy's failure, got {other:?}"),
    }

    // The failed navigations above left Chromium's error page in the
    // frame; the page with the link is opened again, and the guard is
    // still bound to the session.
    session.open(&linked).await.unwrap();

    // An allowed link click on the page is failed before any request.
    let reading = session.read().await.unwrap();
    let number = reading
        .text
        .split('[')
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("the link's reference");
    let reference = Reference::parse(&json!(number.parse::<u32>().unwrap())).unwrap();
    let element = session.element(reference).await.unwrap().expect("the link");
    let at = match session.aim(&element).await.unwrap() {
        crate::tab::Aim::Clear { at, .. } => at,
        other => panic!("expected a clear aim, got {other:?}"),
    };
    session.click(at).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let address = session.address().await.unwrap();
    assert!(
        !address.starts_with("https://evil.example"),
        "the page stayed put: {address}"
    );
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_profile_keeps_the_logins_cookies_and_storage_only() {
    let signed_in = session().await;
    serve(
        &signed_in,
        vec![
            (
                "https://shop.example/",
                Page::html(
                    r#"<script>document.cookie = "sid=abc; Secure; Path=/"; localStorage.k = "v";</script>"#,
                ),
            ),
            (
                "https://other.example/",
                Page::html(r#"<script>document.cookie = "tracker=1; Secure; Path=/";</script>"#),
            ),
        ],
    )
    .await;
    let shop = vec!["https://shop.example".to_string()];
    // A profile is kept while the session's page is on the login's
    // origins — the guard holds it there — so the other site is
    // visited first and the login's last.
    signed_in
        .open(&Url::parse("https://other.example/").unwrap())
        .await
        .unwrap();
    signed_in
        .open(&Url::parse("https://shop.example/").unwrap())
        .await
        .unwrap();

    let kept = signed_in.profile(&shop).await.unwrap();
    let json: Value = serde_json::from_str(&kept).unwrap();
    let names: Vec<&str> = json["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(names.contains(&"sid"), "{names:?}");
    assert!(!names.contains(&"tracker"), "{names:?}");
    let storage = json["storage"].as_array().unwrap();
    assert_eq!(storage.len(), 1, "{storage:?}");
    assert_eq!(storage[0]["origin"], "https://shop.example");
    assert_eq!(storage[0]["items"], json!([["k", "v"]]));
    Tab::close(&*signed_in).await;

    // A fresh session restores the login's cookies and storage.
    let session = session().await;
    serve(
        &session,
        vec![("https://shop.example/", Page::html("plain"))],
    )
    .await;
    session
        .open(&Url::parse("https://shop.example/").unwrap())
        .await
        .unwrap();
    session.restore(&shop, &kept).await.unwrap();
    let answer = session
        .conn
        .call(
            "Storage.getCookies",
            json!({"browserContextId": session.context}),
            None,
        )
        .await
        .unwrap();
    let names: Vec<&str> = answer["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(names.contains(&"sid"), "{names:?}");
    let context = session.world().await.unwrap();
    let value = session
        .evaluate_in(context, "localStorage.getItem(\"k\")".to_string(), true)
        .await
        .unwrap();
    assert_eq!(value.get("value").and_then(Value::as_str), Some("v"));
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_card_form_is_seen_and_a_gift_card_is_not() {
    let session = session().await;
    for (html, want) in [
        (
            r#"<form><input autocomplete="cc-number" name="n"></form>"#,
            true,
        ),
        (r#"<label>Card number <input name="pan"></label>"#, true),
        (r#"<label>CVC <input name="x" size="4"></label>"#, true),
        (
            r#"<iframe title="Secure card number input frame" width="300" height="40"></iframe>"#,
            true,
        ),
        (
            r#"<iframe name="braintree-hosted-field-number" width="300" height="40"></iframe>"#,
            true,
        ),
        (
            r#"<label>Gift card number <input name="giftcard"></label>"#,
            false,
        ),
        (
            r#"<label>Security code <input autocomplete="one-time-code" name="otp"></label>"#,
            false,
        ),
        (
            r#"<div hidden><input autocomplete="cc-number"></div><button>Place order</button>"#,
            false,
        ),
    ] {
        session.open(&page(html)).await.unwrap();
        assert_eq!(session.read().await.unwrap().card_details, want, "{html}");
        assert_eq!(
            Tab::checkout(&*session).await.unwrap().card_details,
            want,
            "{html}"
        );
    }
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_checkout_shows_its_total_and_delivery_address() {
    let session = session().await;
    session
        .open(&page(
            r#"<title>Checkout</title><h1>Checkout</h1>
<section><h2>Delivery address</h2><p>Mark Nygaard<br>Street 1<br>2100 Copenhagen</p><button>Change</button></section>
<table><tr><td>Subtotal</td><td>400,00 kr.</td></tr><tr><td>Delivery</td><td>12,50 kr.</td></tr><tr><th>Total</th><td>412,50 kr.</td></tr></table>
<button>Place order</button>"#,
        ))
        .await
        .unwrap();
    let checkout = Tab::checkout(&*session).await.unwrap();
    assert!(checkout.at_checkout);
    assert_eq!(checkout.total.as_deref(), Some("412,50 kr."));
    assert_eq!(
        checkout.delivery.as_deref(),
        Some("Mark Nygaard, Street 1, 2100 Copenhagen")
    );
    assert!(!checkout.card_details);
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn an_address_in_fields_is_found_and_an_empty_basket_shows_nothing() {
    let session = session().await;
    session
        .open(&page(
            r#"<title>Your details</title>
<input autocomplete="shipping address-line1" value="Street 1">
<input autocomplete="shipping postal-code" value="2100">
<input autocomplete="shipping address-level2" value="Copenhagen">
<input autocomplete="billing address-line1" value="Elsewhere 9">"#,
        ))
        .await
        .unwrap();
    let checkout = Tab::checkout(&*session).await.unwrap();
    assert_eq!(
        checkout.delivery.as_deref(),
        Some("Street 1, 2100 Copenhagen")
    );
    assert_eq!(checkout.total, None);
    assert!(!checkout.at_checkout);
    assert!(!checkout.card_details);

    session
        .open(&page(
            r#"<title>Basket</title><p>Your basket is empty.</p>"#,
        ))
        .await
        .unwrap();
    assert_eq!(Tab::checkout(&*session).await.unwrap(), Checkout::default());
    Tab::close(&*session).await;
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_checkout_is_known_by_its_path() {
    let session = session().await;
    serve(
        &session,
        vec![(
            "https://shop.example/checkout",
            Page::html("<title>Shop</title><button>Fortsæt</button>"),
        )],
    )
    .await;
    session
        .open(&Url::parse("https://shop.example/checkout").unwrap())
        .await
        .unwrap();
    assert!(Tab::checkout(&*session).await.unwrap().at_checkout);
    Tab::close(&*session).await;
}
