import {
  House,
  MessageCircle,
  Music,
  ShoppingBasket,
  SlidersHorizontal,
  Sparkles,
  UserRound,
  Workflow,
} from "lucide-react";
import { PluginIcon } from "@/components/PluginIcon";
import type { BuiltinEntry, MenuEntry } from "@/lib/menu";
import { pageHref } from "@/lib/plugins";
import type { PluginPageEntry } from "@/lib/plugins";

/** A page the menus lead to. */
export interface Destination {
  href: string;
  label: string;
  icon: React.ReactNode;
  /** The routes this is the way back to, its own sub-pages included. */
  owns: (route: string) => boolean;
}

/** The two that stay put: Home first in the main navigation, My
    profile first in the avatar menu. */
export const HOME: Destination = {
  href: "#/",
  label: "Home",
  icon: <House />,
  owns: (r) => r === "/",
};

export const PROFILE: Destination = {
  href: "#/me/profile",
  label: "My profile",
  icon: <UserRound />,
  owns: (r) => r === "/me/profile",
};

/** The ones that move, wherever they are put. One table for both menus
    and the settings page, so an entry looks the same in all three. */
export const DESTINATIONS: Record<BuiltinEntry, Destination> = {
  groceries: {
    href: "#/groceries",
    label: "Groceries",
    icon: <ShoppingBasket />,
    owns: (r) => r === "/groceries",
  },
  chat: {
    href: "#/chat",
    label: "Chat",
    icon: <MessageCircle />,
    owns: (r) => r === "/chat",
  },
  media: {
    href: "#/media",
    label: "Media",
    icon: <Music />,
    owns: (r) => r === "/media",
  },
  settings: {
    href: "#/me/settings",
    label: "Settings",
    icon: <SlidersHorizontal />,
    owns: (r) => r === "/me/settings" || r.startsWith("/me/settings/"),
  },
  automations: {
    href: "#/automations",
    label: "Automations",
    icon: <Workflow />,
    owns: (r) => r === "/automations" || r.startsWith("/automations/"),
  },
  skills: {
    href: "#/skills",
    label: "Skills",
    icon: <Sparkles />,
    owns: (r) => r === "/skills",
  },
};

/**
 * Where an entry leads: a built-in page, or a plugin's page among
 * `pages`. `undefined` for a plugin page that is not installed, which
 * the menus leave out.
 */
export function destinationOf(
  id: MenuEntry,
  pages: PluginPageEntry[] = [],
): Destination | undefined {
  if (Object.hasOwn(DESTINATIONS, id)) return DESTINATIONS[id as BuiltinEntry];
  const page = pages.find((p) => p.id === id);
  if (!page) return undefined;
  const route = pageHref(page).slice(1);
  return {
    href: pageHref(page),
    label: page.label,
    icon: <PluginIcon name={page.icon} />,
    owns: (r) => r === route,
  };
}
