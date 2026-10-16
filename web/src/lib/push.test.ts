import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  base64UrlToBytes,
  deniedHelp,
  dismissNudge,
  isAppleMobile,
  nudgeDismissed,
  pushSupport,
  sameKey,
  shouldNudge,
} from "@/lib/push";
import type { PushEnvironment } from "@/lib/push";

const IPHONE =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1";
// iPadOS asks for the desktop site by default, and says it is a Mac.
const IPAD_AS_MAC =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15";
const ANDROID =
  "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Mobile Safari/537.36";

function env(rest: Partial<PushEnvironment> = {}): PushEnvironment {
  return {
    userAgent: ANDROID,
    maxTouchPoints: 5,
    standalone: false,
    hasPush: true,
    permission: "default",
    ...rest,
  };
}

describe("isAppleMobile", () => {
  it("knows an iPhone, and an iPad that says it is a Mac", () => {
    expect(isAppleMobile(IPHONE, 5)).toBe(true);
    expect(isAppleMobile(IPAD_AS_MAC, 5)).toBe(true);
  });

  it("does not take a Mac for an iPad", () => {
    expect(isAppleMobile(IPAD_AS_MAC, 0)).toBe(false);
    expect(isAppleMobile(ANDROID, 5)).toBe(false);
  });
});

describe("pushSupport", () => {
  it("sends an iPhone in Safari to the Home Screen", () => {
    // Safari in a tab has no PushManager either; the Home Screen is
    // still the answer, not another browser.
    expect(pushSupport(env({ userAgent: IPHONE, hasPush: false }))).toBe("needs-home-screen");
  });

  it("offers the switch in the app on the Home Screen", () => {
    expect(pushSupport(env({ userAgent: IPHONE, standalone: true }))).toBe("available");
  });

  it("says a Home Screen app on an iOS too old for push cannot", () => {
    expect(pushSupport(env({ userAgent: IPHONE, standalone: true, hasPush: false }))).toBe(
      "unsupported",
    );
  });

  it("offers it in a tab anywhere else", () => {
    expect(pushSupport(env())).toBe("available");
    expect(pushSupport(env({ permission: "granted" }))).toBe("available");
  });

  it("says no was said", () => {
    expect(pushSupport(env({ permission: "denied" }))).toBe("denied");
  });

  it("says a browser without push cannot", () => {
    expect(pushSupport(env({ hasPush: false }))).toBe("unsupported");
  });
});

describe("deniedHelp", () => {
  it("points an iPhone at its Settings app", () => {
    expect(deniedHelp(env({ userAgent: IPHONE }))).toContain("Settings → Notifications → Niles");
  });

  it("points Android at the app's info", () => {
    expect(deniedHelp(env())).toContain("App info");
  });

  it("points a computer at the site settings", () => {
    expect(deniedHelp(env({ userAgent: IPAD_AS_MAC, maxTouchPoints: 0 }))).toContain(
      "site settings",
    );
  });
});

describe("base64UrlToBytes", () => {
  it("reads base64url without padding", () => {
    expect(Array.from(base64UrlToBytes("AQID"))).toEqual([1, 2, 3]);
    expect(Array.from(base64UrlToBytes("-_8"))).toEqual([251, 255]);
  });

  it("reads a server key as the 65 bytes of a point", () => {
    const key =
      "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const bytes = base64UrlToBytes(key);
    expect(bytes.length).toBe(65);
    expect(bytes[0]).toBe(4);
  });
});

describe("sameKey", () => {
  it("is the same key byte for byte", () => {
    const key = new Uint8Array([4, 1, 2]);
    expect(sameKey(new Uint8Array([4, 1, 2]).buffer, key)).toBe(true);
    expect(sameKey(new Uint8Array([4, 1, 3]).buffer, key)).toBe(false);
    expect(sameKey(new Uint8Array([4, 1]).buffer, key)).toBe(false);
  });

  it("is not a key the subscription does not say", () => {
    expect(sameKey(null, new Uint8Array([4]))).toBe(false);
  });
});

describe("shouldNudge", () => {
  const ready = { support: "available", serverReady: true, on: false, dismissed: false } as const;

  it("offers it where it would work and is not on", () => {
    expect(shouldNudge(ready)).toBe(true);
  });

  it("does not offer it on, dismissed, denied, or with no database", () => {
    expect(shouldNudge({ ...ready, on: true })).toBe(false);
    expect(shouldNudge({ ...ready, dismissed: true })).toBe(false);
    expect(shouldNudge({ ...ready, support: "denied" })).toBe(false);
    expect(shouldNudge({ ...ready, support: "needs-home-screen" })).toBe(false);
    expect(shouldNudge({ ...ready, serverReady: false })).toBe(false);
  });
});

describe("the nudge, dismissed", () => {
  // jsdom's own localStorage is not reliably present here.
  beforeEach(() => {
    const store: Record<string, string> = {};
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => store[k] ?? null,
      setItem: (k: string, v: string) => {
        store[k] = v;
      },
    });
  });
  afterEach(() => vi.unstubAllGlobals());

  it("stays dismissed on this device", () => {
    expect(nudgeDismissed()).toBe(false);
    dismissNudge();
    expect(nudgeDismissed()).toBe(true);
  });
});
