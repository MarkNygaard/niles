//! A plugin's page or widget, checked and turned into what the app gets.
//!
//! The interface sends a tree as a flat list (WIT has no recursive
//! types), so before anything is drawn Niles makes sure it really is a
//! tree: node 0 is the root, every other node has exactly one parent
//! with a smaller index, only `stack`, `row` and `card` have children,
//! at most 500 nodes, 8 deep and 256 KiB. The app then receives the same
//! flat list as JSON, in [`Surface`]'s shape:
//!
//! ```json
//! {"nodes": [
//!   {"type": "stack", "children": [1, 2]},
//!   {"type": "reading", "label": "Now", "value": "1.23", "unit": "kr/kWh", "children": []},
//!   {"type": "button", "id": "refresh", "label": "Refresh now", "style": "secondary", "children": []}
//! ]}
//! ```

use crate::bindings::niles::integration::ui as wit;
use crate::policy::HttpPolicy;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_NODES: usize = 500;
pub const MAX_DEPTH: usize = 8;
pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_DATA_IMAGE: usize = 64 * 1024;
const MAX_SERIES: usize = 4;
const MAX_POINTS: usize = 200;

/// A rendered surface, as the app receives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    #[serde(flatten)]
    pub kind: Kind,
    pub children: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    Stack,
    Row,
    Card {
        title: Option<String>,
    },
    Heading {
        text: String,
    },
    Text {
        value: String,
        muted: bool,
    },
    Reading {
        label: String,
        value: String,
        unit: Option<String>,
    },
    Badge {
        label: String,
        tone: Tone,
    },
    Items {
        items: Vec<Item>,
    },
    Button {
        id: String,
        label: String,
        style: ButtonStyle,
    },
    Switch {
        id: String,
        label: String,
        value: bool,
    },
    Slider {
        id: String,
        label: String,
        min: f64,
        max: f64,
        step: f64,
        value: f64,
        unit: Option<String>,
    },
    Choice {
        id: String,
        label: String,
        options: Vec<ChoiceOption>,
        value: Option<String>,
    },
    TextField {
        id: String,
        label: String,
        placeholder: Option<String>,
        value: Option<String>,
    },
    Chart {
        kind: ChartKind,
        labels: Vec<String>,
        series: Vec<Series>,
        unit: Option<String>,
    },
    Image {
        src: String,
        alt: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Neutral,
    Good,
    Warning,
    Bad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Line,
    Bar,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub title: String,
    pub detail: Option<String>,
    pub trailing: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub name: String,
    pub values: Vec<f64>,
}

/// What somebody did to a control, as the app posts it:
/// `{"type":"pressed"}`, `{"type":"toggled","value":true}`,
/// `{"type":"number","value":21.5}`, `{"type":"chosen","value":"DK2"}`,
/// `{"type":"text","value":"hello"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ActionValue {
    Pressed,
    Toggled(bool),
    Number(f64),
    Chosen(String),
    Text(String),
}

/// The plugin's answer to an action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionOutcome {
    pub toast: Option<String>,
    pub rerender: bool,
}

/// Where a surface is drawn and for whom: the room whose card a widget
/// is on, and the signed-in person's slug, never more about them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Context {
    pub room: Option<String>,
    pub person: Option<String>,
}

impl From<ActionValue> for wit::ActionValue {
    fn from(value: ActionValue) -> Self {
        match value {
            ActionValue::Pressed => wit::ActionValue::Pressed,
            ActionValue::Toggled(on) => wit::ActionValue::Toggled(on),
            ActionValue::Number(n) => wit::ActionValue::Number(n),
            ActionValue::Chosen(v) => wit::ActionValue::Chosen(v),
            ActionValue::Text(t) => wit::ActionValue::Text(t),
        }
    }
}

impl From<&Context> for wit::RenderContext {
    fn from(context: &Context) -> Self {
        wit::RenderContext {
            room: context.room.clone(),
            person: context.person.clone(),
        }
    }
}

impl From<wit::ActionResult> for ActionOutcome {
    fn from(result: wit::ActionResult) -> Self {
        ActionOutcome {
            toast: result.toast,
            rerender: result.rerender,
        }
    }
}

impl Surface {
    /// The tree as indented text, one node a line, as `niles plugin dev`
    /// prints it.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.write(0, 0, &mut out);
        out
    }

    fn write(&self, index: usize, depth: usize, out: &mut String) {
        let Some(node) = self.nodes.get(index) else {
            return;
        };
        let indent = "  ".repeat(depth);
        let (name, detail) = describe(&node.kind);
        if detail.is_empty() {
            out.push_str(&format!("{indent}{name}\n"));
        } else {
            // Details line up in a column, as far as the nesting allows.
            let width = 13usize.saturating_sub(indent.len()).max(name.len() + 1);
            out.push_str(&format!("{indent}{name:<width$}{detail}\n"));
        }
        if let Kind::Items { items } = &node.kind {
            for item in items {
                let parts: Vec<&str> = [Some(item.title.as_str()), item.detail.as_deref(), item.trailing.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                out.push_str(&format!("{indent}  - {}\n", parts.join(" · ")));
            }
        }
        for &child in &node.children {
            self.write(child as usize, depth + 1, out);
        }
    }
}

fn describe(kind: &Kind) -> (&'static str, String) {
    let unit = |u: &Option<String>| u.as_deref().map(|u| format!(" {u}")).unwrap_or_default();
    match kind {
        Kind::Stack => ("Stack", String::new()),
        Kind::Row => ("Row", String::new()),
        Kind::Card { title } => (
            "Card",
            title.as_ref().map(|t| format!("{t:?}")).unwrap_or_default(),
        ),
        Kind::Heading { text } => ("Heading", text.clone()),
        Kind::Text { value, muted } => (
            "Text",
            format!("{value}{}", if *muted { " (muted)" } else { "" }),
        ),
        Kind::Reading { label, value, unit: u } => ("Reading", format!("{label}: {value}{}", unit(u))),
        Kind::Badge { label, tone } => ("Badge", format!("{label} ({})", format!("{tone:?}").to_lowercase())),
        Kind::Items { items } => ("Items", format!("{} rows", items.len())),
        Kind::Button { id, label, style } => (
            "Button",
            format!("{id} {label:?} ({})", format!("{style:?}").to_lowercase()),
        ),
        Kind::Switch { id, label, value } => (
            "Switch",
            format!("{id} {label:?} {}", if *value { "on" } else { "off" }),
        ),
        Kind::Slider { id, label, min, max, step, value, unit: u } => (
            "Slider",
            format!("{id} {label:?} {value}{} ({min}–{max} by {step})", unit(u)),
        ),
        Kind::Choice { id, label, options, value } => (
            "Choice",
            format!(
                "{id} {label:?} = {} of {}",
                value.as_deref().unwrap_or("-"),
                options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>().join(", ")
            ),
        ),
        Kind::TextField { id, label, value, .. } => (
            "TextField",
            format!("{id} {label:?} = {:?}", value.as_deref().unwrap_or_default()),
        ),
        Kind::Chart { kind, labels, series, unit: u } => (
            "Chart",
            format!(
                "{}, {} points{}{}",
                format!("{kind:?}").to_lowercase(),
                labels.len(),
                if series.len() > 1 { format!(", {} series", series.len()) } else { String::new() },
                u.as_deref().map(|u| format!(", {u}")).unwrap_or_default()
            ),
        ),
        Kind::Image { alt, .. } => ("Image", alt.clone()),
    }
}

/// Check `tree` against the rules and convert it, or say what is wrong.
/// `http` decides which hosts an image may come from.
pub(crate) fn surface(tree: wit::UiTree, http: &HttpPolicy) -> Result<Surface, String> {
    let count = tree.nodes.len();
    if count == 0 {
        return Err("the tree has no nodes".into());
    }
    if count > MAX_NODES {
        return Err(format!("the tree has {count} nodes; at most {MAX_NODES}"));
    }
    let mut depth = vec![0usize; count];
    let mut has_parent = vec![false; count];
    depth[0] = 1;
    let mut controls = HashSet::new();
    let mut nodes = Vec::with_capacity(count);
    for (index, node) in tree.nodes.into_iter().enumerate() {
        if index > 0 && !has_parent[index] {
            return Err(format!("node {index} is not a child of any node before it"));
        }
        let takes_children = matches!(
            node.kind,
            wit::NodeKind::Stack | wit::NodeKind::Row | wit::NodeKind::Card(_)
        );
        if !node.children.is_empty() && !takes_children {
            return Err(format!(
                "node {index} has children, and only stack, row and card may"
            ));
        }
        for &child in &node.children {
            let child = child as usize;
            if child <= index || child >= count {
                return Err(format!(
                    "node {index} lists child {child}; a child comes after its parent, within the list"
                ));
            }
            if std::mem::replace(&mut has_parent[child], true) {
                return Err(format!("node {child} has more than one parent"));
            }
            depth[child] = depth[index] + 1;
            if depth[child] > MAX_DEPTH {
                return Err(format!("the tree is nested more than {MAX_DEPTH} deep"));
            }
        }
        let kind = convert(node.kind, http)?;
        if let Some(id) = control_id(&kind) {
            if id.is_empty() {
                return Err(format!("node {index} is a control with no id"));
            }
            if !controls.insert(id.to_string()) {
                return Err(format!("two controls have the id {id:?}"));
            }
        }
        nodes.push(Node {
            kind,
            children: node.children,
        });
    }
    let surface = Surface { nodes };
    let size = serde_json::to_vec(&surface).map(|v| v.len()).unwrap_or(usize::MAX);
    if size > MAX_BYTES {
        return Err(format!("the tree is {size} bytes; at most {MAX_BYTES}"));
    }
    Ok(surface)
}

fn control_id(kind: &Kind) -> Option<&str> {
    match kind {
        Kind::Button { id, .. }
        | Kind::Switch { id, .. }
        | Kind::Slider { id, .. }
        | Kind::Choice { id, .. }
        | Kind::TextField { id, .. } => Some(id),
        _ => None,
    }
}

fn convert(kind: wit::NodeKind, http: &HttpPolicy) -> Result<Kind, String> {
    Ok(match kind {
        wit::NodeKind::Stack => Kind::Stack,
        wit::NodeKind::Row => Kind::Row,
        wit::NodeKind::Card(title) => Kind::Card { title },
        wit::NodeKind::Heading(text) => Kind::Heading { text },
        wit::NodeKind::Text(t) => Kind::Text {
            value: t.value,
            muted: t.muted,
        },
        wit::NodeKind::Reading(r) => Kind::Reading {
            label: r.label,
            value: r.value,
            unit: r.unit,
        },
        wit::NodeKind::Badge(b) => Kind::Badge {
            label: b.label,
            tone: match b.tone {
                wit::Tone::Neutral => Tone::Neutral,
                wit::Tone::Good => Tone::Good,
                wit::Tone::Warning => Tone::Warning,
                wit::Tone::Bad => Tone::Bad,
            },
        },
        wit::NodeKind::Items(items) => Kind::Items {
            items: items
                .into_iter()
                .map(|i| Item {
                    title: i.title,
                    detail: i.detail,
                    trailing: i.trailing,
                })
                .collect(),
        },
        wit::NodeKind::Button(b) => Kind::Button {
            id: b.id,
            label: b.label,
            style: match b.style {
                wit::ButtonStyle::Primary => ButtonStyle::Primary,
                wit::ButtonStyle::Secondary => ButtonStyle::Secondary,
                wit::ButtonStyle::Destructive => ButtonStyle::Destructive,
            },
        },
        wit::NodeKind::Switch(s) => Kind::Switch {
            id: s.id,
            label: s.label,
            value: s.value,
        },
        wit::NodeKind::Slider(s) => {
            let finite = [s.min, s.max, s.step, s.value].iter().all(|n| n.is_finite());
            if !finite || s.min >= s.max || s.step <= 0.0 {
                return Err(format!(
                    "slider {:?} needs min below max and a step above 0",
                    s.id
                ));
            }
            Kind::Slider {
                id: s.id,
                label: s.label,
                min: s.min,
                max: s.max,
                step: s.step,
                value: s.value,
                unit: s.unit,
            }
        }
        wit::NodeKind::Choice(c) => Kind::Choice {
            id: c.id,
            label: c.label,
            options: c
                .options
                .into_iter()
                .map(|o| ChoiceOption {
                    value: o.value,
                    label: o.label,
                })
                .collect(),
            value: c.value,
        },
        wit::NodeKind::TextField(t) => Kind::TextField {
            id: t.id,
            label: t.label,
            placeholder: t.placeholder,
            value: t.value,
        },
        wit::NodeKind::Chart(c) => {
            if c.series.len() > MAX_SERIES {
                return Err(format!("a chart has at most {MAX_SERIES} series"));
            }
            if c.labels.len() > MAX_POINTS {
                return Err(format!("a chart has at most {MAX_POINTS} points"));
            }
            if let Some(s) = c.series.iter().find(|s| s.values.len() != c.labels.len()) {
                return Err(format!(
                    "chart series {:?} has {} values for {} labels; one value per label",
                    s.name,
                    s.values.len(),
                    c.labels.len()
                ));
            }
            Kind::Chart {
                kind: match c.kind {
                    wit::ChartKind::Line => ChartKind::Line,
                    wit::ChartKind::Bar => ChartKind::Bar,
                },
                labels: c.labels,
                series: c
                    .series
                    .into_iter()
                    .map(|s| Series {
                        name: s.name,
                        values: s.values,
                    })
                    .collect(),
                unit: c.unit,
            }
        }
        wit::NodeKind::Image(i) => {
            check_image(&i.src, http)?;
            Kind::Image {
                src: i.src,
                alt: i.alt,
            }
        }
    })
}

/// An image is a small `data:` URI or an https URL on a declared host,
/// and never SVG, which can carry script.
fn check_image(src: &str, http: &HttpPolicy) -> Result<(), String> {
    const TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];
    if let Some(data) = src.strip_prefix("data:") {
        let mime = data.split([';', ',']).next().unwrap_or_default();
        if !TYPES.contains(&mime.to_ascii_lowercase().as_str()) {
            return Err(format!("an image is PNG, JPEG, WebP or GIF, not {mime:?}"));
        }
        if src.len() > MAX_DATA_IMAGE {
            return Err(format!(
                "a data: image is at most {} KiB",
                MAX_DATA_IMAGE / 1024
            ));
        }
        return Ok(());
    }
    let url: reqwest::Url = src
        .parse()
        .map_err(|_| format!("image {src:?} is neither a data: URI nor a URL"))?;
    if url.scheme() != "https" {
        return Err(format!("image {src:?} must be https"));
    }
    if url.path().to_ascii_lowercase().ends_with(".svg") {
        return Err("SVG images are not allowed".into());
    }
    match url.host_str() {
        Some(host) if http.declares_host(host) => Ok(()),
        _ => Err(format!(
            "image {src:?} is not on a host the http permission declares"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Permissions;

    fn node(kind: wit::NodeKind, children: &[u32]) -> wit::Node {
        wit::Node {
            kind,
            children: children.to_vec(),
        }
    }

    fn heading(text: &str) -> wit::NodeKind {
        wit::NodeKind::Heading(text.into())
    }

    fn button(id: &str) -> wit::NodeKind {
        wit::NodeKind::Button(wit::Button {
            id: id.into(),
            label: "Go".into(),
            style: wit::ButtonStyle::Primary,
        })
    }

    fn check(nodes: Vec<wit::Node>) -> Result<Surface, String> {
        let http = HttpPolicy::new(
            &Permissions {
                http: vec!["img.example.com".into()],
                ..Default::default()
            },
            |_| None,
        );
        surface(wit::UiTree { nodes }, &http)
    }

    #[test]
    fn a_good_tree_becomes_json_in_the_documented_shape() {
        let s = check(vec![
            node(wit::NodeKind::Stack, &[1, 2]),
            node(heading("Hello"), &[]),
            node(button("go"), &[]),
        ])
        .unwrap();
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["nodes"][0]["type"], "stack");
        assert_eq!(json["nodes"][0]["children"], serde_json::json!([1, 2]));
        assert_eq!(json["nodes"][1]["type"], "heading");
        assert_eq!(json["nodes"][1]["text"], "Hello");
        assert_eq!(json["nodes"][2]["style"], "primary");
    }

    #[test]
    fn a_child_must_come_after_its_parent() {
        let err = check(vec![node(wit::NodeKind::Stack, &[0])]).unwrap_err();
        assert!(err.contains("after its parent"), "{err}");
        let err = check(vec![
            node(wit::NodeKind::Stack, &[2]),
            node(wit::NodeKind::Row, &[1]),
            node(heading("x"), &[]),
        ])
        .unwrap_err();
        assert!(err.contains("not a child"), "{err}");
    }

    #[test]
    fn a_node_has_one_parent() {
        let err = check(vec![
            node(wit::NodeKind::Stack, &[1, 2]),
            node(wit::NodeKind::Row, &[2]),
            node(heading("x"), &[]),
        ])
        .unwrap_err();
        assert!(err.contains("more than one parent"), "{err}");
    }

    #[test]
    fn only_layout_nodes_have_children() {
        let err = check(vec![node(heading("x"), &[1]), node(heading("y"), &[])]).unwrap_err();
        assert!(err.contains("only stack, row and card"), "{err}");
    }

    #[test]
    fn a_child_index_past_the_end_is_refused() {
        assert!(check(vec![node(wit::NodeKind::Stack, &[5])]).is_err());
    }

    #[test]
    fn the_tree_is_at_most_eight_deep() {
        let deep = |levels: u32| {
            let mut nodes: Vec<wit::Node> = (0..levels - 1)
                .map(|i| node(wit::NodeKind::Stack, &[i + 1]))
                .collect();
            nodes.push(node(heading("bottom"), &[]));
            check(nodes)
        };
        assert!(deep(8).is_ok());
        assert!(deep(9).unwrap_err().contains("8 deep"));
    }

    #[test]
    fn at_most_five_hundred_nodes() {
        let many = |n: u32| {
            let mut nodes = vec![node(wit::NodeKind::Stack, &(1..n).collect::<Vec<_>>())];
            nodes.extend((1..n).map(|_| node(heading("x"), &[])));
            check(nodes)
        };
        assert!(many(500).is_ok());
        assert!(many(501).unwrap_err().contains("at most 500"));
    }

    #[test]
    fn control_ids_are_unique() {
        let err = check(vec![
            node(wit::NodeKind::Stack, &[1, 2]),
            node(button("go"), &[]),
            node(button("go"), &[]),
        ])
        .unwrap_err();
        assert!(err.contains("two controls"), "{err}");
    }

    #[test]
    fn images_come_from_declared_hosts_and_are_never_svg() {
        let image = |src: &str| {
            check(vec![node(
                wit::NodeKind::Image(wit::Image {
                    src: src.into(),
                    alt: "a".into(),
                }),
                &[],
            )])
        };
        assert!(image("https://img.example.com/a.png").is_ok());
        assert!(image("data:image/png;base64,iVBORw0KGgo=").is_ok());
        assert!(image("https://elsewhere.com/a.png").is_err());
        assert!(image("http://img.example.com/a.png").is_err());
        assert!(image("https://img.example.com/a.svg").is_err());
        assert!(image("data:image/svg+xml;base64,PHN2Zz4=").is_err());
        let big = format!("data:image/png;base64,{}", "A".repeat(MAX_DATA_IMAGE));
        assert!(image(&big).is_err());
    }

    #[test]
    fn a_chart_has_one_value_per_label() {
        let chart = |values: Vec<f64>| {
            check(vec![node(
                wit::NodeKind::Chart(wit::Chart {
                    kind: wit::ChartKind::Bar,
                    labels: vec!["00".into(), "01".into()],
                    series: vec![wit::ChartSeries {
                        name: "price".into(),
                        values,
                    }],
                    unit: None,
                }),
                &[],
            )])
        };
        assert!(chart(vec![1.0, 2.0]).is_ok());
        assert!(chart(vec![1.0]).is_err());
    }

    #[test]
    fn a_surface_prints_as_the_dev_command_shows_it() {
        let s = check(vec![
            node(wit::NodeKind::Stack, &[1, 3]),
            node(wit::NodeKind::Row, &[2]),
            node(
                wit::NodeKind::Reading(wit::Reading {
                    label: "Now".into(),
                    value: "0.91".into(),
                    unit: Some("kr/kWh".into()),
                }),
                &[],
            ),
            node(button("refresh"), &[]),
        ])
        .unwrap();
        assert_eq!(
            s.text(),
            "Stack\n  Row\n    Reading  Now: 0.91 kr/kWh\n  Button     refresh \"Go\" (primary)\n"
        );
    }

    #[test]
    fn actions_arrive_in_the_documented_shape() {
        let parse = |s: &str| serde_json::from_str::<ActionValue>(s).unwrap();
        assert_eq!(parse(r#"{"type":"pressed"}"#), ActionValue::Pressed);
        assert_eq!(parse(r#"{"type":"toggled","value":true}"#), ActionValue::Toggled(true));
        assert_eq!(parse(r#"{"type":"number","value":2.5}"#), ActionValue::Number(2.5));
        assert_eq!(
            parse(r#"{"type":"chosen","value":"DK2"}"#),
            ActionValue::Chosen("DK2".into())
        );
    }
}
