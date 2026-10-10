/** How far the page scrolls each frame for a drag held `y` from the
    top of a `height`-tall window: nothing in the middle, faster the
    deeper into the top or bottom edge. */
export function edgeScroll(y: number, height: number, edge = 96): number {
  if (y < edge) return -Math.ceil(((edge - y) / edge) * 16);
  if (y > height - edge) return Math.ceil(((y - (height - edge)) / edge) * 16);
  return 0;
}
