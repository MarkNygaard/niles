import { useEffect } from "react";

/** How far the page scrolls each frame for a drag held `y` from the
    top of the window, with the page visible down to `bottom`: nothing
    but in a thin strip at the top or bottom edge, faster the deeper
    into it, and fastest past it — over the tab bar. Thin, because a
    card sitting low on a short window is somewhere to drop, not a sign
    to scroll: at 96px the "Not playing" card was mostly strip. */
export function edgeScroll(y: number, bottom: number, edge = 40): number {
  const speed = (depth: number) => Math.ceil((Math.min(depth, edge) / edge) * 12);
  if (y < edge) return -speed(edge - y);
  if (y > bottom - edge) return speed(y - (bottom - edge));
  return 0;
}

/** Where the page stops being visible: the top of the tab bar on a
    phone, which sits over the bottom of the page — the strip below it
    is under the bar and the home indicator, where no finger reaches. */
export function visibleBottom(): number {
  const bar = document.querySelector('nav[aria-label="Main"]');
  if (bar && getComputedStyle(bar).position === "fixed") {
    const box = bar.getBoundingClientRect();
    if (box.height > 0) return box.top;
  }
  return window.innerHeight;
}

/**
 * Keeps the page still under a finger on a drag handle (`data-grip`).
 *
 * iOS decides on the first touchmove whether a touch scrolls the page,
 * and `touch-action: none` on the handle does not reliably stop it:
 * the page pulls, elastic, and the pointer is cancelled with the drag.
 * The touchmove is refused natively, on the document, because React's
 * touch listeners are passive and cannot refuse anything.
 */
export function useGripsHoldStill() {
  useEffect(() => {
    const refuse = (e: Event) => {
      if (e.target instanceof Element && e.target.closest("[data-grip]")) e.preventDefault();
    };
    document.addEventListener("touchmove", refuse, { passive: false });
    return () => document.removeEventListener("touchmove", refuse);
  }, []);
}
