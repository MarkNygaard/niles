import { valueAt } from "@/lib/api";

/** The menu entries that can be moved. Home and My profile are not:
    Home is the main navigation's first entry, My profile the avatar
    menu's. Settings moves but does not hide — it is the way back here. */
export type MenuEntry = "groceries" | "chat" | "media" | "settings";

/** Every movable entry, in the order the main navigation shows them
    before anybody arranges it. */
export const MOVABLE: MenuEntry[] = ["groceries", "chat", "media", "settings"];

/** Where Settings sits before anybody moves it. */
const AVATAR_AT_FIRST: MenuEntry[] = ["settings"];

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

/** The two menus: the avatar menu's entries after My profile, and the
    main navigation's after Home. */
export interface MenuLayout {
  avatar: MenuItem[];
  main: MenuItem[];
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((v) => typeof v === "string") : [];
}

function isEntry(value: string): value is MenuEntry {
  return (MOVABLE as string[]).includes(value);
}

/**
 * Both menus, as `[menu]` arranges them.
 *
 * Read by the menus that obey it and the settings page that writes it,
 * so they agree on what a missing or odd entry means. An entry neither
 * list names goes to the main navigation, after the arranged ones — a
 * page added in a later version shows up rather than not at all.
 *
 * `playing` is whether anything plays now, for Media's "when something
 * plays"; Settings leaves it out, and lists Media either way.
 */
export function menuLayout(effective: unknown, playing?: boolean): MenuLayout {
  const hidden = new Set(strings(valueAt(effective, "menu.hidden")));
  const written = valueAt(effective, "menu.avatar");
  const avatarIds = [
    ...new Set(Array.isArray(written) ? strings(written).filter(isEntry) : AVATAR_AT_FIRST),
  ];
  const arranged = strings(valueAt(effective, "menu.order")).filter(isEntry);
  const mainIds = [...new Set([...arranged, ...MOVABLE])].filter(
    (id) => !avatarIds.includes(id),
  );
  const media = mediaShownOf(effective);
  const item = (id: MenuEntry): MenuItem => ({
    id,
    hidden:
      id === "settings"
        ? false
        : id === "media"
          ? media === "never" || (media === "playing" && playing === false)
          : hidden.has(id),
  });
  return { avatar: avatarIds.map(item), main: mainIds.map(item) };
}
