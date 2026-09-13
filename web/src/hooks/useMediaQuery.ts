import { useEffect, useState } from "react";

/**
 * Whether a media query currently matches, and whether it still does.
 *
 * Used to choose between a drawer and a dialog rather than to style
 * them: a drag handle is meaningless with a mouse, and a centred modal
 * is wrong on a phone — so which component renders is the decision, not
 * which classes it gets.
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() =>
    typeof window === "undefined" ? false : window.matchMedia(query).matches,
  );

  useEffect(() => {
    const media = window.matchMedia(query);
    // Read again on mount: the viewport can have changed between the
    // first render and this effect, and a rotated phone should not be
    // holding the other component.
    setMatches(media.matches);
    const follow = (event: MediaQueryListEvent) => setMatches(event.matches);
    media.addEventListener("change", follow);
    return () => media.removeEventListener("change", follow);
  }, [query]);

  return matches;
}
