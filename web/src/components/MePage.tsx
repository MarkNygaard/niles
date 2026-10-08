import { Check, ChevronRight, LogOut, Monitor, Moon, SlidersHorizontal, Sun, UserRound } from "lucide-react";
import { Avatar } from "@/components/Avatar";
import type { Theme } from "@/lib/theme";
import { cn } from "@/lib/utils";

export interface MePageProps {
  /** The signed-in address, or undefined when sign-in is off. */
  email?: string;
  avatarUrl?: string;
  theme: Theme;
  onTheme: (theme: Theme) => void;
}

/**
 * Everything that is about *you* rather than about the house.
 *
 * A page rather than the dropdown it replaces: a menu hanging off an
 * avatar is a desktop habit, and on a phone the tab bar already says
 * where "you" is.
 */
export function MePage({ email, avatarUrl, theme, onTheme }: MePageProps) {
  return (
    <div className="flex flex-col gap-5">
      {email && (
        <div className="flex items-center gap-3 px-1">
          <Avatar email={email} avatarUrl={avatarUrl} className="size-14 text-lg" />
          <div className="min-w-0">
            <div className="text-muted-foreground text-xs">Signed in as</div>
            <div className="truncate text-sm">{email}</div>
          </div>
        </div>
      )}

      <Group>
        {/* First, because it is the one everybody has a use for. A page
            about you needs a you, so not with sign-in off. */}
        {email && (
          <Row href="#/me/profile" icon={<UserRound />} hint="Your birthday, notes and phone">
            My profile
          </Row>
        )}
        <Row
          href="#/me/settings"
          icon={<SlidersHorizontal />}
          hint="The house: lights, rooms, voices, integrations"
        >
          Settings
        </Row>
      </Group>

      <div>
        <div className="text-muted-foreground mb-1 px-1 text-xs font-medium tracking-wide uppercase">
          Appearance
        </div>
        <div className="bg-card flex gap-1 rounded-xl p-1">
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
      </div>

      {/* A link, not a fetch: signing out clears a cookie on a response
          the browser has to follow. */}
      {email && (
        <Group>
          <Row href="/auth/signout" icon={<LogOut />}>
            Sign out
          </Row>
        </Group>
      )}
    </div>
  );
}

function Group({ children }: { children: React.ReactNode }) {
  return <div className="bg-card overflow-hidden rounded-xl">{children}</div>;
}

function Row({
  href,
  icon,
  hint,
  children,
}: {
  href: string;
  icon: React.ReactNode;
  hint?: string;
  children: React.ReactNode;
}) {
  // Only rows that open a page inside the app get a chevron; signing out
  // leaves it.
  const inside = href.startsWith("#");
  return (
    <a
      href={href}
      className={cn(
        "border-border/60 flex w-full items-center gap-3 border-b px-3 py-2.5 transition-colors last:border-b-0",
        "hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
        "[&>svg]:text-muted-foreground [&>svg]:size-4 [&>svg]:shrink-0",
      )}
    >
      {icon}
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{children}</span>
        {hint && <span className="text-muted-foreground block truncate text-xs">{hint}</span>}
      </span>
      {inside && <ChevronRight aria-hidden />}
    </a>
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
        "flex flex-1 flex-col items-center gap-1 rounded-lg px-1 py-2 text-xs transition-colors",
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
