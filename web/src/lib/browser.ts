/** The person's browsing session, for the Browser tab and the handoff
    cards. */
export const BROWSER = ["me-browser"];

/** The said line of the turn giving the browser back starts. Mirrors
    `GAVE_BACK` in crates/niles-bin/src/browsing/handover.rs. */
export const GAVE_BACK = "Gave the browser back";

/** Keys Niles passes on by name; any other key is one character. Mirrors
    `NAMED` in crates/niles-browser/src/live.rs. */
export const NAMED_KEYS = [
  "Enter",
  "Backspace",
  "Tab",
  "Escape",
  "Delete",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "Home",
  "End",
  "PageUp",
  "PageDown",
] as const;

/** Where the live view's socket is, on the page's own host. */
export function liveUrl(location: Pick<Location, "protocol" | "host">): string {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${location.host}/me/browser/live`;
}

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** The point on the page under a point on the screen: the picture is
    the page scaled to fit, so scale back. `null` off the picture. */
export function pageAt(
  clientX: number,
  clientY: number,
  shown: Box,
  page: { width: number; height: number },
): { x: number; y: number } | null {
  if (shown.width <= 0 || shown.height <= 0) return null;
  const x = ((clientX - shown.left) * page.width) / shown.width;
  const y = ((clientY - shown.top) * page.height) / shown.height;
  if (x < 0 || y < 0 || x > page.width || y > page.height) return null;
  return { x: Math.round(x * 10) / 10, y: Math.round(y * 10) / 10 };
}

/** Alt, Ctrl, Meta and Shift as the browser's bit mask. */
export function modifiersOf(e: {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): number {
  return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
}

/** The key Niles passes on for a keyboard event, or `null` for one it
    doesn't (F-keys, a bare Shift). */
export function keyOf(key: string): string | null {
  if ((NAMED_KEYS as readonly string[]).includes(key)) return key;
  return [...key].length === 1 ? key : null;
}

/** The live view's input messages, as the socket takes them. */
export type LiveInput =
  | { type: "move"; x: number; y: number }
  | {
      type: "press";
      x: number;
      y: number;
      press: "down" | "up";
      button: "left" | "middle" | "right";
      clicks: number;
    }
  | { type: "wheel"; x: number; y: number; dx: number; dy: number }
  | { type: "key"; press: "down" | "up"; key: string; modifiers: number }
  | { type: "text"; text: string };

/** A key pressed and let go, for the buttons a phone has no keys for. */
export function tap(key: (typeof NAMED_KEYS)[number]): LiveInput[] {
  return [
    { type: "key", press: "down", key, modifiers: 0 },
    { type: "key", press: "up", key, modifiers: 0 },
  ];
}

/** How far a finger has to move before a touch is a drag, not a tap. */
export const DRAG = 8;

/** A finger's drag as the wheel the page scrolls by: dragging up shows
    what is below, as a phone's own scrolling does. */
export function dragWheel(
  from: { x: number; y: number },
  to: { x: number; y: number },
): { dx: number; dy: number } {
  return { dx: from.x - to.x, dy: from.y - to.y };
}
