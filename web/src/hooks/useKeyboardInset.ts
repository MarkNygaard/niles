import { useEffect, useState } from "react";

/** Less than this is the browser's own chrome moving, not a keyboard. */
const KEYBOARD_MIN_PX = 100;

/**
 * How much of the bottom of the page the on-screen keyboard covers.
 *
 * iOS lays the keyboard over the page rather than shrinking it, so a
 * field pinned to the bottom of the page ends up behind it — or, once
 * Safari scrolls it into view, with the page's own bottom spacing
 * between it and the keys. The visual viewport is the part still
 * showing; what is missing below it is the keyboard. Android resizes
 * the page instead, and this comes out as zero there.
 */
export function keyboardInset(
  innerHeight: number,
  viewport: { height: number; offsetTop: number },
): number {
  const covered = Math.round(innerHeight - viewport.height - viewport.offsetTop);
  return covered >= KEYBOARD_MIN_PX ? covered : 0;
}

export function useKeyboardInset(): number {
  const [inset, setInset] = useState(0);

  useEffect(() => {
    const viewport = window.visualViewport;
    if (!viewport) return;
    const update = () => setInset(keyboardInset(window.innerHeight, viewport));
    update();
    viewport.addEventListener("resize", update);
    viewport.addEventListener("scroll", update);
    return () => {
      viewport.removeEventListener("resize", update);
      viewport.removeEventListener("scroll", update);
    };
  }, []);

  return inset;
}
