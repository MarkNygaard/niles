import { useState } from "react";
import { LogOut, Monitor, Moon, Sun, UserRound } from "lucide-react";
import { Avatar } from "@/components/Avatar";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { DESTINATIONS, PROFILE } from "@/lib/destinations";
import type { Destination } from "@/lib/destinations";
import type { MenuItem } from "@/lib/menu";
import type { Theme } from "@/lib/theme";
import { cn } from "@/lib/utils";

export interface AvatarMenuProps {
  /** The signed-in address, or undefined when sign-in is off. */
  email?: string;
  avatarUrl?: string;
  /** What Settings → Menu put here, after My profile. */
  items: MenuItem[];
  theme: Theme;
  onTheme: (theme: Theme) => void;
}

/**
 * The avatar in the top corner, and what is about you behind it: your
 * profile, the entries Settings → Menu placed here, the look of the
 * app, and signing out.
 *
 * Back in the corner because the bar along the bottom filled up: every
 * page given a place there took one from the rest, and what you open
 * now and then — Settings, your profile — has no need of a thumb's
 * reach.
 */
export function AvatarMenu({ email, avatarUrl, items, theme, onTheme }: AvatarMenuProps) {
  const [open, setOpen] = useState(false);
  const entries: Destination[] = [
    // A profile needs a you, so not with sign-in off.
    ...(email ? [PROFILE] : []),
    ...items.filter((item) => !item.hidden).map((item) => DESTINATIONS[item.id]),
  ];

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        aria-label="Your menu"
        className={cn(
          "flex size-9 shrink-0 items-center justify-center rounded-full",
          "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          !email && "text-muted-foreground hover:text-foreground",
        )}
      >
        {email ? (
          <Avatar email={email} avatarUrl={avatarUrl} className="size-8 text-xs" />
        ) : (
          <UserRound className="size-5" />
        )}
      </PopoverTrigger>
      <PopoverContent align="end" className="flex w-64 flex-col gap-2 p-2">
        {email && (
          <div className="text-muted-foreground truncate px-2 pt-1 text-xs">{email}</div>
        )}
        <nav aria-label="Your menu" className="flex flex-col">
          {entries.map((entry) => (
            <a
              key={entry.href}
              href={entry.href}
              onClick={() => setOpen(false)}
              className={cn(
                "flex items-center gap-3 rounded-md px-2 py-2 text-sm transition-colors",
                "hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
                "[&>svg]:text-muted-foreground [&>svg]:size-4",
              )}
            >
              {entry.icon}
              {entry.label}
            </a>
          ))}
        </nav>
        <div className="bg-muted flex gap-1 rounded-lg p-1" role="group" aria-label="Appearance">
          <Appearance current={theme} value="light" icon={<Sun />} onPick={onTheme}>
            Light
          </Appearance>
          <Appearance current={theme} value="dark" icon={<Moon />} onPick={onTheme}>
            Dark
          </Appearance>
          <Appearance current={theme} value="system" icon={<Monitor />} onPick={onTheme}>
            Auto
          </Appearance>
        </div>
        {/* A link, not a fetch: signing out clears a cookie on a
            response the browser has to follow. */}
        {email && (
          <a
            href="/auth/signout"
            className={cn(
              "flex items-center gap-3 rounded-md px-2 py-2 text-sm transition-colors",
              "hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
              "[&>svg]:text-muted-foreground [&>svg]:size-4",
            )}
          >
            <LogOut />
            Sign out
          </a>
        )}
      </PopoverContent>
    </Popover>
  );
}

function Appearance({
  current,
  value,
  icon,
  children,
  onPick,
}: {
  current: Theme;
  value: Theme;
  icon: React.ReactNode;
  children: React.ReactNode;
  onPick: (theme: Theme) => void;
}) {
  const active = current === value;
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={() => onPick(value)}
      className={cn(
        "flex flex-1 flex-col items-center gap-1 rounded-md px-1 py-1.5 text-xs transition-colors",
        "focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
        "[&_svg]:size-4",
        active
          ? "bg-background text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground",
      )}
    >
      {icon}
      {children}
    </button>
  );
}
