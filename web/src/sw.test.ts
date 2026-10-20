import { afterEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// The real worker, run against a fake `self`: what it shows, what it
// fetches and which windows it talks to are all observable.
const SOURCE = readFileSync(resolve(process.cwd(), "public/sw.js"), "utf8");

interface PushMessage {
  title?: string;
  body?: string;
  url?: string;
  tag?: string;
  approval?: { id: string; until?: number; actions?: string[] } | null;
  quiet?: boolean;
}

interface NotificationData {
  url?: string;
  approval?: string;
  answering?: boolean;
}

interface Shown {
  title: string;
  body: string;
  tag?: string;
  data: NotificationData;
  actions?: { action: string; title: string }[];
  silent: boolean;
  renotify: boolean;
  closed: boolean;
  close: () => void;
}

interface WindowClient {
  url: string;
  focus: () => Promise<void>;
  postMessage: (message: unknown) => void;
}

interface FetchInit {
  method?: string;
  credentials?: string;
  headers?: Record<string, string>;
  body?: string;
}

/** What answering a notification fetches, and what the server said. */
type DecideResponse = { ok: boolean; status: number };
type FetchCall = (url: string, init?: FetchInit) => Promise<DecideResponse>;
type FetchMock = Mock<FetchCall>;

interface Worker {
  listeners: Map<string, (event: Record<string, unknown>) => void>;
  shown: Shown[];
  showing: Map<string, Shown>;
  fetchMock: FetchMock;
  windows: WindowClient[];
  openWindow: (url: string) => Promise<void>;
}

function pushEvent(message: PushMessage | string) {
  return {
    data: {
      json: () => message,
      text: () => (typeof message === "string" ? message : JSON.stringify(message)),
    },
  };
}

/** Loads the real public/sw.js with a fake `self` and a mocked `fetch`. */
function worker(): Worker {
  const listeners = new Map<string, (event: Record<string, unknown>) => void>();
  const shown: Shown[] = [];
  const showing = new Map<string, Shown>();
  const windows: WindowClient[] = [];
  const openWindow = vi.fn(async (_url: string) => {});
  const fetchMock: FetchMock = vi.fn(async (_url: string, _init?: FetchInit) => ({
    ok: true,
    status: 200,
  }));
  const registration = {
    showNotification: (
      title: string,
      options: Omit<Shown, "title" | "closed" | "close">,
    ) => {
      const notification: Shown = {
        title,
        ...options,
        closed: false,
        close: () => {
          notification.closed = true;
          if (notification.tag && showing.get(notification.tag) === notification) {
            showing.delete(notification.tag);
          }
        },
      };
      shown.push(notification);
      if (notification.tag) showing.set(notification.tag, notification);
      return Promise.resolve();
    },
    getNotifications: async ({ tag }: { tag?: string }) => {
      const found = tag ? showing.get(tag) : undefined;
      return found ? [found] : [];
    },
  };
  const self = {
    addEventListener: (type: string, listener: (event: Record<string, unknown>) => void) => {
      listeners.set(type, listener);
    },
    location: { origin: "https://niles.example" },
    registration,
    clients: {
      matchAll: async () => windows,
      openWindow,
      claim: async () => {},
    },
    skipWaiting: () => {},
  };
  new Function("self", "fetch", SOURCE)(self, fetchMock);
  return { listeners, shown, showing, fetchMock, windows, openWindow };
}

async function fire(w: Worker, type: string, event: Record<string, unknown>): Promise<void> {
  const listener = w.listeners.get(type);
  if (!listener) throw new Error(`no listener for ${type}`);
  const waits: Promise<unknown>[] = [];
  listener({ waitUntil: (p: Promise<unknown>) => waits.push(p), ...event });
  await Promise.all(waits);
}

function shownFor(w: Worker, tag: string): Shown[] {
  return w.shown.filter((n) => n.tag === tag);
}

function lastFor(w: Worker, tag: string): Shown {
  const forTag = shownFor(w, tag);
  if (!forTag.length) throw new Error(`nothing shown for ${tag}`);
  return forTag[forTag.length - 1];
}

function waitingShown(w: Worker, tag: string, approvalId = "a1"): Shown {
  const shown = {
    title: "Let Linear create issue? · Linear",
    body: "Title: WLED presets",
    tag,
    data: { url: `/#/chat?approval=${approvalId}`, approval: approvalId },
    actions: [
      { action: "approve", title: "Approve" },
      { action: "deny", title: "Deny" },
    ],
    silent: false,
    renotify: true,
    closed: false,
    close: vi.fn(),
  };
  w.shown.push(shown);
  w.showing.set(tag, shown);
  return shown;
}

describe("the service worker's push", () => {
  it("shows a plain push as before", async () => {
    const w = worker();
    await fire(w, "push", pushEvent({ title: "Niles", body: "Your plan is ready", url: "/#/chat?artifact=7", tag: "artifact-7" }));
    const n = lastFor(w, "artifact-7");
    expect(n.renotify).toBe(true);
    expect(n.silent).toBe(false);
    expect(n.actions).toEqual([]);
    expect(n.data.url).toBe("/#/chat?artifact=7");
    expect(n.data.approval).toBeUndefined();
  });

  it("shows a non-JSON push as its text", async () => {
    const w = worker();
    await fire(w, "push", {
      data: {
        json: () => {
          throw new Error("not json");
        },
        text: () => "just some words",
      },
    });
    expect(w.shown[0].body).toBe("just some words");
  });

  it("gives a waiting approval its buttons and says until when", async () => {
    const w = worker();
    const until = Date.now() + 30 * 60 * 1000;
    await fire(
      w,
      "push",
      pushEvent({
        title: "Let Linear create issue? · Linear",
        body: "Title: x",
        url: "/#/chat?approval=a1",
        tag: "approval-a1",
        approval: { id: "a1", until, actions: ["approve", "deny"] },
        quiet: false,
      }),
    );
    const n = lastFor(w, "approval-a1");
    expect(n.actions).toEqual([
      { action: "approve", title: "Approve" },
      { action: "deny", title: "Deny" },
    ]);
    expect(n.body).toBe(
      `Title: x\nWaiting until ${new Date(until).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`,
    );
    expect(n.data.approval).toBe("a1");
    expect(n.renotify).toBe(true);
  });

  it("gives a mail Review and Deny", async () => {
    const w = worker();
    const until = Date.now() + 30 * 60 * 1000;
    await fire(
      w,
      "push",
      pushEvent({
        title: "Send this email? · From your Gmail",
        body: "To: jakob@example.com",
        url: "/#/chat?approval=m1",
        tag: "approval-m1",
        approval: { id: "m1", until, actions: ["review", "deny"] },
      }),
    );
    expect(lastFor(w, "approval-m1").actions).toEqual([
      { action: "review", title: "Review" },
      { action: "deny", title: "Deny" },
    ]);
  });

  it("drops buttons it does not know", async () => {
    const w = worker();
    const until = Date.now() + 30 * 60 * 1000;
    await fire(
      w,
      "push",
      pushEvent({
        title: "t",
        body: "b",
        tag: "approval-a1",
        approval: { id: "a1", until, actions: ["approve", "toString", "explode"] },
      }),
    );
    expect(lastFor(w, "approval-a1").actions).toEqual([{ action: "approve", title: "Approve" }]);
  });

  it("asks nothing once its time has passed", async () => {
    const w = worker();
    await fire(
      w,
      "push",
      pushEvent({
        title: "t",
        body: "b",
        url: "/#/chat?approval=a1",
        tag: "approval-a1",
        approval: { id: "a1", until: Date.now() - 1000, actions: ["approve", "deny"] },
      }),
    );
    const n = lastFor(w, "approval-a1");
    expect(n.actions).toEqual([]);
    expect(n.body).toMatch(/No longer waiting\.$/);
    expect(n.data.approval).toBeUndefined();
  });

  it("shows a replacement quietly", async () => {
    const w = worker();
    await fire(
      w,
      "push",
      pushEvent({
        title: "t",
        body: "Allowed. Done.",
        url: "/#/chat?approval=a1",
        tag: "approval-a1",
        approval: null,
        quiet: true,
      }),
    );
    const n = lastFor(w, "approval-a1");
    expect(n.silent).toBe(true);
    expect(n.renotify).toBe(false);
    expect(n.actions).toEqual([]);
  });
});

describe("the service worker's notificationclick", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("approves with the session, as the card's button would", async () => {
    const w = worker();
    const notification = waitingShown(w, "approval-a1");
    await fire(w, "notificationclick", { notification, action: "approve" });
    expect(w.fetchMock).toHaveBeenCalledWith("/me/approvals/a1", {
      method: "POST",
      credentials: "same-origin",
      headers: { "content-type": "application/json" },
      body: '{"decision":"once"}',
    });
    const forTag = shownFor(w, "approval-a1");
    expect(forTag.map((n) => n.body)).toEqual(["Title: WLED presets", "Answering…", "Allowed."]);
    const answering = forTag[1];
    expect(answering.silent).toBe(true);
    expect(answering.data.answering).toBe(true);
    const allowed = forTag[2];
    expect(allowed.silent).toBe(true);
    expect(allowed.actions).toBeUndefined();
    expect(allowed.data.url).toBe("https://niles.example/#/chat?approval=a1");
  });

  it("denies", async () => {
    const w = worker();
    const notification = waitingShown(w, "approval-a1");
    await fire(w, "notificationclick", { notification, action: "deny" });
    expect(w.fetchMock).toHaveBeenCalledWith(
      "/me/approvals/a1",
      expect.objectContaining({ body: '{"decision":"deny"}' }),
    );
    expect(lastFor(w, "approval-a1").body).toBe("Denied. Nothing was done.");
  });

  it("says when it is no longer waiting", async () => {
    for (const status of [404, 409]) {
      const w = worker();
      w.fetchMock.mockResolvedValue({ ok: false, status });
      const notification = waitingShown(w, "approval-a1");
      await fire(w, "notificationclick", { notification, action: "approve" });
      expect(lastFor(w, "approval-a1").body).toBe(
        "No longer waiting. It was answered, or it lapsed.",
      );
      expect(shownFor(w, "approval-a1").map((n) => n.body)).not.toContain("Allowed.");
    }
  });

  it("never fails silently", async () => {
    const cases: Array<[string, { ok: boolean; status: number } | "reject"]> = [
      ["401", { ok: false, status: 401 }],
      ["500", { ok: false, status: 500 }],
      ["offline", "reject"],
    ];
    for (const [name, outcome] of cases) {
      const w = worker();
      if (outcome === "reject") w.fetchMock.mockRejectedValue(new Error("offline"));
      else w.fetchMock.mockResolvedValue(outcome);
      const notification = waitingShown(w, "approval-a1");
      await fire(w, "notificationclick", { notification, action: "approve" });
      const last = lastFor(w, "approval-a1");
      expect(last.body, name).toBe("Couldn't answer. Open Niles to answer it.");
      expect(last.data.url).toBe("https://niles.example/#/chat?approval=a1");
    }
  });

  it("leaves Niles's own word if it came first", async () => {
    const w = worker();
    w.fetchMock.mockImplementation(async () => {
      await fire(
        w,
        "push",
        pushEvent({
          title: "Let Linear create issue? · Linear",
          body: "Allowed. Done.",
          url: "/#/chat?approval=a1",
          tag: "approval-a1",
          approval: null,
          quiet: true,
        }),
      );
      return { ok: true, status: 200 };
    });
    const notification = waitingShown(w, "approval-a1");
    await fire(w, "notificationclick", { notification, action: "approve" });
    const bodies = shownFor(w, "approval-a1").map((n) => n.body);
    expect(bodies).toContain("Allowed. Done.");
    expect(bodies.lastIndexOf("Allowed. Done.")).toBe(bodies.length - 1);
    expect(bodies).not.toContain("Allowed.");
  });

  it("review opens the card and answers nothing", async () => {
    const w = worker();
    const notification = waitingShown(w, "approval-a1");
    await fire(w, "notificationclick", { notification, action: "review" });
    expect(w.fetchMock).not.toHaveBeenCalled();
    expect(notification.close).toHaveBeenCalled();
    expect(w.openWindow).toHaveBeenCalledWith("https://niles.example/#/chat?approval=a1");
  });

  it("a tap opens the card in the window already open", async () => {
    const w = worker();
    const focus = vi.fn(async () => {});
    const postMessage = vi.fn();
    w.windows.push({
      url: "https://niles.example/#/rooms",
      focus,
      postMessage,
    });
    const notification = waitingShown(w, "approval-a1");
    await fire(w, "notificationclick", { notification, action: "" });
    expect(notification.close).toHaveBeenCalled();
    expect(focus).toHaveBeenCalled();
    expect(postMessage).toHaveBeenCalledWith({
      type: "open",
      url: "https://niles.example/#/chat?approval=a1",
    });
    expect(w.openWindow).not.toHaveBeenCalled();
  });

  it("a button on an old notification without an approval just opens it", async () => {
    const w = worker();
    const notification = waitingShown(w, "artifact-7", "a1");
    notification.data = { url: "/#/chat?artifact=7" };
    await fire(w, "notificationclick", { notification, action: "approve" });
    expect(w.fetchMock).not.toHaveBeenCalled();
    expect(notification.close).toHaveBeenCalled();
    expect(w.openWindow).toHaveBeenCalledWith("https://niles.example/#/chat?artifact=7");
  });
});
