/**
 * An artifact's page, and the sealed frame it is shown in.
 *
 * A page is HTML the assistant wrote once that draws the artifact's
 * data. The data stays the truth: Niles hands it to the page, and hands
 * it again whenever it changes, so a run marked on a plan redraws the
 * page without anyone writing it again. The threat model, what was
 * tested and what is left, is in ARCHITECTURE.md's Security notes.
 *
 * The page may have been written by a model that read a web page meant
 * to steer it, so the frame trusts it with nothing: no network, no
 * cookies, no Niles, no navigating anywhere. Its one way out is
 * `postMessage` to this window, and every message it sends is read here
 * as a stranger's. This file is the part of that which can be tested
 * without a browser running the frame: the document the frame is given,
 * and what a message from it may say.
 */

/**
 * The frame's own policy, first in its `<head>`, before anything of the
 * page's. A policy can only be tightened by a later one, so nothing the
 * page says can loosen it. Inline script and style are the page; data:
 * and blob: images, fonts and media are what it can draw with, and what
 * Niles will hand it pictures as. Nothing else loads, connects, frames,
 * submits or sets a base address.
 */
export const PAGE_CSP = [
  "default-src 'none'",
  "script-src 'unsafe-inline'",
  "style-src 'unsafe-inline'",
  "img-src data: blob:",
  "font-src data:",
  "media-src data: blob:",
  "connect-src 'none'",
  "frame-src 'none'",
  "child-src 'none'",
  "worker-src 'none'",
  "object-src 'none'",
  "manifest-src 'none'",
  "form-action 'none'",
  "base-uri 'none'",
  // CSP Level 3; a browser that does not know it ignores it, and the
  // runtime takes the constructors away besides.
  "webrtc 'block'",
].join("; ");

/** `sandbox`: scripts, and nothing else. Without `allow-same-origin`
    the frame's origin is opaque: it has no cookies, no storage of the
    app's, and cannot reach into this window. No popups, no top
    navigation, no forms, no modals, no downloads. */
export const SANDBOX = "allow-scripts";

/** `allow`: no powerful feature is delegated to the frame. */
export const NO_FEATURES = [
  "autoplay",
  "bluetooth",
  "camera",
  "clipboard-read",
  "clipboard-write",
  "display-capture",
  "encrypted-media",
  "fullscreen",
  "geolocation",
  "hid",
  "idle-detection",
  "local-fonts",
  "microphone",
  "midi",
  "payment",
  "picture-in-picture",
  "publickey-credentials-create",
  "publickey-credentials-get",
  "screen-wake-lock",
  "serial",
  "storage-access",
  "usb",
  "web-share",
]
  .map((f) => `${f} 'none'`)
  .join("; ");

/** The app's colours, by the names the page uses them under. */
export const THEME_VARS = [
  "background",
  "foreground",
  "card",
  "card-foreground",
  "muted",
  "muted-foreground",
  "accent",
  "accent-foreground",
  "primary",
  "primary-foreground",
  "border",
  "destructive",
  "radius",
  "font-sans",
] as const;

export interface PageTheme {
  dark: boolean;
  vars: Record<string, string>;
}

/** The light theme's tokens, for a document that has no stylesheet: a
    test, or a token renamed in theme.css. */
const LIGHT: Record<string, string> = {
  background: "oklch(0.975 0.0015 286)",
  foreground: "oklch(0.25 0.006 285)",
  card: "oklch(1 0 0)",
  "card-foreground": "oklch(0.25 0.006 285)",
  muted: "oklch(0.95 0.002 286)",
  "muted-foreground": "oklch(0.52 0.006 285)",
  accent: "oklch(0.94 0.003 286)",
  "accent-foreground": "oklch(0.25 0.006 285)",
  primary: "oklch(0.42 0.085 158)",
  "primary-foreground": "oklch(0.99 0 0)",
  border: "oklch(0.93 0.003 286)",
  destructive: "oklch(0.577 0.245 27.325)",
  radius: "0.4rem",
  "font-sans": "ui-sans-serif, system-ui, sans-serif",
};

/** A token's value as the frame may be handed it: what a colour, a
    length or a font list is written with, and short. */
function tokenValue(raw: string): string | undefined {
  const value = raw.trim();
  return value && value.length <= 200 && /^[\w\s.,%()#/+\-"']+$/.test(value) ? value : undefined;
}

/** The theme as the app shows it now, read off `<html>`. */
export function readTheme(root: HTMLElement = document.documentElement): PageTheme {
  const style = getComputedStyle(root);
  const vars: Record<string, string> = {};
  for (const name of THEME_VARS) {
    vars[name] = tokenValue(style.getPropertyValue(`--${name}`)) ?? LIGHT[name];
  }
  return { dark: root.classList.contains("dark"), vars };
}

/**
 * `value` as JSON that can sit inside a `<script>` element: no `<`,
 * `>` or `&` for the HTML parser to find a `</script>` or a comment in,
 * and the two line separators older JavaScript took for line ends. The
 * data is the page's to draw, and it may carry text from anywhere.
 */
export function scriptJson(value: unknown): string {
  return (JSON.stringify(value) ?? "null")
    .replace(/</g, "\\u003c")
    .replace(/>/g, "\\u003e")
    .replace(/&/g, "\\u0026")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");
}

/** What the page's own markup may not keep. `<link>` is how a page
    hints the browser to look up, connect to or fetch an address
    (`dns-prefetch`, `preconnect`, `prefetch`, `preload`), which a
    policy does not cover everywhere; `<meta http-equiv>` can refresh
    the frame to an address; the rest embed or re-base. `<noscript>` is
    gone because it never shows in a frame that runs scripts, and is
    read differently by a parser that does not (this one), which is how
    markup is smuggled past a cleaner. */
const DROPPED = [
  "link",
  "meta",
  "base",
  "iframe",
  "frame",
  "frameset",
  "object",
  "embed",
  "portal",
  "noscript",
  "script[type='speculationrules' i]",
].join(", ");

function strip(root: ParentNode) {
  root.querySelectorAll(DROPPED).forEach((el) => el.remove());
  root.querySelectorAll("template").forEach((t) => strip(t.content));
}

/**
 * The page's markup without what `DROPPED` names, as a whole document
 * for the frame's body to take in: its `<head>`'s styles and scripts
 * come first, and its `<html>` and `<body>` attributes are kept.
 *
 * A second wall, not the first. The page runs script, and script can
 * make any of these again; what stops them is the policy and the
 * sandbox. What this stops is markup that acts before any policy is
 * asked, and a page that would only work with them.
 */
export function cleanPage(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  strip(doc);
  return doc.documentElement.outerHTML;
}

/** The base the page draws on: the app's font and colours, nothing
    around it, and a body that holds its margins so its height is the
    page's. */
const BASE_STYLE =
  "html{color-scheme:light dark;-webkit-text-size-adjust:100%}" +
  "body{margin:0;display:flow-root;background:transparent;color:var(--foreground);" +
  "font-family:var(--font-sans);font-size:14px;line-height:1.45;overflow-wrap:anywhere}" +
  "img,svg,video,canvas{max-width:100%}";

/**
 * Niles's half of the frame, run before any of the page's script. It
 * gives the page `niles` (the data, the theme, the house's today, the
 * artifact's pictures, and the three things it may ask: to be told of
 * new data, to open a link, to save), sets the theme as CSS variables,
 * and tells this window when it is listening, when it is ready, how
 * tall it is, and when it went wrong. Every link the page has becomes a
 * request to open it, because the frame itself may go nowhere.
 *
 * Pictures come as `Blob`s in an `images` message, and are made `blob:`
 * URLs here, in the frame's own opaque origin: one the app made would
 * belong to the app's origin and not load in here. A page with pictures
 * is not drawn until they are in, so it draws once, with them.
 *
 * Plain ES5 in a string: it is not part of the bundle, it is what the
 * frame runs. `__INIT__` is the data, theme, today and what the
 * pictures are, as `scriptJson` writes them.
 */
export const RUNTIME = `(function () {
  "use strict";
  var parent = window.parent;
  var init = __INIT__;
  var listeners = [];
  var started = false;
  var loaded = false;
  var pictures = Object.freeze({});
  var info = {};
  var awaiting = describe(init.images);
  function post(message) {
    message.niles = 1;
    try { parent.postMessage(message, "*"); } catch (e) {}
  }
  function text(x) {
    return String(x && x.message ? x.message : x).slice(0, 300);
  }
  function describe(list) {
    if (!Array.isArray(list)) return false;
    var next = {};
    for (var i = 0; i < list.length; i++) {
      var m = list[i];
      if (!m || typeof m.id !== "number") continue;
      next[String(m.id)] = Object.freeze({ alt: String(m.alt || ""), credit: m.credit || null,
        source: m.source || null, width: m.width, height: m.height });
    }
    info = Object.freeze(next);
    return list.length > 0;
  }
  function keep(blobs) {
    var next = {};
    for (var id in blobs) {
      if (!Object.prototype.hasOwnProperty.call(blobs, id) || !/^[0-9]+$/.test(id)) continue;
      if (!(blobs[id] instanceof Blob)) continue;
      try { next[id] = URL.createObjectURL(blobs[id]); } catch (e) {}
    }
    var old = pictures;
    pictures = Object.freeze(next);
    return old;
  }
  function revoke(urls) {
    for (var id in urls) {
      try { URL.revokeObjectURL(urls[id]); } catch (e) {}
    }
  }
  ["RTCPeerConnection", "webkitRTCPeerConnection", "RTCDataChannel", "RTCIceCandidate",
   "RTCSessionDescription", "RTCRtpSender", "RTCRtpReceiver"].forEach(function (name) {
    try { Object.defineProperty(window, name, { value: undefined, writable: false, configurable: false }); } catch (e) {}
  });
  function theme(t) {
    var root = document.documentElement;
    root.style.colorScheme = t.dark ? "dark" : "light";
    root.classList.toggle("dark", !!t.dark);
    for (var name in t.vars) {
      if (Object.prototype.hasOwnProperty.call(t.vars, name)) {
        root.style.setProperty("--" + name, t.vars[name]);
      }
    }
  }
  var niles = {
    data: init.data,
    theme: init.theme,
    today: init.today,
    onData: function (fn) {
      if (typeof fn !== "function") return;
      listeners.push(fn);
      if (started) fn(niles.data);
    },
    image: function (name) {
      var m = /^(?:niles-image:)?([0-9]+)$/.exec(String(name).trim());
      return m && Object.prototype.hasOwnProperty.call(pictures, m[1]) ? pictures[m[1]] : undefined;
    },
    open: function (url) { post({ type: "open", url: String(url) }); },
    save: function (content, reason) {
      post({ type: "save", content: content, reason: reason === undefined ? "" : String(reason) });
    }
  };
  Object.defineProperty(niles, "images", { get: function () { return pictures; }, enumerable: true });
  Object.defineProperty(niles, "imageInfo", { get: function () { return info; }, enumerable: true });
  Object.defineProperty(window, "niles", { value: niles, writable: false, configurable: false });
  theme(init.theme);
  function deliver() {
    for (var i = 0; i < listeners.length; i++) {
      try { listeners[i](niles.data); } catch (e) { post({ type: "error", message: text(e) }); }
    }
  }
  var height = -1;
  function measure() {
    if (!document.body) return;
    var h = Math.ceil(document.body.scrollHeight);
    if (h !== height) { height = h; post({ type: "height", height: h }); }
  }
  window.addEventListener("error", function (e) { post({ type: "error", message: text(e.error || e.message) }); });
  window.addEventListener("unhandledrejection", function (e) { post({ type: "error", message: text(e.reason) }); });
  window.addEventListener("message", function (event) {
    if (event.source !== parent) return;
    var m = event.data;
    if (!m || m.niles !== 1) return;
    if (m.type === "data") {
      niles.data = m.data;
      if (typeof m.today === "string") niles.today = m.today;
      deliver();
    } else if (m.type === "theme" && m.theme) {
      niles.theme = m.theme;
      theme(m.theme);
    } else if (m.type === "images" && m.images && typeof m.images === "object") {
      describe(m.info);
      var old = keep(m.images);
      if (awaiting) {
        awaiting = false;
        if (loaded) start();
      } else if (started) {
        deliver();
      }
      revoke(old);
    }
  });
  document.addEventListener("click", function (e) {
    var a = e.target && e.target.closest ? e.target.closest("a") : null;
    if (!a) return;
    var href = a.getAttribute("href") || a.getAttribute("xlink:href");
    if (!href) return;
    e.preventDefault();
    if (href.charAt(0) === "#") {
      var to = document.getElementById(decodeURIComponent(href.slice(1)));
      if (to) to.scrollIntoView();
      return;
    }
    try { niles.open(new URL(href, document.baseURI).href); } catch (x) {}
  }, true);
  document.addEventListener("submit", function (e) { e.preventDefault(); }, true);
  function ready() {
    loaded = true;
    if (!awaiting) start();
  }
  function start() {
    if (started) return;
    started = true;
    deliver();
    measure();
    post({ type: "ready" });
    try { new ResizeObserver(measure).observe(document.body); } catch (e) {}
    window.addEventListener("load", measure);
  }
  post({ type: "hello" });
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", ready);
  else ready();
})();`;

/** What the page is told of a picture the artifact keeps, beside the
    picture itself, which comes as a `Blob`. */
export interface PageImage {
  id: number;
  alt: string;
  credit: string | null;
  source: string | null;
  width: number;
  height: number;
}

/** Only what the page is told of each picture, from what the app has. */
export function pageImages(images: readonly PageImage[] | undefined): PageImage[] {
  return (images ?? []).map(({ id, alt, credit, source, width, height }) => ({
    id,
    alt,
    credit,
    source,
    width,
    height,
  }));
}

export interface PageInput {
  html: string;
  data: unknown;
  theme: PageTheme;
  /** The house's today, YYYY-MM-DD. */
  today: string;
  /** The pictures it will be handed; none when left out. */
  images?: PageImage[];
}

/**
 * The frame's whole document. Niles's `<head>` first: the policy, DNS
 * lookups turned off (which, once off, a document cannot turn on
 * again), no referrer, the base style, and the runtime with the data.
 * Then the page's markup, cleaned, in the body, where nothing of its
 * head can come before ours. Nothing of Niles's comes after it, so
 * there is nothing for unclosed markup in the page to swallow.
 */
export function buildSrcdoc({ html, data, theme, today, images = [] }: PageInput): string {
  const init = scriptJson({ data, theme, today, images });
  return (
    "<!doctype html><html><head><meta charset=\"utf-8\">" +
    `<meta http-equiv="Content-Security-Policy" content="${PAGE_CSP}">` +
    '<meta http-equiv="x-dns-prefetch-control" content="off">' +
    '<meta name="referrer" content="no-referrer">' +
    '<meta name="viewport" content="width=device-width, initial-scale=1">' +
    `<style>${BASE_STYLE}</style>` +
    `<script>${RUNTIME.replace("__INIT__", () => init)}</script>` +
    "</head><body>" +
    cleanPage(html) +
    "</body></html>"
  );
}

/** The tallest the frame grows: a page that says it is taller is a page
    trying to push the rest of the view away. */
export const MAX_HEIGHT = 20_000;
/** The most of the content one save may carry, as JSON: a trip, the
    largest kind, at its largest. */
export const MAX_SAVE_CHARS = 400_000;

export type FrameMessage =
  | { type: "hello" }
  | { type: "ready" }
  | { type: "height"; height: number }
  | { type: "error"; message: string }
  | { type: "open"; url: string }
  | { type: "save"; content: Record<string, unknown>; reason: string };

function isRecord(x: unknown): x is Record<string, unknown> {
  return typeof x === "object" && x !== null && !Array.isArray(x);
}

function oneLine(x: unknown, max: number): string {
  return typeof x === "string" ? x.replace(/\s+/g, " ").trim().slice(0, max) : "";
}

/**
 * What a message says, if it is one from the frame `frame` and one it
 * may send; `null` for anything else, which is then ignored. From the
 * frame means its window sent it, and its origin is the opaque one the
 * sandbox gives it: a frame that somehow had the app's origin would be
 * a frame not to listen to.
 */
export function readMessage(
  event: MessageEvent,
  frame: Window | null | undefined,
): FrameMessage | null {
  if (!frame || event.source !== frame || event.origin !== "null") return null;
  const m: unknown = event.data;
  if (!isRecord(m) || m.niles !== 1) return null;
  switch (m.type) {
    case "hello":
      return { type: "hello" };
    case "ready":
      return { type: "ready" };
    case "height":
      return typeof m.height === "number" && Number.isFinite(m.height)
        ? { type: "height", height: Math.min(MAX_HEIGHT, Math.max(0, Math.ceil(m.height))) }
        : null;
    case "error":
      return { type: "error", message: oneLine(m.message, 300) || "an error" };
    case "open":
      return typeof m.url === "string" && m.url.length <= 2000 ? { type: "open", url: m.url } : null;
    case "save": {
      if (!isRecord(m.content)) return null;
      let size: number;
      try {
        size = JSON.stringify(m.content).length;
      } catch {
        return null;
      }
      if (size > MAX_SAVE_CHARS) return null;
      return { type: "save", content: m.content, reason: oneLine(m.reason, 120) };
    }
    default:
      return null;
  }
}

/**
 * A link the page asked to open, if it may be opened: https, to a named
 * host, with no login in it, and not this app. Only after the person
 * has seen where it goes and said yes: a page could otherwise send what
 * it knows away in an address.
 */
export function openableUrl(raw: string, appOrigin: string): URL | null {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" || !url.hostname || url.username || url.password) return null;
  if (url.origin === appOrigin) return null;
  return url;
}

/** How long a page's saves wait for the next one before going, as the
    trip's packing ticks do: thirty ticks in a minute are one version. */
export const SAVE_AFTER_MS = 1500;
/** However often a page keeps saving, the newest goes this long after
    the first that waits. */
export const SAVE_WITHIN_MS = 10_000;
/** The most versions one opening of a page may save. A page that keeps
    saving would otherwise push everything else out of the last 50. */
export const SAVES_PER_OPEN = 10;

/** The reason a page's batched saves are kept under, said as the page's. */
export function saveReason(reasons: string[]): string {
  const distinct = [...new Set(reasons.filter(Boolean))];
  if (distinct.length === 0) return "Changed on its page";
  const last = distinct[distinct.length - 1];
  return distinct.length === 1
    ? `On its page: ${last}`
    : `On its page: ${last} (and ${distinct.length - 1} more)`;
}

/**
 * A page's saves, batched: the newest content goes `SAVE_AFTER_MS`
 * after the last save asked for, or `SAVE_WITHIN_MS` after the first,
 * or when `flush` is called (the view closes, the app is hidden); and
 * no more than `SAVES_PER_OPEN` of them.
 */
export class PageSaves {
  private pending?: { content: Record<string, unknown>; reasons: string[] };
  private quiet?: ReturnType<typeof setTimeout>;
  private within?: ReturnType<typeof setTimeout>;
  private sent = 0;

  constructor(private readonly send: (content: unknown, reason: string) => void) {}

  /** Whether the save was taken: not once a page has saved its share. */
  push(content: Record<string, unknown>, reason: string): boolean {
    if (this.sent >= SAVES_PER_OPEN) return false;
    this.pending = { content, reasons: [...(this.pending?.reasons ?? []), reason] };
    clearTimeout(this.quiet);
    this.quiet = setTimeout(() => this.flush(), SAVE_AFTER_MS);
    this.within ??= setTimeout(() => this.flush(), SAVE_WITHIN_MS);
    return true;
  }

  flush(): void {
    clearTimeout(this.quiet);
    clearTimeout(this.within);
    this.quiet = this.within = undefined;
    const pending = this.pending;
    this.pending = undefined;
    if (!pending) return;
    this.sent += 1;
    this.send(pending.content, saveReason(pending.reasons));
  }
}
