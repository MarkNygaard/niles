//! `niles plugin check`: everything an install would refuse, said at
//! once, so an author fixes it in one pass.
//!
//! The manifest against `manifest.md`; the logo against the sanitiser's
//! rules; the component compiled and linked against the world its
//! manifest's `interface` names, `niles:integration/plugin@0.1` or `@0.2`; and its tools, asked for
//! without `init` in the same sandbox and limits a real run has, against
//! the rules in `tools.md`.

use crate::manifest::{Manifest, NILES_VERSION};
use crate::runtime::{House, Plugin};
use crate::store::MemoryStore;
use niles_core::{DeviceRegistry, EventBus};
use niles_tools::ToolDescriptor;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const MAX_LOGO: usize = 32 * 1024;

/// What `check` found.
#[derive(Debug, Default)]
pub struct Report {
    pub manifest: Option<Manifest>,
    /// Each a sentence; empty when the plugin would install.
    pub problems: Vec<String>,
    /// What passed, for the report to show its working.
    pub passed: Vec<String>,
    /// The interfaces the component imports.
    pub imports: Vec<String>,
    /// The tools as the model would see them.
    pub tools: Vec<ToolDescriptor>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Check a plugin: its manifest, and its component and logo if found.
pub async fn check(manifest: &str, component: Option<Vec<u8>>, logo: Option<Vec<u8>>) -> Report {
    let mut report = Report::default();
    let manifest = match Manifest::parse(manifest) {
        Ok(manifest) => manifest,
        Err(e) => {
            report.problems.push(e.to_string());
            return report;
        }
    };
    let problems = manifest.problems();
    if problems.is_empty() {
        report.passed.push(format!(
            "manifest: {} {} for niles:integration {} on Niles {} or newer (this is {NILES_VERSION})",
            manifest.id, manifest.version, manifest.interface, manifest.min_niles
        ));
    }
    report
        .problems
        .extend(problems.into_iter().map(|p| format!("manifest: {p}")));

    match (&manifest.logo, logo) {
        (None, _) => {}
        (Some(name), None) => report.problems.push(format!(
            "logo: the manifest names {name}, and it is not there"
        )),
        (Some(name), Some(svg)) => match check_logo(&svg) {
            Ok(()) => report
                .passed
                .push(format!("logo: {name} passes the sanitiser")),
            Err(why) => report.problems.push(format!("logo: {why}")),
        },
    }

    match component {
        None => report.problems.push(
            "component: no .wasm found; build it with cargo build --release --target wasm32-wasip2"
                .into(),
        ),
        Some(bytes) => check_component(&manifest, bytes, &mut report).await,
    }
    report.manifest = Some(manifest);
    report
}

async fn check_component(manifest: &Manifest, bytes: Vec<u8>, report: &mut Report) {
    let size = bytes.len();
    if size > crate::runtime::MAX_COMPONENT {
        report
            .problems
            .push(format!("component: {size} bytes; at most 64 MiB"));
        return;
    }
    let house = Arc::new(House::new(
        Arc::new(DeviceRegistry::new()),
        EventBus::default(),
        Arc::new(MemoryStore::new()),
    ));
    let plugin = Arc::new(Plugin::new(manifest.clone()));
    let checked = tokio::task::spawn_blocking(move || inspect(bytes, plugin, house)).await;
    let inspected = match checked {
        Ok(Ok(inspected)) => inspected,
        Ok(Err(why)) => {
            report.problems.push(format!("component: {why}"));
            return;
        }
        Err(e) => {
            report
                .problems
                .push(format!("component: the check failed: {e}"));
            return;
        }
    };
    report.imports = inspected.imports;
    report.passed.push(format!(
        "component: {} KiB, targets niles:integration/plugin@{} and exports every function",
        size / 1024,
        inspected.interface
    ));
    let tools = match inspected.tools {
        Ok(tools) => tools,
        Err(why) => {
            report.problems.push(format!("tools: {why}"));
            return;
        }
    };
    if !tools.is_empty() && !manifest.permissions.tools {
        report.problems.push(
            "tools: the component offers tools, and the manifest does not ask for tools = true"
                .into(),
        );
    }
    if tools.len() > crate::manifest::MAX_TOOLS {
        report.problems.push(format!(
            "tools: {} offered; at most {}",
            tools.len(),
            crate::manifest::MAX_TOOLS
        ));
    }
    let mut names = std::collections::HashSet::new();
    for tool in &tools {
        if !names.insert(tool.name.clone()) {
            report
                .problems
                .push(format!("tools: {:?} is offered twice", tool.name));
        }
        match crate::tools::describe(&manifest.id, tool) {
            Ok(descriptor) => report.tools.push(descriptor),
            Err(why) => report.problems.push(format!("tools: {why}")),
        }
    }
}

struct Inspected {
    imports: Vec<String>,
    /// The world it targets: `0.1` or `0.2`.
    interface: &'static str,
    tools: Result<Vec<crate::bindings::niles::integration::types::ToolDef>, String>,
}

fn inspect(bytes: Vec<u8>, plugin: Arc<Plugin>, house: Arc<House>) -> Result<Inspected, String> {
    let engine = crate::runtime::engine().map_err(|e| e.to_string())?;
    let linker = crate::runtime::linker(&engine).map_err(|e| e.to_string())?;
    let component = wasmtime::component::Component::new(&engine, &bytes)
        .map_err(|e| format!("is not a WebAssembly component: {}", e.root_cause()))?;
    let imports = component
        .component_type()
        .imports(&engine)
        .map(|(name, _)| name.to_string())
        .collect();
    let interface = crate::worker::interface_of(&component, &engine)?;
    let pre = crate::worker::prepare(&engine, &linker, &bytes, &plugin.manifest.interface)?;

    let ticking = engine.weak();
    std::thread::spawn(move || {
        while let Some(engine) = ticking.upgrade() {
            engine.increment_epoch();
            drop(engine);
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let state = crate::host::HostState::new(
        plugin,
        house,
        toml::Table::new(),
        crate::policy::HttpPolicy::default(),
        crate::policy::MqttPolicy::default(),
    );
    let mut store = wasmtime::Store::new(&engine, state);
    store.limiter(|state| &mut state.memory);
    let limit = crate::runtime::Limits::default().call;
    store.set_epoch_deadline(limit.as_millis() as u64 / 100);
    store.data_mut().deadline = Instant::now() + limit;
    let tools = pre
        .instantiate(&mut store)
        .and_then(|bindings| bindings.call_tools(&mut store))
        .map_err(|e| format!("tools() did not answer: {}", e.root_cause()));
    Ok(Inspected {
        imports,
        interface,
        tools,
    })
}

/// What the install's sanitiser accepts: an SVG on a 24×24 view box,
/// drawn with shapes, paths and groups, with geometry, fill, stroke and
/// transform attributes. No script, style, image, use, foreignObject,
/// links or external references, and at most 32 KiB.
pub fn check_logo(svg: &[u8]) -> Result<(), String> {
    sanitise_logo(svg).map(|_| ())
}

/// The logo as Niles keeps and serves it, or why it is refused.
///
/// An allowlist, applied twice. First the parsed document is checked
/// against it, and anything outside it refuses the logo with a reason
/// for its author, as `niles plugin check` reports it. Then the SVG is
/// written out again from the parse, element by allowed element and
/// attribute by allowed attribute, so what is stored is only ever what
/// the allowlist describes: comments, processing instructions, text and
/// whatever the parser tolerated are not carried over. roxmltree refuses
/// a DTD, so there are no entities to expand.
pub fn sanitise_logo(svg: &[u8]) -> Result<String, String> {
    const ELEMENTS: &[&str] = &[
        "svg", "g", "path", "circle", "ellipse", "rect", "line", "polyline", "polygon",
    ];
    const ATTRIBUTES: &[&str] = &[
        "viewBox",
        "width",
        "height",
        "version",
        "d",
        "cx",
        "cy",
        "r",
        "rx",
        "ry",
        "x",
        "y",
        "x1",
        "y1",
        "x2",
        "y2",
        "points",
        "fill",
        "fill-rule",
        "fill-opacity",
        "clip-rule",
        "stroke",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
        "stroke-miterlimit",
        "stroke-dasharray",
        "stroke-dashoffset",
        "stroke-opacity",
        "opacity",
        "transform",
    ];
    if svg.len() > MAX_LOGO {
        return Err(format!("{} bytes; at most 32 KiB", svg.len()));
    }
    let text = std::str::from_utf8(svg).map_err(|_| "it is not UTF-8 text".to_string())?;
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("it is not valid SVG: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err("the root element is not <svg>".into());
    }
    let view_box: Vec<&str> = root
        .attribute("viewBox")
        .unwrap_or_default()
        .split([' ', ','])
        .filter(|s| !s.is_empty())
        .collect();
    if view_box != ["0", "0", "24", "24"] {
        return Err("it needs viewBox=\"0 0 24 24\", like the built-in marks".into());
    }
    for node in doc.descendants().filter(|n| n.is_element()) {
        let tag = node.tag_name();
        if !ELEMENTS.contains(&tag.name())
            || tag
                .namespace()
                .is_some_and(|ns| ns != "http://www.w3.org/2000/svg")
        {
            return Err(format!("<{}> is not allowed", tag.name()));
        }
        for attribute in node.attributes() {
            if attribute.namespace().is_some() || !ATTRIBUTES.contains(&attribute.name()) {
                return Err(format!(
                    "the {} attribute on <{}> is not allowed",
                    attribute.name(),
                    tag.name()
                ));
            }
            let value = attribute.value().to_ascii_lowercase();
            if value.contains("url(") || value.contains("javascript:") {
                return Err(format!(
                    "the {} attribute refers to something outside the logo",
                    attribute.name()
                ));
            }
        }
    }
    let mut out = String::with_capacity(svg.len());
    write_element(root, &mut out);
    Ok(out)
}

fn write_element(node: roxmltree::Node, out: &mut String) {
    let name = node.tag_name().name();
    out.push('<');
    out.push_str(name);
    if name == "svg" {
        out.push_str(r#" xmlns="http://www.w3.org/2000/svg""#);
    }
    for attribute in node.attributes() {
        out.push(' ');
        out.push_str(attribute.name());
        out.push_str("=\"");
        for c in attribute.value().chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    let children: Vec<_> = node.children().filter(|n| n.is_element()).collect();
    if children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for child in children {
        write_element(child, out);
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worked_examples_logo_passes() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <path d="M13 2 4 14h7l-1 8 9-12h-7z" fill="#F5B400"/>
</svg>"##;
        assert_eq!(check_logo(svg), Ok(()));
    }

    #[test]
    fn script_style_links_and_odd_sizes_are_refused() {
        let wrap = |inner: &str| {
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">{inner}</svg>"#)
        };
        for bad in [
            "<script>alert(1)</script>",
            "<style>path{fill:red}</style>",
            r#"<image href="https://x.example/a.png"/>"#,
            r##"<use href="#a"/>"##,
            r#"<path d="M0 0" onclick="x()"/>"#,
            r#"<path d="M0 0" fill="url(https://x.example/#g)"/>"#,
            "<foreignObject/>",
        ] {
            assert!(check_logo(wrap(bad).as_bytes()).is_err(), "{bad}");
        }
        assert!(
            check_logo(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"/>"#)
                .is_err()
        );
        let big = wrap(&format!(r#"<path d="M0 0{}"/>"#, " L1 1".repeat(8000)));
        assert!(check_logo(big.as_bytes()).is_err());
    }

    #[test]
    fn the_kept_logo_is_rewritten_from_the_allowlist() {
        let svg = br##"<?xml version="1.0"?>
<!-- made by hand -->
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24">
  <?php echo 1; ?>
  <g fill="currentColor"><path d="M1 1h2" fill="#F5B400"/>stray text</g>
</svg>"##;
        assert_eq!(
            sanitise_logo(svg).unwrap(),
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24"><g fill="currentColor"><path d="M1 1h2" fill="#F5B400"/></g></svg>"##
        );
    }

    #[test]
    fn a_dtd_and_its_entities_are_refused() {
        let svg = br#"<!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa">]>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="&a;"/></svg>"#;
        assert!(sanitise_logo(svg).is_err());
    }

    #[test]
    fn values_are_escaped_when_written_out() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0" fill="a&amp;&quot;b"/></svg>"#;
        assert!(
            sanitise_logo(svg)
                .unwrap()
                .contains(r#"fill="a&amp;&quot;b""#)
        );
    }
}
