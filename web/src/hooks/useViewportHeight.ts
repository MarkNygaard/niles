import { useEffect, useState } from "react";

/**
 * How tall the screen is right now, in pixels.
 *
 * For the one thing `100dvh` turned out not to be reliable for: a sheet
 * that has to cover an iPhone. Opened without the page having been
 * touched, the sheet came up short at the bottom by about the height of
 * the home indicator, and a single scroll gesture — on a page with
 * nothing to scroll, so a rubber-band and nothing more — made every
 * later one correct. That is a stale viewport measurement being
 * refreshed, not a layout that was ever wrong.
 *
 * `window.innerHeight` rather than `visualViewport.height`, because the
 * two differ exactly when we do not want them to: a pinch zoom or a
 * keyboard shrinks the visual viewport, and a sheet that resized itself
 * to the keyboard would be a second bug wearing the first one's
 * clothes.
 */
export function useViewportHeight(): number {
  const [height, setHeight] = useState(() =>
    typeof window === "undefined" ? 0 : window.innerHeight,
  );

  useEffect(() => {
    const measure = () => setHeight(window.innerHeight);
    measure();
    window.addEventListener("resize", measure);
    // Fires where `resize` does not on iOS — the browser's own chrome
    // appearing and disappearing.
    window.visualViewport?.addEventListener("resize", measure);
    return () => {
      window.removeEventListener("resize", measure);
      window.visualViewport?.removeEventListener("resize", measure);
    };
  }, []);

  return height;
}
