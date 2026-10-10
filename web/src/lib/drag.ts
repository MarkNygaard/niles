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
