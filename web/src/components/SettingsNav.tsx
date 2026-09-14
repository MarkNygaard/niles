import {
  ChevronRight,
  Clock,
  FileCode,
  House,
  KeyRound,
  Lightbulb,
  MessagesSquare,
  Plug,
  Users,
} from "lucide-react";
import { cn } from "@/lib/utils";

export interface Section {
  id: string;
  label: string;
  hint: string;
  icon: typeof Lightbulb;
}

/**
 * Grouped the way somebody looks for a setting, not the way the config
 * file is laid out. "Everything else" is last and honest about being a
 * dump: it is the one section that exists because a config has more in
 * it than a page should pretend to curate.
 */
export const SECTIONS: { group: string; items: Section[] }[] = [
  {
    group: "The house",
    items: [
      {
        id: "lighting",
        label: "Lighting",
        hint: "The daily curve, and which lights sit it out",
        icon: Lightbulb,
      },
      {
        id: "people",
        label: "People",
        hint: "Who can sign in",
        icon: Users,
      },
      {
        id: "home",
        label: "Home",
        hint: "Where the house is, and what clock it keeps",
        icon: House,
      },
    ],
  },
  {
    group: "Connections",
    items: [
      {
        id: "integrations",
        label: "Integrations",
        hint: "The services Niles reads from",
        icon: Plug,
      },
      {
        id: "language",
        label: "Speech & language",
        hint: "Who transcribes, who answers, and with what model",
        icon: MessagesSquare,
      },
      {
        id: "credentials",
        label: "Credentials",
        hint: "Keys and passwords, kept encrypted",
        icon: KeyRound,
      },
    ],
  },
  {
    group: "Advanced",
    items: [
      {
        id: "all",
        label: "Raw config",
        hint: "Every section, including ones with no page yet",
        icon: FileCode,
      },
      {
        id: "history",
        label: "History",
        hint: "What changed, and undoing it",
        icon: Clock,
      },
    ],
  },
];

export interface SettingsNavProps {
  current: string | null;
  onPick: (id: string) => void;
}

/**
 * The way into a section.
 *
 * A list rather than a tab bar, because there are now more sections
 * than a phone can show as tabs without shrinking them to initials —
 * and a list can carry a line saying what each one is, which a tab
 * cannot. On a phone it is the whole screen and tapping pushes in; from
 * `sm` it sits beside the section it opened, which is the same shape
 * once there is room for both.
 */
export function SettingsNav({ current, onPick }: SettingsNavProps) {
  return (
    <nav className="flex w-full shrink-0 flex-col gap-5 sm:w-64">
      {SECTIONS.map((group) => (
        <div key={group.group}>
          <div className="text-muted-foreground mb-1 px-1 text-xs font-medium tracking-wide uppercase">
            {group.group}
          </div>
          <div className="bg-card overflow-hidden rounded-xl">
            {group.items.map((item) => {
              const Icon = item.icon;
              const active = current === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  aria-current={active ? "page" : undefined}
                  onClick={() => onPick(item.id)}
                  className={cn(
                    "border-border/60 flex w-full items-center gap-3 border-b px-3 py-2.5 text-left transition-colors last:border-b-0",
                    "hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
                    active && "bg-muted/60",
                  )}
                >
                  <Icon
                    aria-hidden
                    className={cn(
                      "size-4 shrink-0",
                      active ? "text-foreground" : "text-muted-foreground",
                    )}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">
                      {item.label}
                    </span>
                    <span className="text-muted-foreground block truncate text-xs">
                      {item.hint}
                    </span>
                  </span>
                  {/* Only on a phone, where tapping goes somewhere. Beside
                      the section it opened, it would point at itself. */}
                  <ChevronRight
                    aria-hidden
                    className="text-muted-foreground size-4 shrink-0 sm:hidden"
                  />
                </button>
              );
            })}
          </div>
        </div>
      ))}
    </nav>
  );
}

/** What a section is called, for the header above it. */
export function labelOf(id: string): string {
  for (const group of SECTIONS) {
    const found = group.items.find((item) => item.id === id);
    if (found) return found.label;
  }
  return "Settings";
}
