import { beforeEach, describe, expect, it, vi } from "vitest";
import { applyTheme, isDark, storedTheme } from "./theme";

function systemPrefers(dark: boolean) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn().mockReturnValue({
      matches: dark,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  );
}

// jsdom's own localStorage is not reliably present here, and the point
// of these tests is the logic rather than the browser's storage.
let store: Record<string, string> = {};
beforeEach(() => {
  store = {};
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store[k] ?? null,
    setItem: (k: string, v: string) => {
      store[k] = v;
    },
  });
  document.documentElement.className = "";
  document.head.innerHTML = '<meta name="theme-color" content="">';
});

describe("storedTheme", () => {
  it("follows the system when nothing has been chosen", () => {
    // The honest default: the device already knows whether it is 7am.
    expect(storedTheme()).toBe("system");
  });

  it("remembers a choice", () => {
    store["niles-theme"] = "light";
    expect(storedTheme()).toBe("light");
  });

  it("ignores a value it does not recognise", () => {
    store["niles-theme"] = "sepia";
    expect(storedTheme()).toBe("system");
  });
});

describe("isDark", () => {
  it("asks the device when set to system", () => {
    systemPrefers(true);
    expect(isDark("system")).toBe(true);
    systemPrefers(false);
    expect(isDark("system")).toBe(false);
  });

  it("does not ask when told", () => {
    systemPrefers(true);
    expect(isDark("light")).toBe(false);
    systemPrefers(false);
    expect(isDark("dark")).toBe(true);
  });
});

describe("applyTheme", () => {
  it("puts the choice where the CSS is listening", () => {
    systemPrefers(false);
    applyTheme("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    applyTheme("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("moves the browser chrome with the page", () => {
    // A dark page framed in a light status bar is worse than either.
    systemPrefers(false);
    applyTheme("dark");
    expect(
      document.querySelector('meta[name="theme-color"]')?.getAttribute("content"),
    ).toBe("#0a0a0a");
    applyTheme("light");
    expect(
      document.querySelector('meta[name="theme-color"]')?.getAttribute("content"),
    ).toBe("#f7f7f8");
  });
});
