import { Check, LogOut, Monitor, Moon, SlidersHorizontal, Sun } from "lucide-react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useTheme } from "@/lib/theme";
import type { Theme } from "@/lib/theme";
import { cn } from "@/lib/utils";

export interface AccountMenuProps {
  /** The signed-in address, or undefined when sign-in is off. */
  email?: string;
  onOpenSettings: () => void;
}

/**
 * Everything that is about *you* rather than about the house.
 *
 * A phone app puts settings behind the avatar, not in a tab beside the
 * thing you came to use — the house is the page, and how it looks and
 * who you are are both one press away rather than half the top bar.
 */
export function AccountMenu({ email, onOpenSettings }: AccountMenuProps) {
  const [theme, setTheme] = useTheme();

  return (
    <Popover>
      <PopoverTrigger
        aria-label={email ? `Account — ${email}` : "Account and appearance"}
        className={cn(
          "bg-primary text-primary-foreground flex size-9 items-center justify-center rounded-full",
          "text-sm font-medium transition-opacity hover:opacity-90",
          "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
        )}
      >
        {initials(email)}
      </PopoverTrigger>

      <PopoverContent align="end" className="w-64 p-0">
        {email && (
          <div className="border-border border-b px-3 py-2.5">
            <div className="text-muted-foreground text-xs">Signed in as</div>
            <div className="truncate text-sm">{email}</div>
          </div>
        )}

        <div className="border-border border-b px-3 py-2.5">
          <div className="text-muted-foreground mb-2 text-xs">Appearance</div>
          <div className="bg-muted/60 flex gap-1 rounded-lg p-1">
            <Appearance current={theme} value="light" icon={<Sun />} onPick={setTheme}>
              Light
            </Appearance>
            <Appearance current={theme} value="dark" icon={<Moon />} onPick={setTheme}>
              Dark
            </Appearance>
            <Appearance
              current={theme}
              value="system"
              icon={<Monitor />}
              onPick={setTheme}
            >
              Auto
            </Appearance>
          </div>
        </div>

        <div className="p-1">
          <MenuItem icon={<SlidersHorizontal />} onClick={onOpenSettings}>
            Settings
          </MenuItem>
          {/* A link, not a fetch: signing out clears a cookie on a
              response the browser has to follow. */}
          {email && (
            <MenuItem icon={<LogOut />} href="/auth/signout">
              Sign out
            </MenuItem>
          )}
        </div>
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
        "flex flex-1 flex-col items-center gap-1 rounded-md px-1 py-1.5 text-[11px] transition-colors",
        "focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
        "[&_svg]:size-4",
        active
          ? "bg-background text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground",
      )}
    >
      {icon}
      {children}
      {active && <Check className="sr-only" />}
    </button>
  );
}

function MenuItem({
  icon,
  children,
  href,
  onClick,
}: {
  icon: React.ReactNode;
  children: React.ReactNode;
  href?: string;
  onClick?: () => void;
}) {
  const className = cn(
    "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors",
    "hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
    "[&_svg]:size-4 [&_svg]:text-muted-foreground",
  );
  return href ? (
    <a href={href} className={className}>
      {icon}
      {children}
    </a>
  ) : (
    <button type="button" onClick={onClick} className={className}>
      {icon}
      {children}
    </button>
  );
}

/**
 * Initials, not a photo.
 *
 * GitHub has an avatar, but fetching it would mean either storing a URL
 * in the session or asking Gravatar — which is handing somebody the
 * hash of a household member's address to look up. Two letters cost
 * nothing and leave the building empty-handed.
 */
export function initials(email?: string): string {
  if (!email) return "·";
  const [local] = email.split("@");
  const parts = local.split(/[._-]+/).filter(Boolean);
  if (parts.length >= 2) {
    return (parts[0][0] + parts[1][0]).toUpperCase();
  }
  return (local.slice(0, 2) || "·").toUpperCase();
}
