import { useState } from "react";
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
  /** Their GitHub avatar, when there is one. */
  avatarUrl?: string;
  onOpenSettings: () => void;
}

/**
 * Everything that is about *you* rather than about the house.
 *
 * A phone app puts settings behind the avatar, not in a tab beside the
 * thing you came to use — the house is the page, and how it looks and
 * who you are are both one press away rather than half the top bar.
 */
export function AccountMenu({ email, avatarUrl, onOpenSettings }: AccountMenuProps) {
  const [theme, setTheme] = useTheme();
  // The picture comes from GitHub, so it can be slow, blocked by a
  // content blocker, or simply gone. Any of those falls back to the
  // letters rather than leaving a hole where the button was.
  const [broken, setBroken] = useState(false);
  const picture = avatarUrl && !broken;

  return (
    <Popover>
      <PopoverTrigger
        aria-label={email ? `Account — ${email}` : "Account and appearance"}
        className={cn(
          "flex size-9 items-center justify-center overflow-hidden rounded-full",
          "text-sm font-medium transition-opacity hover:opacity-90",
          "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          !picture && "bg-primary text-primary-foreground",
        )}
      >
        {picture ? (
          <img
            src={avatarUrl}
            alt=""
            width={36}
            height={36}
            // Nothing about this house travels to GitHub with the
            // request for a picture.
            referrerPolicy="no-referrer"
            className="size-full object-cover"
            onError={() => setBroken(true)}
          />
        ) : (
          initials(email)
        )}
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
 * What to draw when there is no picture.
 *
 * The avatar comes from GitHub, addressed by the numeric account id the
 * session carries — not from Gravatar, which would mean handing a third
 * party the hash of a household member's address. When it is missing,
 * slow or blocked, two letters are better than a hole.
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
