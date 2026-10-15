import { valueAt } from "@/lib/api";

/** The built-in menu entries that can be moved. Home and My profile
    are not: Home is the main navigation's first entry, My profile the
    avatar menu's. Settings moves but does not hide — it is the way back
    here. */
export type BuiltinEntry =
  | "groceries"
  | "chat"
  | "media"
  | "settings"
  | "automations"
  | "skills";

/** A built-in entry, or a plugin's page as `<plugin-id>.<page-id>`
    (`spotprice.electricity`), which moves and hides like the others. */
export type MenuEntry = BuiltinEntry | `${string}.${string}`;

/** Every movable built-in entry, in the order the main navigation shows
    them before anybody arranges it. */
export const MOVABLE: BuiltinEntry[] = [
  "groceries",
  "chat",
  "media",
  "settings",
  "automations",
  "skills",
];

/** Where Settings sits before anybody moves it. */
const AVATAR_AT_FIRST: BuiltinEntry[] = ["settings"];

/** Entries that go to the end of the avatar menu, not the main
    navigation, while neither list names them. Automations is opened
    now and then, to look or to fix one, and Skills to see what Niles
    has learned: neither has a claim on a tab, and a menu arranged
    before they existed should not gain one. */
export const AVATAR_UNLESS_PLACED: BuiltinEntry[] = ["automations", "skills"];

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

export function isBuiltin(value: string): value is BuiltinEntry {
  return (MOVABLE as string[]).includes(value);
}

/**
 * Both menus, as `[menu]` arranges them.
 *
 * Read by the menus that obey it and the settings page that writes it,
 * so they agree on what a missing or odd entry means. An entry neither
 * list names goes to the main navigation, after the arranged ones — a
 * page added in a later version shows up rather than not at all — or,
 * for the few in `AVATAR_UNLESS_PLACED`, to the end of the avatar menu.
 *
 * `playing` is whether anything plays now, for Media's "when something
 * plays"; Settings leaves it out, and lists Media either way.
 *
 * `pages` are the installed plugins' pages (`spotprice.electricity`).
 * They join like a page added in a later version; an entry naming a
 * plugin that is not installed, or is off, is left out.
 */
export function menuLayout(
  effective: unknown,
  playing?: boolean,
  pages: string[] = [],
): MenuLayout {
  const isEntry = (value: string): value is MenuEntry =>
    isBuiltin(value) || pages.includes(value);
  const hidden = new Set(strings(valueAt(effective, "menu.hidden")));
  const written = valueAt(effective, "menu.avatar");
  const placed = [
    ...new Set(Array.isArray(written) ? strings(written).filter(isEntry) : AVATAR_AT_FIRST),
  ];
  const arranged = strings(valueAt(effective, "menu.order")).filter(isEntry);
  const avatarIds = [
    ...placed,
    ...AVATAR_UNLESS_PLACED.filter(
      (id) => !placed.includes(id) && !arranged.includes(id),
    ),
  ];
  const mainIds = [...new Set<MenuEntry>([...arranged, ...MOVABLE, ...pages.filter(isEntry)])].filter(
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
