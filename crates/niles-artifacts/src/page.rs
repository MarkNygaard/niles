//! An artifact's page: HTML the assistant writes once that draws the
//! artifact's data, for when the look matters more than a kind's own
//! view can give.
//!
//! The data stays the truth. A page never holds it: the app hands the
//! current content to the page in a sealed frame (no network, no access
//! to Niles, an opaque origin), and hands it again whenever it changes,
//! so a run Niles marks on a plan redraws the page without anyone
//! writing it again. Voice, cards and notifications read the data and
//! never the page. The threat model is in ARCHITECTURE.md's Security
//! notes, and the frame in `web/src/lib/sandboxedPage.ts`.
//!
//! What is checked here is what helps the model get a page that works:
//! a size, and nothing loaded from elsewhere, which the frame would
//! block anyway. The frame is the wall; this is the sign in front of it.

use crate::error::{Error, Result};

/// Room for a page with its own styles and script and a few drawings
/// in SVG; short enough that a runaway model is stopped, and that a
/// page is not most of what an artifact keeps.
pub const MAX_BYTES: usize = 200_000;

/// What the model is told a page is and how one is written: the
/// contract the app's frame keeps (`web/src/lib/sandboxedPage.ts`).
pub const HELP: &str = "A page is one self-contained HTML document, at most 200 KB, that draws \
    the artifact's data; the app shows it in place of the kind's own view, in a sealed frame \
    with no network and no access to Niles. Write it to read the data, never to contain it: \
    draw it in niles.onData(function (data) { … }), which is called with the data once the \
    page has loaded and again whenever the data changes (Niles marks a run done, the person \
    ticks something), so the page stays true without being written again; niles.data is the \
    data now. Draw every part of it from the data, so a change shows. Everything inline, in \
    <style> and <script>: no scripts, stylesheets, fonts or images from elsewhere and no \
    fetching (the frame blocks them, and a page that tries is refused); draw shapes in SVG. \
    Pictures are the ones the artifact keeps, handed to the page: niles.images is {id: url} \
    for each, niles.image(name) the url for a name in the data such as \"niles-image:12\" \
    (undefined for one it does not keep: draw without it), and niles.imageInfo[id] its \
    {alt, credit, source, width, height}; give each <img> its alt and show its credit, \
    small. A new picture is kept with artifact_add_image, after image_search, where it is \
    offered, finds it; a \
    picture's web address never shows in a page. \
    Use the app's colours as CSS variables, so it matches in light and dark: \
    var(--background), --foreground, --card, --card-foreground, --muted, --muted-foreground, \
    --accent, --accent-foreground, --primary, --primary-foreground, --border, --destructive, \
    --radius and --font-sans; niles.theme.dark says which it is. Mobile first: it is read on \
    a phone, about 400 px wide, and the frame grows to the page's height, so never fix the \
    height to the window. niles.today is the house's date, YYYY-MM-DD. A link \
    (<a href=\"https://…\"> or niles.open(url)) opens only after the person agrees. \
    niles.save(content, reason) saves new data for this artifact, the whole content as \
    artifact_update takes it and checked the same way, for a page where the person ticks or \
    changes something: only after their tap, never on load.";

fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid {
        kind: "page",
        reason: reason.into(),
    }
}

/// Why a page cannot be set on an artifact with nothing written yet.
pub fn nothing_to_draw() -> Error {
    invalid("nothing is written yet for it to draw: save the content first")
}

/// A page as it is kept: line endings made `\n`, control characters
/// other than tabs and newlines taken out, not empty, not too big, and
/// loading nothing from elsewhere.
pub fn page(raw: &str) -> Result<String> {
    let html: String = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let html = html.trim();
    if html.is_empty() {
        return Err(invalid("the page is empty"));
    }
    if html.len() > MAX_BYTES {
        return Err(invalid(format!(
            "it is {} KB, and a page is at most {} KB: draw more in script from the data, \
             and repeat less in the markup",
            html.len().div_ceil(1000),
            MAX_BYTES / 1000
        )));
    }
    if let Some(what) = from_elsewhere(html) {
        return Err(invalid(format!(
            "it {what}, which the frame blocks: a page has no network. Put scripts, styles \
             and fonts inline, draw pictures in SVG, and read the data from niles.data"
        )));
    }
    Ok(html.to_string())
}

/// What in `html` would reach outside the frame, said for the model.
fn from_elsewhere(html: &str) -> Option<&'static str> {
    let squeezed: String = html
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let tags = |name: &str| {
        let open = format!("<{name}");
        squeezed
            .match_indices(&open)
            .map(|(at, _)| {
                let rest = &squeezed[at..];
                &rest[..rest.find('>').unwrap_or(rest.len())]
            })
            .collect::<Vec<_>>()
    };
    if tags("script").iter().any(|t| t.contains("src=")) {
        return Some("loads a script from a file");
    }
    if !tags("link").is_empty() || squeezed.contains("@import") {
        return Some("links a stylesheet, font or icon from elsewhere");
    }
    if tags("meta").iter().any(|t| t.contains("http-equiv")) {
        return Some("has a <meta http-equiv>");
    }
    for frame in ["iframe", "frame", "object", "embed", "portal", "base"] {
        if !tags(frame).is_empty() {
            return Some("embeds another page or sets a base address");
        }
    }
    for call in [
        "fetch(",
        "xmlhttprequest",
        "websocket",
        "eventsource",
        "sendbeacon",
        "importscripts",
    ] {
        if squeezed.contains(call) {
            return Some("fetches from the network");
        }
    }
    for attr in ["src=", "srcset=", "poster=", "url("] {
        for (at, _) in squeezed.match_indices(attr) {
            if attr == "url(" && squeezed[..at].ends_with("new") {
                continue;
            }
            let value = squeezed[at + attr.len()..].trim_start_matches(['"', '\'']);
            if ["http:", "https:", "//"]
                .iter()
                .any(|s| value.starts_with(s))
            {
                return Some("loads an image, font or file from a web address");
            }
        }
    }
    None
}

/// The page in a job's final answer, when it answered with the page
/// instead of saving it: from the first tag to the last, code fence
/// and sentences around it left out.
pub fn from_answer(answer: &str) -> Option<String> {
    let (open, close) = (answer.find('<')?, answer.rfind('>')?);
    let html = answer.get(open..=close)?;
    html.contains("</").then(|| html.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<style>h1{color:var(--foreground)}</style><h1 id=t></h1>\
        <script>niles.onData(function(d){t.textContent=d.goal.race})</script>";

    #[test]
    fn a_page_is_kept_with_its_line_endings_made_plain() {
        let kept = page(&format!("  {PAGE}\r\n\u{7}")).unwrap();
        assert_eq!(kept, PAGE);
    }

    #[test]
    fn an_empty_page_is_refused() {
        assert!(page(" \n ").is_err());
    }

    #[test]
    fn a_page_over_the_cap_is_refused_with_its_size() {
        let big = format!("<p>{}</p>", "x".repeat(MAX_BYTES));
        let e = page(&big).unwrap_err().to_string();
        assert!(e.contains("200 KB"), "{e}");
        assert!(page(&format!("<p>{}</p>", "x".repeat(MAX_BYTES - 10))).is_ok());
    }

    #[test]
    fn anything_loaded_from_elsewhere_is_refused_with_what_to_do() {
        for html in [
            "<script src=\"https://cdn.example/chart.js\"></script>",
            "<script type=module src = '/app.js'></script>",
            "<link rel=\"stylesheet\" href=\"https://fonts.example/a.css\">",
            "<link rel=dns-prefetch href=//x.example>",
            "<style>@import 'https://x.example/a.css';</style>",
            "<img src=\"https://x.example/a.png\">",
            "<img srcset='//x.example/a.png 2x'>",
            "<div style=\"background: url( https://x.example/a.png )\"></div>",
            "<meta http-equiv=\"refresh\" content=\"0;url=https://x.example\">",
            "<iframe srcdoc=\"x\"></iframe>",
            "<base href=\"https://x.example/\">",
            "<script>fetch('/me/artifacts')</script>",
            "<script>new WebSocket('wss://x.example')</script>",
            "<script>navigator.sendBeacon('https://x.example', d)</script>",
        ] {
            let e = page(html).unwrap_err().to_string();
            assert!(e.contains("frame blocks"), "{html}: {e}");
        }
    }

    #[test]
    fn links_inline_pictures_and_urls_made_in_script_are_fine() {
        for html in [
            "<a href=\"https://www.jayfai.com/\">Jay Fai</a>",
            "<img src=\"data:image/png;base64,iVBORw0KGgo=\">",
            "<svg viewBox=\"0 0 10 10\"><circle r=\"4\"/></svg>",
            "<script>var u = new URL('https://maps.example/?q=' + q); niles.open(u)</script>",
            "<style>.x{background:url(data:image/svg+xml,%3Csvg%3E)}</style>",
        ] {
            assert!(page(html).is_ok(), "{html}");
        }
    }

    #[test]
    fn a_page_is_read_out_of_an_answer_with_its_fence() {
        let answer = format!("Here it is:\n\n```html\n{PAGE}\n```\n\nEnjoy.");
        assert_eq!(from_answer(&answer).as_deref(), Some(PAGE));
        assert_eq!(from_answer("Done."), None);
        assert_eq!(from_answer("I saved it <3 >"), None);
    }
}
