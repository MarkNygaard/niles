import { useEffect } from "react";

/**
 * Lend the browser's own chrome a colour while something is open.
 *
 * In a standalone app the strip behind the clock belongs to iOS, not
 * to the page: no element reaches it and no stylesheet paints it. The
 * `theme-color` meta is the whole of the page's say in the matter, and
 * iOS re-reads it when it changes — so a full-screen sheet can carry
 * its colour up there for as long as it is open.
 *
 * `null` puts back whatever was there, which is the theme's own colour
 * as index.html set it before the bundle had loaded.
 */
export function useThemeColor(color: string | null) {
  useEffect(() => {
    if (color === null) return;
    const meta = document.querySelector('meta[name="theme-color"]');
    if (!meta) return;
    const previous = meta.getAttribute("content");
    meta.setAttribute("content", color);
    return () => {
      if (previous !== null) meta.setAttribute("content", previous);
    };
  }, [color]);
}
