import { DESTINATIONS, HOME } from "@/lib/destinations";
import type { Destination } from "@/lib/destinations";
import { MOVABLE } from "@/lib/menu";
import type { MenuItem } from "@/lib/menu";
import { cn } from "@/lib/utils";

export interface TabBarProps {
  /** The current route, from `useRoute`. */
  route: string;
  /** The main navigation after Home, from `menuLayout`. */
  menu?: MenuItem[];
}

const UNARRANGED: MenuItem[] = MOVABLE.filter((id) => id !== "settings").map((id) => ({
  id,
  hidden: false,
}));

/**
 * The main navigation, at the bottom of a phone where a thumb is.
 *
 * From `sm` the same links sit in the header instead: at the bottom of a
 * desktop window they would be a long way from everything else. Home
 * comes first; the rest is what Settings → Menu put here. You — your
 * profile, the look of the app — are in the avatar menu in the corner.
 */
export function TabBar({ route, menu = UNARRANGED }: TabBarProps) {
  const tabs: Destination[] = [
    HOME,
    ...menu.filter((item) => !item.hidden).map((item) => DESTINATIONS[item.id]),
  ];

  return (
    <nav
      aria-label="Main"
      className={cn(
        "border-border bg-background/90 fixed inset-x-0 bottom-0 z-40 border-t backdrop-blur",
        "pb-[env(safe-area-inset-bottom)]",
        "sm:static sm:z-auto sm:border-0 sm:bg-transparent sm:pb-0 sm:backdrop-blur-none",
        // Out of the way while somebody types in the chat: on a phone
        // the keyboard already takes half the screen.
        "max-sm:in-data-typing:hidden",
      )}
    >
      <ul className="mx-auto flex max-w-md sm:max-w-none sm:gap-1">
        {tabs.map((tab) => {
          const active = tab.owns(route);
          return (
            <li key={tab.href} className="flex-1 sm:flex-none">
              <a
                href={tab.href}
                aria-current={active ? "page" : undefined}
                className={cn(
                  // Held 8px off the bar's top edge rather than centred:
                  // centred, the icons sat right against it. The bar keeps
                  // its height; the chat and the page padding count on it.
                  "flex h-12 flex-col items-center justify-start gap-0.5 pt-2 text-[11px] font-medium transition-colors",
                  "sm:h-9 sm:flex-row sm:justify-center sm:gap-2 sm:rounded-full sm:px-3 sm:pt-0 sm:text-sm",
                  "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                  "[&_svg]:size-6 sm:[&_svg]:size-4",
                  active
                    ? "text-foreground sm:bg-muted"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {tab.icon}
                {tab.label}
              </a>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
