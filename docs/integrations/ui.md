# Pages and widgets

> v0.1 specification. Not implemented yet.

A plugin can add to the Niles app:

- a **page in the menu**, beside Groceries, Chat and Automations;
- **widgets**: cards on the front page or on a room's card.

Both are **declarative**. Your plugin returns a description of what to show,
built from a small set of Niles's own components, and the app draws it. No
HTML, CSS or script of yours runs in the app. That is a restriction, and it
buys three things: your page looks like every other page, it follows dark mode
and the phone layout without you doing anything, and it cannot read the
signed-in session or anything else in the browser, which is why a household can
install it without reading your code first.

Needs `ui = true` in the manifest's permissions.

## Declaring surfaces

Every page and widget, a *surface*, is declared in the manifest.

```toml
[[ui.menu]]
id = "electricity"        # [a-z0-9_]+, unique among your surfaces
label = "Electricity"     # the menu entry's text and the page's title
icon = "zap"              # a Lucide icon name: https://lucide.dev/icons
refresh_every = 300       # optional, seconds, at least 10

[[ui.widgets]]
id = "price_now"
title = "Electricity now" # the card's heading
room = "kitchen"          # optional; without it the widget starts on the front page
refresh_every = 300
```

- **Menu entries** join the menu as `<plugin-id>.<id>` (`spotprice.electricity`).
  The household shows, hides and moves them in Settings → Menu like the
  built-in pages.
- **Widgets** start where the manifest says: a room's card, or the front page.
  The household can move one to another room or the front page, or hide it. A
  `room` the house does not have puts it on the front page.

The approval screen lists each: "Adds a page to the menu: Electricity", "Adds a
card to the front page: Electricity now", "Adds a card to the Kitchen".

## Rendering

When a surface is shown, Niles calls your `render` export:

```rust
fn render(surface_id: String, context: RenderContext) -> Result<UiTree, String>
```

- `surface_id` is the `id` from the manifest.
- `context.room` is the room whose card a widget sits on (or its default room
  on the front page); `None` for menu pages.
- `context.person` is the signed-in person's slug (`mark`) if the app knows who
  is looking, else `None`. It is a slug, never a name or an email address; use
  it to show "your" things, if your plugin has per-person data.

You have **3 seconds**. Draw from what you already hold, in memory or storage;
do not fetch in `render`. Fetch on a timer and call `ui::invalidate` when
there is something new.

Return `Err("No prices yet.")` and the app shows that sentence in place of the
surface.

## The tree

A surface is a tree of nodes. WIT, the interface language, cannot describe a
type that contains itself, so the tree is sent as a **flat list**:

- `nodes[0]` is the root;
- each node lists its children by their index in the same list, in display order;
- a child's index is always greater than its parent's, and a node has at most
  one parent;
- only `stack`, `row` and `card` have children.

Niles rejects a tree that breaks these rules. A small helper keeps it right by
construction, since a node is always pushed after its parent:

```rust
struct Tree(Vec<Node>);

impl Tree {
    fn new(root: NodeKind) -> Self {
        Tree(vec![Node { kind: root, children: Vec::new() }])
    }

    /// Add `kind` as the last child of `parent`; returns its index.
    fn add(&mut self, parent: u32, kind: NodeKind) -> u32 {
        let index = self.0.len() as u32;
        self.0.push(Node { kind, children: Vec::new() });
        self.0[parent as usize].children.push(index);
        index
    }

    fn done(self) -> UiTree {
        UiTree { nodes: self.0 }
    }
}
```

```rust
let mut tree = Tree::new(NodeKind::Stack);
let top = tree.add(0, NodeKind::Row);
tree.add(top, NodeKind::Reading(Reading {
    label: "Now".into(),
    value: "1.23".into(),
    unit: Some("kr/kWh".into()),
}));
tree.add(top, NodeKind::Badge(Badge { label: "Cheap now".into(), tone: Tone::Good }));
tree.add(0, NodeKind::Button(Button {
    id: "refresh".into(),
    label: "Refresh now".into(),
    style: ButtonStyle::Secondary,
}));
Ok(tree.done())
```

At most 500 nodes, nested at most 8 deep, 256 KiB in all.

## The vocabulary

All in `niles::integration::ui`.

### Layout

| Node | What it draws |
|---|---|
| `Stack` | Its children one above the other, spaced like the rest of the app. |
| `Row` | Its children side by side. On a phone it becomes a stack. |
| `Card(Option<String>)` | A card around its children, with an optional title. A widget is already a card, so use this on pages. |

### Content

| Node | Fields | What it draws |
|---|---|---|
| `Heading(String)` | | A section heading. |
| `Text(TextBlock)` | `value`, `muted` | A paragraph. `muted` is smaller and grey, for footnotes. |
| `Reading(Reading)` | `label`, `value`, `unit` | One value, large, with its label: the main number on a page or widget. `value` is a string, formatted by you ("1.23", "21.5"). |
| `Badge(Badge)` | `label`, `tone` | A short pill. `tone` is `Neutral`, `Good`, `Warning` or `Bad`, in the app's colours. |
| `Items(Vec<ListItem>)` | `title`, `detail`, `trailing` | A list: each row a title, an optional second line, and an optional value at the right end ("12 min", "1.04 kr"). |
| `Chart(Chart)` | `kind`, `labels`, `series`, `unit` | A `Line` or `Bar` chart, one label per point and up to 4 series of up to 200 values, in the app's chart colours. `unit` labels the y axis. |
| `Image(Image)` | `src`, `alt` | A picture. See below. |

### Controls

Each control has an `id`, unique within the surface, which `on_action` receives.

| Node | Fields | `on_action` gets |
|---|---|---|
| `Button(Button)` | `id`, `label`, `style`: `Primary`, `Secondary` or `Destructive` | `Pressed` |
| `Switch(SwitchControl)` | `id`, `label`, `value` | `Toggled(bool)` |
| `Slider(Slider)` | `id`, `label`, `min`, `max`, `step`, `value`, `unit` | `Number(f64)`, when let go |
| `Choice(Choice)` | `id`, `label`, `options` (`value`, `label`), `value` | `Chosen(String)`: the option's value |
| `TextField(TextField)` | `id`, `label`, `placeholder`, `value` | `Text(String)`, when submitted |

A control shows the state you rendered. After the user moves it, the app shows
the new value at once and keeps it if your `on_action` succeeds, or puts it
back and shows the error if not.

### Images

`src` is one of:

- an `https` URL on a host your `http` permission declares. Niles fetches it
  itself and serves it to the app, so the viewer's browser never contacts that
  host: no cookies, no referrer, no tracking pixel. PNG, JPEG, WebP or GIF, at
  most 4 MiB.
- a `data:` URI (`data:image/png;base64,...`) of at most 64 KiB.

No SVG, which can carry script.

## Actions

```rust
fn on_action(
    surface_id: String,
    control_id: String,
    value: ActionValue,
    context: RenderContext,
) -> Result<ActionResult, String> {
    match (surface_id.as_str(), control_id.as_str()) {
        ("electricity", "refresh") => {
            fetch_and_store()?;
            Ok(ActionResult { toast: Some("Prices updated".into()), rerender: true })
        }
        _ => Err("That button does nothing.".into()),
    }
}
```

- `context` is the same as for `render`: which room's widget, who pressed it.
- `toast` is a short message the app shows for a few seconds.
- `rerender: true` renders the surface again straight away.
- `Err(text)` shows `text` as an error toast.
- The usual 10-second limit applies. A button that starts something long
  should start it, return, and let a timer finish the work and call
  `ui::invalidate`.

## Refresh

A surface is rendered:

1. when it comes on screen;
2. after an action that returns `rerender: true`;
3. every `refresh_every` seconds while it is on screen, if the manifest sets it;
4. when your plugin calls `ui::invalidate("electricity")`.

`invalidate` is the one to reach for: call it whenever your data changes, from
a timer or an event. Niles pushes a `ui_invalidated` message to every app that
has the surface open, over the WebSocket the app already holds for live device
state (`/events/stream`), and the app asks for a new render only if the surface
is on screen. Nothing polls for invalidations. Calling it when nobody is
looking costs nothing.

`refresh_every` is for things that change with time rather than with data,
like "fetched 12 minutes ago". The app asks again on that interval while the
surface is visible.

Niles keeps the last tree per surface, room and person, and serves it to every
viewer until it is invalidated or `refresh_every` passes, so ten phones open on
one page cost one `render`.

## Not in v0.1

Pages that need more than this vocabulary (a floor plan, a camera grid,
drag-and-drop) cannot be built in v0.1. A later version may add a separate
permission for a page of your own static HTML in a sandboxed iframe, on its own
origin and without the session. Until then, the vocabulary grows instead: if
you need a node that is not here, open an issue on Niles saying what for.
