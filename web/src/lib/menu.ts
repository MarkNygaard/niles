import { valueAt } from "@/lib/api";

/** The menu entries that can be moved and hidden. Home and Me are not:
    Home is where the app opens, and Me is the way to the setting that
    would bring a hidden entry back. */
export type MenuEntry = "groceries" | "chat" | "media";

/** The order they come in before anybody arranges them. */
export const MOVABLE: MenuEntry[] = ["groceries", "chat", "media"];

/** When the Media entry is in the menu. */
export type MediaShown = "playing" | "always" | "never";

/** `[menu] media`, with anything else read as the default. */
export function mediaShownOf(effective: unknown): MediaShown {
  const value = valueAt(effective, "menu.media");
  return value === "always" || value === "never" ? value : "playing";
}

export interface MenuItem {
  id: MenuEntry;
  hidden: boolean;
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((v) => typeof v === "string") : [];
}

function isEntry(value: string): value is MenuEntry {
  return (MOVABLE as string[]).includes(value);
}

/**
 * The middle of the menu, as `[menu]` arranges it.
 *
 * Read by the menu that obeys it and the settings page that writes it,
 * so they agree on what a missing or odd entry means. An entry the order
 * does not name follows the ones it does — a page added in a later
 * version shows up rather than not at all. Only `hidden` hides.
 */
export function menuOf(effective: unknown, playing?: boolean): MenuItem[] {
  const hidden = new Set(strings(valueAt(effective, "menu.hidden")));
  const arranged = strings(valueAt(effective, "menu.order")).filter(isEntry);
  const order = [...new Set([...arranged, ...MOVABLE])];
  const media = mediaShownOf(effective);
  return order.map((id) => ({
    id,
    // Media follows its own setting; `playing` is only known to the menu
    // itself, and Settings lists the entry either way.
    hidden:
      id === "media"
        ? media === "never" || (media === "playing" && playing === false)
        : hidden.has(id),
  }));
}
