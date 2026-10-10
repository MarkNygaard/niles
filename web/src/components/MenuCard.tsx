import { useRef, useState } from "react";
import { GripVertical, Lock } from "lucide-react";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { DESTINATIONS, HOME, PROFILE } from "@/lib/destinations";
import type { MediaShown, MenuEntry, MenuLayout } from "@/lib/menu";
import { cn } from "@/lib/utils";

/** The two menus as lists of entries, avatar menu first. */
export interface Lists {
  avatar: MenuEntry[];
  main: MenuEntry[];
}

/** Where an entry is put: a menu, and a place in it. */
export interface Spot {
  list: keyof Lists;
  at: number;
}

/** Take `id` out of wherever it is and put it at `spot`. */
export function place(lists: Lists, id: MenuEntry, spot: Spot): Lists {
  const next: Lists = {
    avatar: lists.avatar.filter((e) => e !== id),
    main: lists.main.filter((e) => e !== id),
  };
  const target = next[spot.list];
  target.splice(Math.min(Math.max(spot.at, 0), target.length), 0, id);
  return next;
}

/**
 * Which spot a row of the page stands for. The rows are, in order: the
 * avatar menu's heading, My profile, its entries, the main navigation's
 * heading, Home, its entries. Above an entry is that entry's place;
 * a heading or a locked row is the nearest place beside it.
 */
export function spotAt(row: number, avatarLength: number): Spot {
  if (row <= 1) return { list: "avatar", at: 0 };
  if (row < 2 + avatarLength) return { list: "avatar", at: row - 2 };
  if (row === 2 + avatarLength) return { list: "avatar", at: avatarLength };
  if (row === 3 + avatarLength) return { list: "main", at: 0 };
  return { list: "main", at: row - (4 + avatarLength) };
}

/** One step up or down for the arrow keys: past the top of the main
    navigation is the end of the avatar menu, and back. */
export function step(lists: Lists, id: MenuEntry, by: -1 | 1): Lists {
  const list: keyof Lists = lists.avatar.includes(id) ? "avatar" : "main";
  const at = lists[list].indexOf(id);
  const to = at + by;
  if (to >= 0 && to < lists[list].length) return place(lists, id, { list, at: to });
  if (list === "main" && by === -1) {
    return place(lists, id, { list: "avatar", at: lists.avatar.length });
  }
  if (list === "avatar" && by === 1) return place(lists, id, { list: "main", at: 0 });
  return lists;
}

/** What the Media row offers, in the order it offers them. */
export const MEDIA_SHOWN: { value: MediaShown; label: string }[] = [
  { value: "playing", label: "When something plays" },
  { value: "always", label: "Always" },
  { value: "never", label: "Never" },
];

export interface MenuCardProps {
  layout: MenuLayout;
  disabled?: boolean;
  /** Both menus and what is hidden, once anything has changed. */
  onChange: (next: Lists & { hidden: MenuEntry[] }) => void;
  mediaShown: MediaShown;
  onMediaShown: (shown: MediaShown) => void;
}

type Row =
  | { kind: "heading"; label: string }
  | { kind: "locked"; label: string; icon: React.ReactNode }
  | { kind: "entry"; id: MenuEntry };

/**
 * The two menus, arranged by dragging: the avatar menu in the corner,
 * and the main navigation along the bottom. A row dragged past the
 * main navigation's heading moves into it, and back; within either, the
 * order is the order they are shown in. Home and My profile stay first
 * in theirs.
 *
 * One list with the headings in it rather than two lists, so the one
 * drag that reorders is also the drag that moves between them.
 */
export function MenuCard({ layout, disabled, onChange, mediaShown, onMediaShown }: MenuCardProps) {
  const [draft, setDraft] = useState<Lists | null>(null);
  const [held, setHeld] = useState<MenuEntry | null>(null);
  const list = useRef<HTMLUListElement | null>(null);

  const lists: Lists = draft ?? {
    avatar: layout.avatar.map((i) => i.id),
    main: layout.main.map((i) => i.id),
  };
  const hidden = new Set(
    [...layout.avatar, ...layout.main].filter((i) => i.hidden).map((i) => i.id),
  );
  // Media's hiding is its own setting, not the switch's.
  hidden.delete("media");
  const rows: Row[] = [
    { kind: "heading", label: "Avatar menu" },
    { kind: "locked", label: PROFILE.label, icon: PROFILE.icon },
    ...lists.avatar.map((id): Row => ({ kind: "entry", id })),
    { kind: "heading", label: "Main navigation" },
    { kind: "locked", label: HOME.label, icon: HOME.icon },
    ...lists.main.map((id): Row => ({ kind: "entry", id })),
  ];

  function save(next: Lists, nextHidden = hidden) {
    onChange({ ...next, hidden: [...nextHidden] });
  }

  /** Which row a pointer at `clientY` is over. Every row is one height. */
  function rowAt(clientY: number): number {
    const box = list.current?.getBoundingClientRect();
    if (!box || rows.length === 0) return 0;
    const index = Math.floor((clientY - box.top) / (box.height / rows.length));
    return Math.min(Math.max(index, 0), rows.length - 1);
  }

  return (
    <ul ref={list} className="flex flex-col">
      {rows.map((row) => {
        if (row.kind === "heading") {
          return (
            <li
              key={row.label}
              className="text-muted-foreground flex h-12 items-end px-1 pb-2 text-xs font-medium tracking-wide uppercase"
            >
              {row.label}
            </li>
          );
        }
        if (row.kind === "locked") {
          return (
            <li key={row.label} className="flex h-12 items-center py-0.5">
              <div className="bg-card flex h-full w-full items-center gap-3 rounded-lg border px-3 [&>svg]:size-4">
                <Lock aria-hidden className="text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{row.label}</span>
                <span className="text-muted-foreground text-xs">Stays first</span>
              </div>
            </li>
          );
        }
        const { id } = row;
        const destination = DESTINATIONS[id];
        return (
          <li key={id} className="flex h-12 items-center py-0.5">
            <div
              className={cn(
                "bg-card flex h-full w-full items-center gap-3 rounded-lg border px-3",
                held === id && "border-ring shadow-sm",
              )}
            >
              <button
                type="button"
                disabled={disabled}
                aria-label={`Move ${destination.label}`}
                className={cn(
                  "text-muted-foreground hover:text-foreground -m-1 cursor-grab touch-none p-1",
                  "focus-visible:ring-3 focus-visible:ring-ring/50 rounded focus-visible:outline-none",
                  "disabled:cursor-not-allowed disabled:opacity-50",
                )}
                onPointerDown={(e) => {
                  e.currentTarget.setPointerCapture(e.pointerId);
                  setHeld(id);
                }}
                onPointerMove={(e) => {
                  if (held !== id) return;
                  const spot = spotAt(rowAt(e.clientY), lists.avatar.length);
                  const next = place(lists, id, spot);
                  if (JSON.stringify(next) !== JSON.stringify(lists)) setDraft(next);
                }}
                onPointerUp={() => {
                  setHeld(null);
                  // Nothing to save for a row picked up and put back.
                  if (draft) save(draft);
                  setDraft(null);
                }}
                onKeyDown={(e) => {
                  const by = e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : 0;
                  if (by === 0) return;
                  e.preventDefault();
                  save(step(lists, id, by));
                }}
              >
                <GripVertical aria-hidden className="size-4" />
              </button>
              <span
                className={cn(
                  "min-w-0 flex-1 truncate text-sm font-medium",
                  hidden.has(id) && "text-muted-foreground",
                )}
              >
                {destination.label}
              </span>
              {id === "settings" ? (
                // The way back here: it moves, but it does not hide.
                <span className="text-muted-foreground text-xs">Always shown</span>
              ) : id === "media" ? (
                <Select
                  items={MEDIA_SHOWN}
                  value={mediaShown}
                  disabled={disabled}
                  onValueChange={(next: MediaShown | null) => {
                    if (next) onMediaShown(next);
                  }}
                >
                  <SelectTrigger aria-label="Show Media" className="h-8 w-44">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {MEDIA_SHOWN.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : (
                <Switch
                  aria-label={`Show ${destination.label}`}
                  checked={!hidden.has(id)}
                  disabled={disabled}
                  onCheckedChange={(shown) => {
                    const next = new Set(hidden);
                    if (shown) next.delete(id);
                    else next.add(id);
                    save(lists, next);
                  }}
                />
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
