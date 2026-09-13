import { useEffect, useState } from "react";

/**
 * Light, dark, or whatever the phone is doing.
 *
 * `system` is the default and the honest one: a house dashboard is
 * opened at 7am and at midnight, and the device already knows which of
 * those it is. The other two exist because sometimes it is wrong — a
 * bright kitchen in the evening, a phone left on light all night.
 */
export type Theme = "light" | "dark" | "system";

const KEY = "niles-theme";

/** Matches the two `--background` tokens, so the browser chrome agrees
 *  with the page rather than framing it in the other theme. */
const THEME_COLOR = { dark: "#0a0a0a", light: "#f7f7f8" } as const;

export function storedTheme(): Theme {
  try {
    const value = localStorage.getItem(KEY);
    return value === "light" || value === "dark" ? value : "system";
  } catch {
    // Private windows and blocked site data throw rather than return
    // null. The system setting is a fine answer.
    return "system";
  }
}

export function isDark(theme: Theme): boolean {
  return theme === "system"
    ? window.matchMedia("(prefers-color-scheme: dark)").matches
    : theme === "dark";
}

/**
 * Put the choice on `<html>`, where the CSS is listening.
 *
 * Exported because the same two lines run in an inline script in
 * index.html *before* first paint — without that, a dark-mode phone
 * flashes the light theme for as long as the bundle takes to load,
 * which at 6am is genuinely unpleasant.
 */
export function applyTheme(theme: Theme): void {
  const dark = isDark(theme);
  document.documentElement.classList.toggle("dark", dark);
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", dark ? THEME_COLOR.dark : THEME_COLOR.light);
}

export function useTheme(): [Theme, (next: Theme) => void] {
  const [theme, setThemeState] = useState<Theme>(storedTheme);

  // Following the system means following it as it changes, not only as
  // it was when the page loaded — a phone that switches at sunset
  // should take the page with it.
  useEffect(() => {
    applyTheme(theme);
    if (theme !== "system") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const follow = () => applyTheme("system");
    media.addEventListener("change", follow);
    return () => media.removeEventListener("change", follow);
  }, [theme]);

  return [
    theme,
    (next: Theme) => {
      setThemeState(next);
      try {
        localStorage.setItem(KEY, next);
      } catch {
        // The choice still holds for this page; it just won't outlive it.
      }
    },
  ];
}
