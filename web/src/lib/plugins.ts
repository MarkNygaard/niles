import { valueAt } from "@/lib/api";
import type { PluginSetting, PluginSummary, PluginWidget } from "@/lib/api";
import { humanize } from "@/lib/rooms";

/** The group a plugin naming none, or one this Niles does not know, is in. */
export const CUSTOM = "custom";

/** Where Niles's own add-ons are released. */
const FIRST_PARTY_REPO = "https://github.com/marknygaard/niles-addons";

/** One of Niles's own add-ons, which carries no Custom badge. GitHub
    names are case-insensitive, so the repo is compared the same way. */
export function isFirstParty(plugin: Pick<PluginSummary, "repo">): boolean {
  return plugin.repo?.toLowerCase().replace(/\/$/, "") === FIRST_PARTY_REPO;
}

/**
 * Plugins under their groups, in the order they first appear, with
 * Custom last: it follows the built-in groups on the page.
 */
export function pluginGroupsOf(
  plugins: PluginSummary[],
): { group: PluginSummary["group"]; items: PluginSummary[] }[] {
  const groups = new Map<string, { group: PluginSummary["group"]; items: PluginSummary[] }>();
  for (const plugin of plugins) {
    const entry = groups.get(plugin.group.id);
    if (entry) entry.items.push(plugin);
    else groups.set(plugin.group.id, { group: plugin.group, items: [plugin] });
  }
  return [...groups.values()].sort(
    (a, b) => Number(a.group.id === CUSTOM) - Number(b.group.id === CUSTOM),
  );
}

// ---------------------------------------------------------------------------
// The setup panel

/** What the form holds while it is edited: text for anything typed
    (a number too, so a half-typed "1." is not lost), a flag for a
    boolean. Secrets are not in it: each has its own field. */
export type Draft = Record<string, string | boolean>;

/** The form as it opens: what the household entered, else empty, so a
    default shows as the placeholder rather than as an entered value. */
export function draftOf(settings: PluginSetting[]): Draft {
  const draft: Draft = {};
  for (const s of settings) {
    if (s.type === "secret") continue;
    if (s.type === "boolean") {
      draft[s.key] = typeof s.value === "boolean" ? s.value : s.default === true;
    } else {
      draft[s.key] = s.value === null ? "" : String(s.value);
    }
  }
  return draft;
}

/** A choice's options, as its Select offers them. */
export function choiceOptions(setting: PluginSetting): { value: string; label: string }[] {
  return setting.options.map((o) => ({ value: o.value, label: o.label }));
}

/** What a room setting offers: the house's rooms, and the one already
    chosen even if nothing is in it any more, so it is not lost. */
export function roomOptions(
  rooms: string[],
  current?: string,
): { value: string; label: string }[] {
  const all = current && !rooms.includes(current) ? [...rooms, current] : rooms;
  return all.map((room) => ({ value: room, label: humanize(room) }));
}

/** How a default reads beside its field. */
export function defaultText(setting: PluginSetting): string | null {
  if (setting.default === null) return null;
  if (setting.type === "boolean") return setting.default ? "On" : "Off";
  if (setting.type === "choice") {
    return setting.options.find((o) => o.value === setting.default)?.label ?? String(setting.default);
  }
  if (setting.type === "room") return humanize(String(setting.default));
  return String(setting.default);
}

/**
 * What saving the form writes: `PATCH /config` under `plugins.<id>`,
 * only for what changed, with `null` for a field emptied back to its
 * default. `errors` names the fields that cannot be saved as typed.
 */
export function settingsPatch(
  pluginId: string,
  settings: PluginSetting[],
  draft: Draft,
): { patch: Record<string, unknown> | null; errors: Record<string, string> } {
  const changes: Record<string, unknown> = {};
  const errors: Record<string, string> = {};
  for (const s of settings) {
    if (s.type === "secret" || !(s.key in draft)) continue;
    const typed = draft[s.key];
    let next: string | number | boolean | null;
    if (s.type === "boolean") {
      next = typed === true;
      // A boolean left at its default is not something entered.
      if (s.value === null && next === (s.default === true)) continue;
    } else {
      const text = String(typed).trim();
      if (text === "") {
        next = null;
      } else if (s.type === "number") {
        const n = Number(text.replace(",", "."));
        if (!Number.isFinite(n)) {
          errors[s.key] = `${s.label} is a number.`;
          continue;
        }
        if ((s.min !== null && n < s.min) || (s.max !== null && n > s.max)) {
          errors[s.key] = `${s.label} is between ${s.min ?? "-∞"} and ${s.max ?? "∞"}.`;
          continue;
        }
        next = n;
      } else {
        next = text;
      }
    }
    if (next !== s.value) changes[s.key] = next;
  }
  const patch =
    Object.keys(changes).length > 0 ? { plugins: { [pluginId]: changes } } : null;
  return { patch, errors };
}

/** The labels of required settings with no value: none entered or
    stored, and no default. */
export function missingRequired(settings: PluginSetting[]): string[] {
  return settings
    .filter((s) => s.required && !s.set && (s.type === "secret" || s.default === null))
    .map((s) => s.label);
}

/** "Not set up: needs API key", while a required setting is missing;
    otherwise what the plugin says itself. */
export function setupLine(plugin: PluginSummary): string {
  const missing = missingRequired(plugin.settings);
  if (missing.length > 0 && plugin.enabled) return `Not set up: needs ${and(missing)}`;
  return plugin.status_line;
}

function and(items: string[]): string {
  if (items.length < 2) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

// ---------------------------------------------------------------------------
// Dates

/** `14 Oct 2026`, in the reader's own format: when a plugin was
    installed, or when a library release came out. */
export function day(iso: string | null): string | null {
  if (!iso) return null;
  return new Date(iso).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

// ---------------------------------------------------------------------------
// Pages in the menu

/** A plugin's page as the menu has it. */
export interface PluginPageEntry {
  /** `spotprice.electricity`, as `[menu]` names it. */
  id: string;
  plugin: string;
  page: string;
  label: string;
  icon: string;
  refresh_every: number | null;
}

/** Every page of every plugin that is on. A plugin switched off takes
    its pages out of the menu; one that is failing keeps them, and they
    say so. */
export function pluginPages(plugins: PluginSummary[]): PluginPageEntry[] {
  return plugins
    .filter((p) => p.enabled)
    .flatMap((p) =>
      p.menu.map((m) => ({
        id: `${p.id}.${m.id}`,
        plugin: p.id,
        page: m.id,
        label: m.label,
        icon: m.icon,
        refresh_every: m.refresh_every,
      })),
    );
}

/** `#/plugins/spotprice/electricity`. */
export function pageHref(entry: { plugin: string; page: string }): string {
  return `#/plugins/${encodeURIComponent(entry.plugin)}/${encodeURIComponent(entry.page)}`;
}

/** The plugin and page a route names, if it names one. */
export function pageRoute(route: string): { plugin: string; page: string } | null {
  const match = /^\/plugins\/([^/]+)\/([^/]+)$/.exec(route);
  return match
    ? { plugin: decodeURIComponent(match[1]), page: decodeURIComponent(match[2]) }
    : null;
}

// ---------------------------------------------------------------------------
// Widgets

/** One widget where it is drawn. */
export interface PlacedWidget {
  plugin: string;
  widget: PluginWidget;
  /** The room it is drawn for: its card's, or its home room on the
      front page. What `render` gets as the context's room. */
  room: string | null;
}

/**
 * Where a widget goes: `[menu.widgets.<plugin>]` if the household moved
 * it ("front", "room:<id>" or "hidden"), else where its manifest starts
 * it, a room or the front page.
 */
export function widgetPlace(plugin: string, widget: PluginWidget, effective: unknown): string {
  const moved = valueAt(effective, `menu.widgets.${plugin}`);
  const placed =
    moved && typeof moved === "object" ? (moved as Record<string, unknown>)[widget.id] : undefined;
  if (typeof placed === "string") return placed;
  return widget.room ? `room:${widget.room}` : "front";
}

/**
 * Every widget of every plugin that is on, on the front page or under
 * a room. A room the house does not have puts it on the front page, as
 * the spec says, rather than nowhere.
 */
export function placeWidgets(
  plugins: PluginSummary[],
  effective: unknown,
  rooms: string[],
): { front: PlacedWidget[]; rooms: Record<string, PlacedWidget[]> } {
  const front: PlacedWidget[] = [];
  const byRoom: Record<string, PlacedWidget[]> = {};
  for (const plugin of plugins.filter((p) => p.enabled)) {
    for (const widget of plugin.widgets) {
      const place = widgetPlace(plugin.id, widget, effective);
      if (place === "hidden") continue;
      const room = place.startsWith("room:") ? place.slice(5) : null;
      if (room && rooms.includes(room)) {
        (byRoom[room] ??= []).push({ plugin: plugin.id, widget, room });
      } else {
        front.push({ plugin: plugin.id, widget, room: room ?? widget.room });
      }
    }
  }
  return { front, rooms: byRoom };
}

/** What the place of a widget can be set to, as its Select offers it. */
export function widgetPlaceOptions(rooms: string[]): { value: string; label: string }[] {
  return [
    { value: "front", label: "Front page" },
    ...rooms.map((room) => ({ value: `room:${room}`, label: humanize(room) })),
    { value: "hidden", label: "Hidden" },
  ];
}

// ---------------------------------------------------------------------------
// Pictures

/**
 * Where the app loads a page's picture from. A `data:` image is drawn
 * as it is, since it was checked with the tree; anything else comes
 * through Niles, so the browser never contacts the plugin's host.
 */
export function imageUrl(plugin: string, src: string): string {
  if (src.startsWith("data:image/")) return src;
  return `/plugins/${encodeURIComponent(plugin)}/image?src=${encodeURIComponent(src)}`;
}

// ---------------------------------------------------------------------------
// Accounts and webhooks (interface 0.3)

/**
 * What is wrong with a public address, as `[public] url` would refuse
 * it, or null. Mirrors `PublicConfig::validate` in niles-config, so the
 * field can say so before a save does.
 */
export function publicUrlProblem(url: string): string | null {
  const trimmed = url.trim().replace(/\/+$/, "");
  if (trimmed === "") return null;
  if (!trimmed.startsWith("https://")) return "It has to start with https://.";
  const host = trimmed.slice("https://".length).split("/")[0];
  if (!host) return "It has no host.";
  if (/\s/.test(trimmed)) return "It has a space in it.";
  if (/[?#@]/.test(trimmed)) return "Just the address: no query, fragment or username.";
  return null;
}

/**
 * Where a plugin's Connect button goes: its connect route on the public
 * address, not on whichever address this page was opened at. The
 * service sends the browser back to the public one, so the attempt has
 * to start there, with that address's cookie and sign-in.
 */
export function connectHref(callbackUrl: string): string {
  return callbackUrl.replace(/\/callback$/, "/connect");
}

/**
 * What a return from a sign-in page says, read from the query Niles
 * sent the browser back with, `?plugin=withings&plugin_connected=…`.
 * Null when the page was not opened by one.
 */
export function connectOutcome(
  search: string,
): { plugin: string; ok: boolean; text: string } | null {
  const query = new URLSearchParams(search);
  const plugin = query.get("plugin");
  if (!plugin) return null;
  const connected = query.get("plugin_connected");
  const error = query.get("plugin_error");
  if (connected !== null) return { plugin, ok: true, text: connected };
  if (error !== null) return { plugin, ok: false, text: error };
  return null;
}

/** When the webhook was last called and what it answered, in words. */
export function lastHookLine(
  last: { at: string; status: number } | null,
  now: Date = new Date(),
): string {
  if (!last) return "Not called since the add-on started.";
  const minutes = Math.round((now.getTime() - new Date(last.at).getTime()) / 60_000);
  const when =
    minutes < 1
      ? "just now"
      : minutes < 60
        ? `${minutes} min ago`
        : new Date(last.at).toLocaleString(undefined, {
            day: "numeric",
            month: "short",
            hour: "2-digit",
            minute: "2-digit",
          });
  const ok = last.status >= 200 && last.status < 300;
  return `Last called ${when}: ${ok ? "answered" : "refused"} (${last.status}).`;
}
