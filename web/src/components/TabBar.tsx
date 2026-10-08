import { House, UserRound } from "lucide-react";
import { Avatar } from "@/components/Avatar";
import { cn } from "@/lib/utils";

export interface TabBarProps {
  /** The current route, from `useRoute`. */
  route: string;
  email?: string;
  avatarUrl?: string;
}

interface Tab {
  href: string;
  label: string;
  icon: React.ReactNode;
  /** The routes this tab is the way back to, its own sub-pages included. */
  owns: (route: string) => boolean;
}

/**
 * The app's few destinations, at the bottom of a phone where a thumb is.
 *
 * From `sm` the same links sit in the header instead: at the bottom of a
 * desktop window they would be a long way from everything else.
 */
export function TabBar({ route, email, avatarUrl }: TabBarProps) {
  const tabs: Tab[] = [
    {
      href: "#/",
      label: "Home",
      icon: <House />,
      owns: (r) => r === "/",
    },
    {
      href: "#/me",
      label: "Me",
      // A face rather than a silhouette once there is one: it is how a
      // phone app says "this one is yours".
      icon: email ? (
        <Avatar email={email} avatarUrl={avatarUrl} className="size-6 text-[10px] sm:size-5" />
      ) : (
        <UserRound />
      ),
      owns: (r) => r === "/me" || r.startsWith("/me/"),
    },
  ];

  return (
    <nav
      aria-label="Main"
      className={cn(
        "border-border bg-background/90 fixed inset-x-0 bottom-0 z-40 border-t backdrop-blur",
        "pb-[env(safe-area-inset-bottom)]",
        "sm:static sm:z-auto sm:border-0 sm:bg-transparent sm:pb-0 sm:backdrop-blur-none",
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
                  "flex h-14 flex-col items-center justify-center gap-0.5 text-[11px] font-medium transition-colors",
                  "sm:h-9 sm:flex-row sm:gap-2 sm:rounded-full sm:px-3 sm:text-sm",
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
