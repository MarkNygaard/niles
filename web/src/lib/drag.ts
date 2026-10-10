import { useEffect } from "react";

/** How far the page scrolls each frame for a drag held `y` from the
    top of a `height`-tall window: nothing but in a thin strip at the
    top or bottom edge, and faster the deeper into it. Thin, because a
    card sitting low on a short window is somewhere to drop, not a sign
    to scroll: at 96px the "Not playing" card was mostly strip. */
export function edgeScroll(y: number, height: number, edge = 40): number {
  if (y < edge) return -Math.ceil(((edge - y) / edge) * 12);
  if (y > height - edge) return Math.ceil(((y - (height - edge)) / edge) * 12);
  return 0;
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
