import {
  AudioLines,
  Brain,
  ChevronRight,
  Clock,
  House,
  KeyRound,
  LayoutGrid,
  Lightbulb,
  Menu,
  Mic,
  Network,
  Puzzle,
  Speaker,
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
 * file is laid out.
 *
 * The house is what is in it. System is what Niles needs to work at
 * all — the broker its lights are reached through, and who hears,
 * thinks and speaks — set up by the job each does. Add-ons are the
 * optional extras, Niles's own and other people's, in one list.
 */
export const SECTIONS: { group: string; items: Section[] }[] = [
  {
    group: "The house",
    items: [
      {
        id: "home",
        label: "Home",
        hint: "Where the house is, and what clock it keeps",
        icon: House,
      },
      {
        id: "rooms",
        label: "Rooms",
        hint: "Which the front page shows, and in what order",
        icon: LayoutGrid,
      },
      {
        id: "menu",
        label: "Menu",
        hint: "Which pages the menu leads to, and in what order",
        icon: Menu,
      },
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
        id: "memory",
        label: "Memory",
        hint: "What Niles remembers between conversations",
        icon: Brain,
      },
      {
        id: "satellites",
        label: "Satellites",
        hint: "The microphones, and which room each is in",
        icon: Mic,
      },
      {
        id: "speakers",
        label: "Speakers",
        hint: "The Sonos, and which room each plays in",
        icon: Speaker,
      },
    ],
  },
  {
    group: "System",
    items: [
      {
        id: "devices",
        label: "Devices",
        hint: "The broker your Zigbee lights and WLED strips are reached through",
        icon: Network,
      },
      {
        id: "voice",
        label: "Voice & AI",
        hint: "Who hears, who thinks, who speaks, and the assistant",
        icon: AudioLines,
      },
    ],
  },
  {
    group: "Add-ons",
    items: [
      {
        id: "addons",
        label: "Add-ons",
        hint: "Music, presence, calendars and more, and add-ons from GitHub",
        icon: Puzzle,
      },
    ],
  },
  {
    group: "Advanced",
    items: [
      {
        id: "history",
        label: "History",
        hint: "What changed, and undoing it",
        icon: Clock,
      },
      {
        id: "api",
        label: "API access",
        hint: "A key for scripts and other systems",
        icon: KeyRound,
      },
    ],
  },
];

/**
 * Where a section that has moved lives now.
 *
 * "Integrations" became System (the broker, the voice providers) and
 * Add-ons; "Speech & language" became Voice & AI. A link or bookmark to
 * the old id — `#/me/settings/integrations` — lands where its contents
 * went rather than on a blank pane.
 */
const MOVED: Record<string, string> = {
  integrations: "addons",
  language: "voice",
};

/** The section an id names, after any move; `null` for none at all. */
export function sectionId(id: string | null | undefined): string | null {
  if (!id) return null;
  const moved = MOVED[id] ?? id;
  return SECTIONS.some((group) => group.items.some((item) => item.id === moved))
    ? moved
    : null;
}

export interface SettingsNavProps {
  current: string | null;
  onPick: (id: string) => void;
  /** How many things each section still has to set up, by its id. */
  todo?: Record<string, number>;
  /** What a count is of, for a screen reader: "to set up", "waiting". */
  counting?: string;
  /** The groups to list. Settings's own unless My profile lends its. */
  sections?: { group: string; items: Section[] }[];
}

/**
 * The way into a section.
 *
 * A list rather than a tab bar, because there are now more sections
 * than a phone can show as tabs without shrinking them to initials —
 * and a list can carry a line saying what each one is, which a tab
 * cannot. On a phone it is the whole screen and tapping pushes in; from
 * `sm` it sits beside the section it opened, which is the same shape
 * once there is room for both. My profile is laid out by the same list,
 * so the two pages behind the avatar work alike.
 */
export function SettingsNav({
  current,
  onPick,
  todo = {},
  counting = "to set up",
  sections = SECTIONS,
}: SettingsNavProps) {
  return (
    <nav className="flex w-full shrink-0 flex-col gap-5 sm:w-64">
      {sections.map((group) => (
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
                  {/* A new install sees where to start without opening
                      every section to find out. */}
                  {(todo[item.id] ?? 0) > 0 && (
                    <span className="bg-destructive/10 text-destructive flex size-5 shrink-0 items-center justify-center rounded-full text-xs font-medium tabular-nums">
                      {todo[item.id]}
                      <span className="sr-only"> {counting}</span>
                    </span>
                  )}
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
  const moved = sectionId(id);
  for (const group of SECTIONS) {
    const found = group.items.find((item) => item.id === moved);
    if (found) return found.label;
  }
  return "Settings";
}
