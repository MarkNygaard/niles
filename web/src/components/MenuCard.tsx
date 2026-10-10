import { useEffect, useRef, useState } from "react";
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
import { edgeScroll, useGripsHoldStill } from "@/lib/drag";
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
 * Which spot a gap between the page's rows stands for, gap `n` being
 * the one above row `n`. The rows are, in order: the avatar menu's
 * heading, My profile, its entries, the main navigation's heading,
 * Home, its entries. Above a locked row or a heading is the nearest
 * place beside it; past the main navigation's heading is in it.
 */
export function spotAt(gap: number, avatarLength: number): Spot {
  if (gap <= 2) return { list: "avatar", at: 0 };
  if (gap <= 2 + avatarLength) return { list: "avatar", at: gap - 2 };
  if (gap <= 4 + avatarLength) return { list: "main", at: 0 };
  return { list: "main", at: gap - (4 + avatarLength) };
}

/** The gap a spot is drawn at: where the line goes while dragging. */
export function gapOf(spot: Spot, avatarLength: number): number {
  return spot.list === "avatar" ? 2 + spot.at : 4 + avatarLength + spot.at;
}

/** Put `id` down at `spot`, a place counted with `id` still where it
    was — which is how the page shows it while it is held. */
export function drop(lists: Lists, id: MenuEntry, spot: Spot): Lists {
  const from = lists[spot.list].indexOf(id);
  const at = from !== -1 && from < spot.at ? spot.at - 1 : spot.at;
  return place(lists, id, { list: spot.list, at });
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
 * drag that reorders is also the drag that moves between them. The row
 * follows the pointer and a line shows where it will go; nothing moves
 * until it is let go. (Moving rows under the pointer moved the held row
 * in the page too, and a browser lets go of a pointer whose element
 * moves: every drag stopped after one step.)
 */
export function MenuCard({ layout, disabled, onChange, mediaShown, onMediaShown }: MenuCardProps) {
  const [held, setHeld] = useState<{
    id: MenuEntry;
    /** Where the press began, in the page rather than the window, so
        the row stays under the pointer while the page scrolls. */
    from: number;
    y: number;
    scroll: number;
  } | null>(null);
  const [gap, setGap] = useState<number | null>(null);
  const list = useRef<HTMLUListElement | null>(null);
  const pointer = useRef(0);
  useGripsHoldStill();

  const lists: Lists = {
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

  /** The gap nearest a pointer at `clientY`. Every row is one height. */
  function gapAt(clientY: number): number {
    const box = list.current?.getBoundingClientRect();
    if (!box || box.height === 0 || rows.length === 0) return 0;
    const index = Math.round((clientY - box.top) / (box.height / rows.length));
    return Math.min(Math.max(index, 0), rows.length);
  }

  // Where the held row would go, and whether that is anywhere new.
  const target = held && gap !== null ? spotAt(gap, lists.avatar.length) : null;
  const next = held && target ? drop(lists, held.id, target) : null;
  const moves = next !== null && JSON.stringify(next) !== JSON.stringify(lists);

  // The page moves under a drag only at the screen's edges.
  const dragging = held !== null;
  useEffect(() => {
    if (!dragging) return;
    // Not for a drag begun at the edge: only once it has been away.
    let armed = false;
    let frame = requestAnimationFrame(function tick() {
      const y = pointer.current;
      const by = edgeScroll(y, window.innerHeight);
      if (by === 0) armed = true;
      if (by !== 0 && armed) {
        window.scrollBy(0, by);
        const scroll = window.scrollY;
        setHeld((now) => now && { ...now, scroll });
        setGap(gapAt(y));
      }
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
    // gapAt reads the list as it is drawn at the time, so the loop
    // needs no restart when the rows change.
  }, [dragging]);

  function letGo(keep: boolean) {
    if (keep && moves && next) save(next);
    setHeld(null);
    setGap(null);
  }

  return (
    <ul ref={list} className="relative flex flex-col">
      {held && target && moves && (
        <li
          aria-hidden
          className="bg-foreground pointer-events-none absolute inset-x-1 z-20 h-0.5 -translate-y-1/2 rounded-full"
          style={{ top: `${(gapOf(target, lists.avatar.length) / rows.length) * 100}%` }}
        />
      )}
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
        const lifted = held?.id === id;
        return (
          <li
            key={id}
            className={cn("flex h-12 items-center py-0.5", lifted && "relative z-10")}
            style={
              lifted
                ? { transform: `translateY(${held.y + held.scroll - held.from}px)` }
                : undefined
            }
          >
            <div
              className={cn(
                "bg-card flex h-full w-full items-center gap-3 rounded-lg border px-3",
                lifted && "border-foreground/30 shadow-lg",
              )}
            >
              <button
                type="button"
                disabled={disabled}
                aria-label={`Move ${destination.label}`}
                data-grip
                className={cn(
                  "text-muted-foreground hover:text-foreground -m-1 cursor-grab touch-none p-1 select-none [-webkit-touch-callout:none]",
                  "focus-visible:ring-3 focus-visible:ring-ring/50 rounded focus-visible:outline-none",
                  "disabled:cursor-not-allowed disabled:opacity-50",
                )}
                onPointerDown={(e) => {
                  // No text selection, which would scroll the page with
                  // the mouse.
                  e.preventDefault();
                  e.currentTarget.setPointerCapture(e.pointerId);
                  pointer.current = e.clientY;
                  setHeld({
                    id,
                    from: e.clientY + window.scrollY,
                    y: e.clientY,
                    scroll: window.scrollY,
                  });
                }}
                onPointerMove={(e) => {
                  if (held?.id !== id) return;
                  const y = e.clientY;
                  pointer.current = y;
                  setHeld({ ...held, y, scroll: window.scrollY });
                  setGap(gapAt(y));
                }}
                onPointerUp={() => letGo(true)}
                onPointerCancel={() => letGo(false)}
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
