//! Integration tests against a real Chromium's DevTools.
//!
//! Ignored unless `NILES_TEST_CHROMIUM` names them, such as
//! `127.0.0.1:9222`. Start one with:
//! `chromium --headless=new --no-sandbox --remote-debugging-port=9222 \
//!   --user-data-dir=/tmp/niles-chromium-test about:blank`

#![allow(clippy::unwrap_used, reason = "test helpers")]

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use niles_browser::{Aim, Browser, Driver, Kind, Proxy, Reference, Scroll, Tab, picture};
use url::Url;

/// Where the test Chromium's DevTools listens.
fn devtools() -> SocketAddr {
    let raw =
        std::env::var("NILES_TEST_CHROMIUM").expect("NILES_TEST_CHROMIUM, such as 127.0.0.1:9222");
    SocketAddr::from_str(&raw).expect("an address")
}

/// A data: URL carrying a small HTML page. Strict percent-encoding: a
/// data URL decodes `%20` to a space, while a `+` stays a plus.
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

/// A connected browser, a session on it, and the proxy every connection
/// of the session goes through.
async fn session() -> (Arc<dyn Tab>, SocketAddr) {
    let devtools = devtools();
    let proxy = Proxy::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    tokio::spawn(proxy.serve());
    let browser = Browser::connect(devtools, proxy_addr.port())
        .await
        .expect("connect to Chromium");
    let tab = browser.session().await.expect("a session");
    (tab, proxy_addr)
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn reads_a_page_as_text_with_references() {
    let (tab, _) = session().await;
    tab.open(&page(
        r#"<!doctype html><title>The shop</title>
           <h1>Tools</h1>
           <p>All you need.</p>
           <a href="https://shop.example/buy">Buy now</a>
           <button id="go">Go</button>
           <label for="q">Search</label><input id="q" type="text" value="wrench">
           <label for="p">Password</label><input id="p" type="password" value="s3cret">
           <label for="c">Colour</label><select id="c">
             <option value="r">Red</option><option value="b" selected>Blue</option>
           </select>
           <p hidden>you can't see this</p>"#,
    ))
    .await
    .unwrap();

    let reading = tab.read().await.unwrap();
    assert_eq!(reading.title, "The shop");
    assert!(reading.text.contains("# Tools"), "{}", reading.text);
    assert!(
        reading.text.contains("link \"Buy now\" → "),
        "{}",
        reading.text
    );
    assert!(reading.text.contains("button \"Go\""), "{}", reading.text);
    assert!(
        reading.text.contains("textbox \"Search\" = \"wrench\""),
        "{}",
        reading.text
    );
    assert!(
        reading.text.contains("password \"Password\" (hidden)"),
        "{}",
        reading.text
    );
    assert!(!reading.text.contains("s3cret"), "no password value ever");
    assert!(reading.text.contains("Red"), "{}", reading.text);
    assert!(reading.text.contains("Blue"), "{}", reading.text);
    assert!(
        !reading.text.contains("can't see this"),
        "hidden text is absent"
    );

    // The references resolve to the elements they named.
    let reference = reading
        .text
        .split('[')
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .and_then(|n| Reference::parse(&serde_json::json!(n)))
        .expect("a reference on the page");
    let element = tab.element(reference).await.unwrap().expect("the element");
    assert!(!element.label.is_empty());
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn references_stay_the_same_across_reads() {
    let (tab, _) = session().await;
    let url =
        page(r#"<h1>Stable</h1><button id="one">One</button><a href="https://a.example/">A</a>"#);
    tab.open(&url).await.unwrap();
    let first = tab.read().await.unwrap();
    tab.scroll(Scroll::Down).await.unwrap();
    tab.scroll(Scroll::Top).await.unwrap();
    let second = tab.read().await.unwrap();
    assert_eq!(first.text, second.text);
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn a_read_starts_at_the_view_so_scrolling_matters() {
    let (tab, _) = session().await;
    tab.open(&page(
        r#"<h1>Top</h1><p>first</p><h1 id="later" style="margin-top:1500px">Later</h1>"#,
    ))
    .await
    .unwrap();

    let top = tab.read().await.unwrap();
    assert!(top.text.contains("# Top"), "{}", top.text);
    // Below the view is read too; only what is scrolled past is not.
    assert!(top.text.contains("Later"), "{}", top.text);
    assert!(!top.more_above, "at the top: {:?}", top);

    // Far enough that the first heading is wholly above the view; the
    // read starts where the view starts, and says there is more above.
    tab.scroll(Scroll::Down).await.unwrap();
    tab.scroll(Scroll::Down).await.unwrap();
    let below = tab.read().await.unwrap();
    assert!(!below.text.contains("# Top"), "{}", below.text);
    assert!(below.text.contains("Later"), "{}", below.text);
    assert!(below.more_above, "scrolled down: {:?}", below);
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn an_element_is_found_again_until_it_is_gone() {
    let (tab, _) = session().await;
    tab.open(&page(
        r#"<button id="soon">Soon gone</button>
           <script>setTimeout(() => document.getElementById('soon').remove(), 2_000)</script>"#,
    ))
    .await
    .unwrap();
    let reading = tab.read().await.unwrap();
    let number = reading
        .text
        .split('[')
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    let reference = Reference::parse(&serde_json::json!(number)).unwrap();
    let element = tab.element(reference).await.unwrap().expect("the element");
    let still = tab.again(&element).await.unwrap();
    assert!(still.is_some(), "still there just after the read");
    // Gone once the script's timer fires; a slow, loaded browser gets
    // room, and no pass waits the whole timeout out.
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if tab.again(&element).await.unwrap().is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the element goes away");
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn aim_sees_what_covers_an_element() {
    let (tab, _) = session().await;
    tab.open(&page(
        r#"<style>body{margin:0}button{display:block}
           #overlay{position:fixed;top:0;left:0;right:0;height:80px;background:#fff}
           #top{margin-top:120px}</style>
           <div id="overlay"></div>
           <button id="under">Under</button><button id="top">Top</button>"#,
    ))
    .await
    .unwrap();

    let reading = tab.read().await.unwrap();
    let number_of = |label: &str| {
        reading
            .text
            .split('[')
            .skip(1)
            .map(|part| part.split(']').next().unwrap())
            .zip(reading.text.split(']').skip(1))
            .find(|(_, rest)| rest.contains(label))
            .map(|(n, _)| Reference::parse(&serde_json::json!(n)).unwrap())
            .unwrap_or_else(|| panic!("no [{label}] on the page"))
    };
    let under = number_of("Under");
    let top = number_of("Top");
    let under = tab.element(under).await.unwrap().expect("under");
    let top = tab.element(top).await.unwrap().expect("top");
    assert!(matches!(
        tab.aim(&under).await.unwrap(),
        Aim::Covered { .. }
    ));
    assert!(
        matches!(tab.aim(&top).await.unwrap(), Aim::Clear { .. }),
        "the button below the overlay is clear to click"
    );
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn click_type_choose_and_submit_act_on_the_page() {
    let (tab, _) = session().await;
    tab.open(&page(
        r#"<button id="b" onclick="document.title='clicked'">Press</button>
           <label for="t">Words</label><input id="t" type="text">
           <label for="s">Pick</label><select id="s" onchange="document.title='chose:'+this.value">
             <option value="1">One</option><option value="2">Two</option>
           </select>
           <form onsubmit="event.preventDefault(); document.title='submitted:'+this.words.value">
             <label for="w">In form</label><input id="w" name="words" type="text">
             <button id="send" type="submit">Send</button>
           </form>"#,
    ))
    .await
    .unwrap();

    let reference = |reading: &niles_browser::Reading, name: &str| {
        reading
            .text
            .split('[')
            .skip(1)
            .map(|part| part.split(']').next().unwrap())
            .zip(reading.text.split(']').skip(1))
            .find(|(_, rest)| rest.contains(name))
            .map(|(n, _)| Reference::parse(&serde_json::json!(n)).unwrap())
            .unwrap_or_else(|| panic!("no [{name}] on the page"))
    };

    let reading = tab.read().await.unwrap();
    let press = tab
        .element(reference(&reading, "Press"))
        .await
        .unwrap()
        .unwrap();
    let aim = tab.aim(&press).await.unwrap();
    let at = match aim {
        Aim::Clear { at, .. } => at,
        other => panic!("expected a clear aim, got {other:?}"),
    };
    tab.click(at).await.unwrap();
    assert_eq!(tab.read().await.unwrap().title, "clicked");

    let reading = tab.read().await.unwrap();
    let words = tab
        .element(reference(&reading, "In form"))
        .await
        .unwrap()
        .unwrap();
    tab.type_into(&words, "hello niles").await.unwrap();
    let reading = tab.read().await.unwrap();
    assert!(
        reading.text.contains("= \"hello niles\""),
        "{}",
        reading.text
    );

    let reading = tab.read().await.unwrap();
    let pick = tab
        .element(reference(&reading, "Pick"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pick.kind, Kind::Select);
    tab.choose(&pick, "2").await.unwrap();
    assert_eq!(tab.read().await.unwrap().title, "chose:2");

    let reading = tab.read().await.unwrap();
    let send = tab
        .element(reference(&reading, "Send"))
        .await
        .unwrap()
        .unwrap();
    tab.submit(&send).await.unwrap();
    assert_eq!(tab.read().await.unwrap().title, "submitted:hello niles");
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn an_alert_does_not_hang_the_session() {
    let (tab, _) = session().await;
    tab.open(&page(
        r#"<button id="a" onclick="alert('hi there')">Alert me</button>"#,
    ))
    .await
    .unwrap();
    let reading = tab.read().await.unwrap();
    let number = reading
        .text
        .split('[')
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    let button = tab
        .element(Reference::parse(&serde_json::json!(number)).unwrap())
        .await
        .unwrap()
        .unwrap();
    let at = match tab.aim(&button).await.unwrap() {
        Aim::Clear { at, .. } => at,
        other => panic!("expected a clear aim, got {other:?}"),
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), tab.click(at))
        .await
        .expect("the alert didn't hang the click")
        .unwrap();
    // The session still reads after the alert was accepted.
    assert!(tab.read().await.is_ok());
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn the_screenshot_is_a_png_and_the_picture_a_jpeg() {
    let (tab, _) = session().await;
    tab.open(&page(r#"<h1>Shot</h1><button id="b">Here</button>"#))
        .await
        .unwrap();
    let png = tab.screenshot().await.unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

    let reading = tab.read().await.unwrap();
    let number = reading
        .text
        .split('[')
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    let button = tab
        .element(Reference::parse(&serde_json::json!(number)).unwrap())
        .await
        .unwrap()
        .unwrap();
    let outline = match tab.aim(&button).await.unwrap() {
        Aim::Clear { outline, .. } => outline,
        other => panic!("expected a clear aim, got {other:?}"),
    };
    let jpeg = picture(&png, Some(outline)).unwrap();
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
}

#[tokio::test]
#[ignore = "needs a Chromium's DevTools at NILES_TEST_CHROMIUM, such as 127.0.0.1:9222"]
async fn every_connection_goes_through_niles_proxy() {
    let (tab, _) = session().await;
    // These are loopback or private names and IPs: the proxy must
    // refuse them, and `<-loopback>` must make even loopback go through
    // the proxy. If Chromium answers ERR_CONNECTION_REFUSED instead,
    // the bypass list didn't take: stop and report that, don't weaken
    // this test.
    for address in [
        "https://127.0.0.1/",
        "https://localhost/",
        "https://10.0.0.1/",
    ] {
        let err = tab
            .open(&Url::parse(address).unwrap())
            .await
            .expect_err("the proxy refuses loopback and private addresses");
        let reason = err.to_string();
        assert!(
            reason.contains("ERR_TUNNEL_CONNECTION_FAILED"),
            "{address}: the proxy's refusal was expected, got: {reason}"
        );
    }
}
