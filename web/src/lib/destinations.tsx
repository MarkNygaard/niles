import {
  House,
  MessageCircle,
  Music,
  ShoppingBasket,
  SlidersHorizontal,
  UserRound,
} from "lucide-react";
import type { MenuEntry } from "@/lib/menu";

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
export const DESTINATIONS: Record<MenuEntry, Destination> = {
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
    owns: (r) => r === "/me/settings",
  },
};
