import { afterEach, describe, expect, it, vi } from "vitest";
import { JSDOM } from "jsdom";
import {
  MAX_HEIGHT,
  MAX_SAVE_CHARS,
  NO_FEATURES,
  PAGE_CSP,
  PageSaves,
  SANDBOX,
  SAVES_PER_OPEN,
  SAVE_AFTER_MS,
  SAVE_WITHIN_MS,
  buildSrcdoc,
  cleanPage,
  openableUrl,
  readMessage,
  readTheme,
  saveReason,
  scriptJson,
} from "@/lib/sandboxedPage";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const THEME = { dark: false, vars: { foreground: "black" } };

function srcdoc(html: string, data: unknown = { markdown: "# Packing" }) {
  return buildSrcdoc({ html, data, theme: THEME, today: "2026-10-16" });
}

function parse(html: string): Document {
  return new DOMParser().parseFromString(html, "text/html");
}

describe("the frame's document", () => {
  it("starts with Niles's policy, before anything of the page's", () => {
    const doc = parse(srcdoc("<h1>Hi</h1>"));
    const head = [...doc.head.children];
    expect(head[0].getAttribute("charset")).toBe("utf-8");
    expect(head[1].getAttribute("http-equiv")).toBe("Content-Security-Policy");
    expect(head[1].getAttribute("content")).toBe(PAGE_CSP);
    expect(head[2].getAttribute("http-equiv")).toBe("x-dns-prefetch-control");
    expect(head[2].getAttribute("content")).toBe("off");
    expect(doc.body.querySelector("h1")?.textContent).toBe("Hi");
  });

  it("allows nothing to load, connect, frame, submit or re-base", () => {
    for (const rule of [
      "default-src 'none'",
      "connect-src 'none'",
      "frame-src 'none'",
      "form-action 'none'",
      "base-uri 'none'",
      "img-src data: blob:",
      "font-src data:",
    ]) {
      expect(PAGE_CSP).toContain(rule);
    }
    expect(PAGE_CSP).not.toMatch(/https?:|\*/);
    expect(SANDBOX).toBe("allow-scripts");
    expect(NO_FEATURES).toContain("camera 'none'");
    expect(NO_FEATURES).toContain("geolocation 'none'");
  });

  it("puts the runtime, with the data, in the head and the page after it", () => {
    const text = srcdoc("<script>niles.onData(draw)</script>");
    const doc = parse(text);
    const [runtime, ...rest] = [...doc.querySelectorAll("script")];
    expect(runtime.parentElement).toBe(doc.head);
    expect(runtime.textContent).toContain('Object.defineProperty(window, "niles"');
    expect(runtime.textContent).toContain('"markdown":"# Packing"');
    expect(rest.map((s) => s.parentElement)).toEqual([doc.body]);
    expect(text.indexOf("Content-Security-Policy")).toBeLessThan(text.indexOf("niles.onData(draw)"));
  });

  it("gives the page's head and its own policy no place before Niles's", () => {
    const doc = parse(
      srcdoc(
        '<!doctype html><html class="poster"><head><meta http-equiv="Content-Security-Policy" ' +
          "content=\"default-src *\"><style>h1{}</style><script>early()</script></head>" +
          '<body class="wide"><h1>Hi</h1></body></html>',
      ),
    );
    const policies = doc.querySelectorAll('meta[http-equiv="Content-Security-Policy"]');
    expect(policies).toHaveLength(1);
    expect(policies[0].getAttribute("content")).toBe(PAGE_CSP);
    const early = [...doc.querySelectorAll("script")].find((s) => s.textContent === "early()");
    expect(early?.parentElement).toBe(doc.body);
    const style = [...doc.querySelectorAll("style")].find((s) => s.textContent === "h1{}");
    expect(style?.parentElement).toBe(doc.body);
    expect(doc.body.classList.contains("wide")).toBe(true);
    expect(doc.documentElement.classList.contains("poster")).toBe(true);
  });

  it("takes out the hints, refreshes, frames and smuggling places of the page's markup", () => {
    const cleaned = parse(
      cleanPage(
        '<link rel="dns-prefetch" href="//leak.example"><link rel="preconnect" href="https://leak.example">' +
          '<link rel="prefetch" href="https://leak.example/a"><link rel="preload" href="https://leak.example/b">' +
          '<meta http-equiv="refresh" content="0;url=https://leak.example">' +
          '<base href="https://leak.example/"><iframe src="https://leak.example"></iframe>' +
          '<object data="https://leak.example"></object><embed src="https://leak.example">' +
          '<noscript><p title="</noscript><link rel=dns-prefetch href=//leak.example>"></p></noscript>' +
          '<script type="speculationrules">{"prefetch":[{"urls":["https://leak.example"]}]}</script>' +
          '<template><link rel="prefetch" href="https://leak.example/t"><p>kept</p></template>' +
          "<p>Packing</p>",
      ),
    );
    expect(cleaned.querySelectorAll("link, meta, base, iframe, object, embed, noscript")).toHaveLength(0);
    expect(cleaned.querySelectorAll("script")).toHaveLength(0);
    const template = cleaned.querySelector("template")!;
    expect(template.content.querySelector("link")).toBeNull();
    expect(template.content.querySelector("p")?.textContent).toBe("kept");
    expect(cleaned.body.textContent).toContain("Packing");
  });

  it("keeps data that looks like markup from ending the runtime's script", () => {
    const sly = {
      markdown: "</script><script>parent.location='https://leak.example'</script><!-- $& $' $`",
      line: `a${String.fromCharCode(0x2028)}b`,
    };
    const text = srcdoc("<p>x</p>", sly);
    const doc = parse(text);
    expect(doc.querySelectorAll("script")).toHaveLength(1);
    expect(text).not.toContain("</script><script>parent");
    const json = scriptJson(sly);
    expect(json).not.toMatch(/[<>&]/);
    expect(JSON.parse(json)).toEqual(sly);
    expect(doc.head.querySelector("script")?.textContent).toContain(json);
  });
});

describe("the runtime, run", async () => {
  /** The frame's document run in a window of its own, once it says it
      is ready. */
  async function run(html: string, data: unknown = { markdown: "# Packing" }) {
    const posts: Record<string, unknown>[] = [];
    const dom = new JSDOM(srcdoc(html, data), {
      runScripts: "dangerously",
      beforeParse(w) {
        // Here the frame is its own parent: what it posts up is caught.
        w.postMessage = ((m: Record<string, unknown>) => posts.push(m)) as typeof w.postMessage;
      },
    });
    for (let i = 0; i < 100 && !posts.some((p) => p.type === "ready"); i++) {
      await new Promise((r) => setTimeout(r, 10));
    }
    return { w: dom.window, posts, types: () => posts.map((p) => p.type) };
  }

  it("hands the page the data, the theme and today, and says when it is ready", async () => {
    const { w, posts, types } = await run(
      "<h1 id=t></h1><script>niles.onData(function (d) { t.textContent = d.markdown + ' ' + niles.today })</script>",
    );
    expect(w.document.getElementById("t")?.textContent).toBe("# Packing 2026-10-16");
    expect(types()).toContain("ready");
    expect(posts.every((p) => p.niles === 1)).toBe(true);
    expect(w.document.documentElement.style.getPropertyValue("--foreground")).toBe("black");
    expect((w as unknown as { RTCPeerConnection?: unknown }).RTCPeerConnection).toBeUndefined();
  });

  it("draws new data from its parent, and nobody else's", async () => {
    const { w } = await run(
      "<h1 id=t></h1><script>niles.onData(function (d) { t.textContent = d.markdown })</script>",
    );
    const send = (data: unknown, source: unknown) =>
      w.dispatchEvent(
        new w.MessageEvent("message", { data: { niles: 1, type: "data", data }, source: source as Window }),
      );
    send({ markdown: "# Marked" }, w);
    expect(w.document.getElementById("t")?.textContent).toBe("# Marked");
    send({ markdown: "# Forged" }, null);
    expect(w.document.getElementById("t")?.textContent).toBe("# Marked");
  });

  it("turns a link into a request to open it, and goes nowhere", async () => {
    const { w, posts } = await run('<a id=a href="https://www.jayfai.com/menu">Jay Fai</a>');
    const click = new w.MouseEvent("click", { bubbles: true, cancelable: true });
    w.document.getElementById("a")!.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(posts).toContainEqual({ niles: 1, type: "open", url: "https://www.jayfai.com/menu" });
  });

  it("reports an error in the page's script", async () => {
    const { posts } = await run("<script>niles.onData(function (d) { d.nothing.here })</script>");
    expect(posts.find((p) => p.type === "error")?.message).toMatch(/here|undefined/);
  });

  it("passes on a save and an open the page asks for", async () => {
    const { posts } = await run(
      "<script>niles.save({ markdown: '- [x] passport' }, 'Packed'); niles.open('https://x.example')</script>",
    );
    expect(posts).toContainEqual({
      niles: 1,
      type: "save",
      content: { markdown: "- [x] passport" },
      reason: "Packed",
    });
    expect(posts).toContainEqual({ niles: 1, type: "open", url: "https://x.example" });
  });
});

describe("a message from the frame", () => {
  const frame = {} as Window;
  const from = (data: unknown, source: unknown = frame, origin = "null") =>
    readMessage(new MessageEvent("message", { data, origin, source: source as Window }), frame);

  it("is read only from the frame's own window, with its opaque origin", () => {
    expect(from({ niles: 1, type: "ready" })).toEqual({ type: "ready" });
    expect(from({ niles: 1, type: "ready" }, {} as Window)).toBeNull();
    expect(from({ niles: 1, type: "ready" }, null)).toBeNull();
    expect(from({ niles: 1, type: "ready" }, frame, "https://niles.example")).toBeNull();
    expect(readMessage(new MessageEvent("message", { data: { niles: 1, type: "ready" } }), null)).toBeNull();
  });

  it("is ignored when it is not one the frame may send", () => {
    for (const data of [
      null,
      "ready",
      { type: "ready" },
      { niles: 2, type: "ready" },
      { niles: 1, type: "token" },
      { niles: 1, type: "height", height: "tall" },
      { niles: 1, type: "height", height: Infinity },
      { niles: 1, type: "open", url: 42 },
      { niles: 1, type: "open", url: `https://x.example/${"a".repeat(2000)}` },
      { niles: 1, type: "save", content: "text" },
      { niles: 1, type: "save", content: [1, 2] },
      { niles: 1, type: "save", content: { markdown: "x".repeat(MAX_SAVE_CHARS) } },
    ]) {
      expect(from(data), JSON.stringify(data)?.slice(0, 80)).toBeNull();
    }
  });

  it("is held to what the view can take", () => {
    expect(from({ niles: 1, type: "height", height: 1e9 })).toEqual({ type: "height", height: MAX_HEIGHT });
    expect(from({ niles: 1, type: "height", height: -5 })).toEqual({ type: "height", height: 0 });
    expect(from({ niles: 1, type: "error", message: "x\n".repeat(400) })).toMatchObject({
      type: "error",
    });
    expect(
      (from({ niles: 1, type: "error", message: "y".repeat(400) }) as { message: string }).message,
    ).toHaveLength(300);
    expect(from({ niles: 1, type: "save", content: { a: 1 }, reason: 7 })).toEqual({
      type: "save",
      content: { a: 1 },
      reason: "",
    });
  });
});

describe("a link the page asks to open", () => {
  const app = "https://niles.example";

  it("opens only over https, to a named host, and never the app", () => {
    expect(openableUrl("https://www.jayfai.com/", app)?.hostname).toBe("www.jayfai.com");
    for (const url of [
      "http://www.jayfai.com/",
      "javascript:alert(1)",
      "data:text/html,hi",
      "https://user:pw@www.jayfai.com/",
      "https://niles.example/me/artifacts",
      "/me/artifacts",
      "not a url",
    ]) {
      expect(openableUrl(url, app), url).toBeNull();
    }
  });
});

describe("a page's saves", () => {
  afterEach(() => vi.useRealTimers());

  it("go together a moment after the last, with the page's reasons", () => {
    vi.useFakeTimers();
    const send = vi.fn();
    const saves = new PageSaves(send);
    saves.push({ n: 1 }, "Packed sunscreen");
    vi.advanceTimersByTime(SAVE_AFTER_MS - 1);
    saves.push({ n: 2 }, "Packed charger");
    vi.advanceTimersByTime(SAVE_AFTER_MS - 1);
    expect(send).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(send).toHaveBeenCalledExactlyOnceWith({ n: 2 }, "On its page: Packed charger (and 1 more)");
  });

  it("go within a while however often the page keeps saving", () => {
    vi.useFakeTimers();
    const send = vi.fn();
    const saves = new PageSaves(send);
    for (let t = 0; t < SAVE_WITHIN_MS; t += SAVE_AFTER_MS / 2) {
      saves.push({ t }, "");
      vi.advanceTimersByTime(SAVE_AFTER_MS / 2);
    }
    expect(send).toHaveBeenCalledTimes(1);
  });

  it("stop being taken once a page has saved its share", () => {
    const send = vi.fn();
    const saves = new PageSaves(send);
    for (let i = 0; i < SAVES_PER_OPEN; i++) {
      expect(saves.push({ i }, "")).toBe(true);
      saves.flush();
    }
    expect(saves.push({ i: "more" }, "")).toBe(false);
    saves.flush();
    expect(send).toHaveBeenCalledTimes(SAVES_PER_OPEN);
  });

  it("say they were the page's", () => {
    expect(saveReason([])).toBe("Changed on its page");
    expect(saveReason(["", "Ticked passport", "Ticked passport"])).toBe("On its page: Ticked passport");
  });
});

describe("the theme handed in", () => {
  it("is the app's tokens, with the light ones where a token is missing", () => {
    const root = document.createElement("html");
    root.classList.add("dark");
    root.style.setProperty("--background", "oklch(0.145 0 0)");
    root.style.setProperty("--foreground", "url(https://leak.example/a.png)");
    document.body.append(root);
    const theme = readTheme(root);
    root.remove();
    expect(theme.dark).toBe(true);
    expect(theme.vars.background).toBe("oklch(0.145 0 0)");
    expect(theme.vars.foreground).toBe("oklch(0.25 0.006 285)");
    expect(Object.keys(theme.vars)).toContain("radius");
  });
});

describe("the app around the frame", () => {
  it("sets no script policy the frame would inherit, and lets no frame navigate", () => {
    // A srcdoc frame inherits the policy of the page it is in. One with
    // script-src would stop the runtime and the page; frame-src 'none'
    // stops the frame sending itself, with what it knows, to an address.
    const indexHtml = readFileSync(resolve(process.cwd(), "index.html"), "utf8");
    const policies = [
      ...parse(indexHtml).querySelectorAll('meta[http-equiv="Content-Security-Policy"]'),
    ].map((m) => m.getAttribute("content") ?? "");
    expect(policies).toEqual(["frame-src 'none'"]);
  });
});
