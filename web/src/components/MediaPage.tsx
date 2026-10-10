import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, GripVertical, Music, Pause, Play, Radio, Speaker, Tv } from "lucide-react";
import { BrandMark } from "@/components/BrandMark";
import { PowerButton } from "@/components/PowerButton";
import { StartDialog } from "@/components/StartDialog";
import { tvLine, tvOn } from "@/components/TvRow";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Slider } from "@/components/ui/slider";
import { api } from "@/lib/api";
import type { MediaGroup, MediaSpeaker, MediaView, RoomMusic } from "@/lib/api";
import { humanize } from "@/lib/rooms";
import { cn } from "@/lib/utils";

/** Whether anything plays — music, radio, or the TV on — for the menu's
    "when something is playing". */
export function somethingPlays(music?: RoomMusic[], tvOn?: boolean): boolean {
  return Boolean(tvOn) || Boolean(music?.some((m) => m.playing));
}

export type Startable = "radio" | "spotify";

/** Where a speaker can be put down: a group, "not playing", or a card
    with something new to start. Written on the page as `data-drop`. */
export type Drop =
  | { to: "group"; leader: string }
  | { to: "idle" }
  | { to: "start"; source: Startable };

export function dropAt(target: string | null | undefined): Drop | null {
  if (!target) return null;
  if (target === "idle") return { to: "idle" };
  if (target === "start:radio") return { to: "start", source: "radio" };
  if (target === "start:spotify") return { to: "start", source: "spotify" };
  if (target.startsWith("group:")) return { to: "group", leader: target.slice(6) };
  return null;
}

export type Move =
  | { action: "join"; leader: string }
  | { action: "leave" }
  | { action: "start"; source: Startable };

/** What putting `speaker` down on `drop` does — nothing when it is
    already there, or when the drop is the TV, whose sound stays in its
    own room. */
export function moveFor(view: MediaView, speaker: string, drop: Drop): Move | null {
  switch (drop.to) {
    case "group": {
      const group = view.groups.find((g) => g.leader === drop.leader);
      if (!group || group.kind === "tv") return null;
      if (group.speakers.some((s) => s.id === speaker)) return null;
      return { action: "join", leader: drop.leader };
    }
    case "idle":
      return view.idle.some((s) => s.id === speaker) ? null : { action: "leave" };
    case "start":
      return { action: "start", source: drop.source };
  }
}

/** Every speaker on the page, by name. */
export function everySpeaker(view: MediaView): MediaSpeaker[] {
  return [...view.groups.flatMap((g) => g.speakers), ...view.idle].sort((a, b) =>
    a.name.localeCompare(b.name),
  );
}

/** "Living Room, Living Room Back" — a group by its speakers. */
function names(group: MediaGroup): string {
  return group.speakers.map((s) => s.name).join(", ");
}

const KIND_TITLES: Record<MediaGroup["kind"], string> = {
  tv: "TV",
  radio: "Radio",
  spotify: "Spotify",
  music: "Other music",
};

/** Where a speaker can go, for the move menu: each group but the TV and
    its own, and "not playing" unless it is there already. */
export function destinations(view: MediaView, speaker: string): { label: string; drop: Drop }[] {
  const out: { label: string; drop: Drop }[] = [];
  for (const group of view.groups) {
    const drop: Drop = { to: "group", leader: group.leader };
    if (!moveFor(view, speaker, drop)) continue;
    const what = group.what ? ` — ${group.what}` : "";
    out.push({ label: `${KIND_TITLES[group.kind]}${what} (${names(group)})`, drop });
  }
  out.push({ label: "Start the radio", drop: { to: "start", source: "radio" } });
  out.push({ label: "Start Spotify", drop: { to: "start", source: "spotify" } });
  if (moveFor(view, speaker, { to: "idle" })) {
    out.push({ label: "Not playing", drop: { to: "idle" } });
  }
  return out;
}

/** How far the page scrolls each frame for a drag held `y` from the
    top of a `height`-tall window: nothing in the middle, faster the
    deeper into the top or bottom edge. */
export function edgeScroll(y: number, height: number, edge = 96): number {
  if (y < edge) return -Math.ceil(((edge - y) / edge) * 16);
  if (y > height - edge) return Math.ceil(((y - (height - edge)) / edge) * 16);
  return 0;
}

/** The drop target under a point, read off the page's `data-drop`. */
function dropUnder(x: number, y: number): string | null {
  const el = document.elementFromPoint(x, y);
  return el?.closest("[data-drop]")?.getAttribute("data-drop") ?? null;
}

/**
 * Everything that plays, by where it comes from: the TV, the radio,
 * Spotify, and the speakers with nothing loaded at all.
 *
 * A group sits under what it has loaded, playing or paused — two
 * speakers paused with a Spotify queue are under Spotify, because play
 * starts both. A speaker is dragged into a group to join it, or into
 * "Not playing" to leave and stop. Its handle, tapped, offers the same
 * moves as a list.
 */
export function MediaPage() {
  const queryClient = useQueryClient();
  const [held, setHeld] = useState<{ id: string; name: string } | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const [moving, setMoving] = useState<MediaSpeaker | null>(null);
  const [starting, setStarting] = useState<{ source: Startable; preset: string[] } | null>(null);
  const pointer = useRef<{ x: number; y: number } | null>(null);

  // The page moves under a drag only at the screen's edges, and then by
  // itself, so a speaker can be carried to a card out of sight.
  const dragging = held !== null;
  useEffect(() => {
    if (!dragging) return;
    let frame = requestAnimationFrame(function tick() {
      const at = pointer.current;
      const by = at ? edgeScroll(at.y, window.innerHeight) : 0;
      if (at && by !== 0) {
        window.scrollBy(0, by);
        setOver(dropUnder(at.x, at.y));
      }
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  }, [dragging]);

  const media = useQuery({
    queryKey: ["media"],
    queryFn: api.media,
    retry: false,
    // Not under a speaker being dragged: the rows would move beneath it.
    refetchInterval: held ? false : 5_000,
  });
  const tv = useQuery({
    queryKey: ["tv"],
    queryFn: api.tv,
    retry: false,
    refetchInterval: 30_000,
  });
  const settle = () => {
    queryClient.invalidateQueries({ queryKey: ["media"] });
    queryClient.invalidateQueries({ queryKey: ["music"] });
  };
  const move = useMutation({
    mutationFn: ({ id, move }: { id: string; move: Move }) =>
      move.action === "join"
        ? api.speakerControl(id, { action: "join", leader: move.leader })
        : api.speakerControl(id, { action: "leave" }),
    onSettled: settle,
  });
  const volume = useMutation({
    mutationFn: ({ id, percent }: { id: string; percent: number }) =>
      api.speakerControl(id, { action: "volume", percent }),
    onSettled: settle,
  });
  const group = useMutation({
    mutationFn: ({ leader, play }: { leader: string; play: boolean }) =>
      api.mediaGroup(leader, play),
    onSettled: settle,
  });
  const tvPower = useMutation({
    // On is the TV and its sound: the soundbar out of whatever group
    // it was in and onto the TV's input.
    mutationFn: async (on: boolean) => {
      await api.tvPower(on);
      if (on) await api.tvSound();
    },
    onSettled: () => {
      settle();
      // A TV takes a few seconds to come on; ask again once it has.
      setTimeout(() => queryClient.invalidateQueries({ queryKey: ["tv"] }), 4_000);
    },
  });
  const tvSound = useMutation({ mutationFn: api.tvSound, onSettled: settle });

  const view = media.data;

  const apply = (speaker: MediaSpeaker, drop: Drop | null) => {
    if (!view || !drop) return;
    const next = moveFor(view, speaker.id, drop);
    if (!next) return;
    if (next.action === "start") setStarting({ source: next.source, preset: [speaker.id] });
    else move.mutate({ id: speaker.id, move: next });
  };

  if (media.isLoading) {
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }
  if (!view) {
    return (
      <p className="text-muted-foreground text-sm">
        The speakers did not answer. Sonos is set up under Settings → Integrations.
      </p>
    );
  }

  const of = (kind: MediaGroup["kind"]) => view.groups.filter((g) => g.kind === kind);
  const pairedTv = tv.data?.paired ? tv.data : undefined;
  const tvGroups = of("tv");
  const others = of("music");
  const busy = move.isPending || group.isPending;
  const failure = [move, group, volume, tvPower, tvSound].find((m) => m.error)?.error;

  const rows = (speakers: MediaSpeaker[]) =>
    speakers.map((speaker) => (
      <SpeakerRow
        key={speaker.id}
        speaker={speaker}
        held={held?.id === speaker.id}
        disabled={busy}
        onHold={() => setHeld({ id: speaker.id, name: speaker.name })}
        onDrag={(x, y) => {
          pointer.current = { x, y };
          setAt({ x, y });
          setOver(dropUnder(x, y));
        }}
        onDrop={(x, y) => {
          pointer.current = null;
          setHeld(null);
          setOver(null);
          setAt(null);
          apply(speaker, dropAt(dropUnder(x, y)));
        }}
        onMenu={() => setMoving(speaker)}
        onVolume={(percent) => volume.mutate({ id: speaker.id, percent })}
      />
    ));

  const groupBlock = (g: MediaGroup) => (
    <div
      key={g.leader}
      data-drop={g.kind === "tv" ? undefined : `group:${g.leader}`}
      className={cn(
        "rounded-lg border px-3 transition-colors",
        held && over === `group:${g.leader}` && "border-foreground/40 bg-muted/50",
      )}
    >
      <div className="flex items-center gap-3 pt-3 pb-1">
        {g.kind === "radio" || g.kind === "spotify" ? (
          // Something else on these speakers: what plays is where to
          // change it, rather than a second play button on the card.
          <button
            type="button"
            aria-label={`Play something else on ${names(g)}`}
            className={cn(
              "group -mx-1 flex min-w-0 flex-1 items-center gap-1 rounded px-1 text-left",
              "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
            )}
            onClick={() =>
              setStarting({
                source: g.kind as Startable,
                preset: g.speakers.map((s) => s.id),
              })
            }
          >
            <span className="min-w-0">
              <span className="block truncate text-sm font-medium group-hover:underline">
                {g.what ?? KIND_TITLES[g.kind]}
              </span>
              <span className="text-muted-foreground block text-xs">
                {g.playing ? "Playing" : "Paused"}
              </span>
            </span>
            <ChevronRight aria-hidden className="text-muted-foreground size-4 shrink-0" />
          </button>
        ) : (
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-medium">
              {g.what ?? (g.kind === "tv" ? "The TV's sound" : KIND_TITLES[g.kind])}
            </div>
            <div className="text-muted-foreground text-xs">
              {g.playing ? "Playing" : "Paused"}
            </div>
          </div>
        )}
        {g.kind !== "tv" && (
          <Button
            variant="outline"
            size="icon"
            aria-label={`${g.playing ? "Pause" : "Play"} ${names(g)}`}
            disabled={busy}
            onClick={() => group.mutate({ leader: g.leader, play: !g.playing })}
          >
            {g.playing ? <Pause aria-hidden /> : <Play aria-hidden />}
          </Button>
        )}
      </div>
      {rows(g.speakers)}
    </div>
  );

  const startable = (source: Startable, title: string, icon: React.ReactNode) => {
    const groups = of(source);
    return (
      <SourceCard
        title={title}
        icon={icon}
        drop={`start:${source}`}
        lit={Boolean(held) && over === `start:${source}`}
        action={
          // One play button at a time: once something is loaded, its
          // group's own.
          groups.length === 0 && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => setStarting({ source, preset: [] })}
            >
              <Play aria-hidden /> Play
            </Button>
          )
        }
      >
        {groups.map(groupBlock)}
        {groups.length === 0 && (
          <p className="text-muted-foreground py-2 text-sm">
            Nothing loaded. Press play, or drop a speaker here.
          </p>
        )}
      </SourceCard>
    );
  };

  return (
    <div className="flex flex-col gap-3">
      {failure && (
        <p role="alert" className="text-destructive text-sm">
          {failure.message}
        </p>
      )}
      {(pairedTv || tvGroups.length > 0) && (
        <SourceCard
          title="TV"
          icon={<Tv aria-hidden className="size-5" />}
          subtitle={pairedTv ? tvLine(pairedTv) : undefined}
          action={
            pairedTv && (
              <PowerButton
                on={tvOn(pairedTv)}
                label="TV"
                disabled={tvPower.isPending}
                onToggle={(on) => tvPower.mutate(on)}
              />
            )
          }
        >
          {tvGroups.map(groupBlock)}
          {tvOn(pairedTv) && tvGroups.length === 0 && (
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              disabled={tvSound.isPending}
              onClick={() => tvSound.mutate()}
            >
              <Speaker aria-hidden /> The TV's sound to the soundbar
            </Button>
          )}
        </SourceCard>
      )}
      {startable("radio", "Radio", <Radio aria-hidden className="size-5" />)}
      {startable("spotify", "Spotify", <BrandMark id="spotify" label="Spotify" className="size-5" />)}
      {others.length > 0 && (
        <SourceCard title="Other music" icon={<Music aria-hidden className="size-5" />}>
          {others.map(groupBlock)}
        </SourceCard>
      )}
      <SourceCard
        title="Not playing"
        icon={<Speaker aria-hidden className="text-muted-foreground size-5" />}
        drop="idle"
        lit={Boolean(held) && over === "idle"}
      >
        {view.idle.length > 0 ? (
          <div className="px-3">{rows(view.idle)}</div>
        ) : (
          <p className="text-muted-foreground py-2 text-sm">
            Every speaker has something loaded. Drop one here to stop it.
          </p>
        )}
      </SourceCard>

      {held && at && (
        <div
          aria-hidden
          className="bg-popover pointer-events-none fixed z-50 -translate-x-1/2 -translate-y-full rounded-md border px-2 py-1 text-xs font-medium shadow-md"
          style={{ left: at.x, top: at.y - 12 }}
        >
          {held.name}
        </div>
      )}

      <MoveDialog
        view={view}
        speaker={moving}
        onClose={() => setMoving(null)}
        onMove={(speaker, drop) => {
          setMoving(null);
          apply(speaker, drop);
        }}
      />
      <StartDialog
        source={starting?.source}
        preset={starting?.preset ?? []}
        speakers={everySpeaker(view).filter(
          (s) => !tvGroups.some((g) => g.speakers.some((t) => t.id === s.id)),
        )}
        onClose={() => setStarting(null)}
        onStarted={settle}
      />
    </div>
  );
}

function SourceCard({
  title,
  icon,
  subtitle,
  action,
  drop,
  lit,
  children,
}: {
  title: string;
  icon: React.ReactNode;
  subtitle?: string;
  action?: React.ReactNode;
  drop?: string;
  lit?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section
      aria-label={title}
      data-drop={drop}
      className={cn(
        "bg-card flex flex-col gap-2 rounded-xl px-4 py-3 ring-1 ring-transparent transition-shadow",
        lit && "ring-foreground/40",
      )}
    >
      <div className="flex items-center gap-3">
        {icon}
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">{title}</h2>
          {subtitle && <div className="text-muted-foreground truncate text-xs">{subtitle}</div>}
        </div>
        {action}
      </div>
      {children}
    </section>
  );
}

function SpeakerRow({
  speaker,
  held,
  disabled,
  onHold,
  onDrag,
  onDrop,
  onMenu,
  onVolume,
}: {
  speaker: MediaSpeaker;
  held: boolean;
  disabled?: boolean;
  onHold: () => void;
  onDrag: (x: number, y: number) => void;
  onDrop: (x: number, y: number) => void;
  onMenu: () => void;
  onVolume: (percent: number) => void;
}) {
  const [draft, setDraft] = useState(speaker.volume ?? 0);
  useEffect(() => setDraft(speaker.volume ?? 0), [speaker.volume]);
  // Where the press began, and whether it went anywhere: a press that
  // stays put is a tap, and a tap opens the move menu instead.
  const start = useRef<{ x: number; y: number } | null>(null);
  const dragged = useRef(false);
  const room = speaker.room ? humanize(speaker.room) : null;

  return (
    <div className={cn("flex flex-col gap-1 py-2", held && "opacity-50")}>
      <div className="flex items-center gap-2">
        <button
          type="button"
          disabled={disabled}
          aria-label={`Move ${speaker.name}`}
          className={cn(
            "text-muted-foreground hover:text-foreground -m-1 cursor-grab touch-none rounded p-1",
            "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
            "disabled:cursor-not-allowed disabled:opacity-50",
          )}
          onPointerDown={(e) => {
            // No text selection, which would scroll the page along with
            // the mouse: the page scrolls only at the screen's edges.
            e.preventDefault();
            e.currentTarget.setPointerCapture(e.pointerId);
            start.current = { x: e.clientX, y: e.clientY };
            dragged.current = false;
          }}
          onPointerMove={(e) => {
            if (!start.current) return;
            const far =
              Math.abs(e.clientX - start.current.x) + Math.abs(e.clientY - start.current.y) > 8;
            if (!dragged.current && far) {
              dragged.current = true;
              onHold();
            }
            if (dragged.current) onDrag(e.clientX, e.clientY);
          }}
          onPointerUp={(e) => {
            start.current = null;
            if (dragged.current) onDrop(e.clientX, e.clientY);
          }}
          onClick={() => {
            if (dragged.current) {
              dragged.current = false;
              return;
            }
            onMenu();
          }}
        >
          <GripVertical aria-hidden className="size-4" />
        </button>
        <div className="min-w-0 flex-1 truncate text-sm">
          {speaker.name}
          {room && room !== speaker.name && (
            <span className="text-muted-foreground"> · {room}</span>
          )}
        </div>
        {speaker.volume !== null && (
          <span className="text-muted-foreground w-8 text-right text-xs tabular-nums">{draft}</span>
        )}
      </div>
      {speaker.volume !== null && (
        <Slider
          className="pl-6"
          value={draft}
          min={0}
          max={100}
          step={1}
          thumbLabel={`${speaker.name} volume`}
          thumbValueText={`${draft}%`}
          onValueChange={(next, details) => {
            const picked = Array.isArray(next) ? next[0] : next;
            setDraft(picked);
            // A tap on the track never reaches onValueCommitted on a
            // touch screen — the Base UI quirk the light rows hit.
            if (details.reason === "track-press") onVolume(picked);
          }}
          onValueCommitted={(next) => onVolume(Array.isArray(next) ? next[0] : next)}
        />
      )}
    </div>
  );
}

/** The moves a drag makes, as a list: for a tap on the handle, and for
    a keyboard. */
function MoveDialog({
  view,
  speaker,
  onClose,
  onMove,
}: {
  view: MediaView;
  speaker: MediaSpeaker | null;
  onClose: () => void;
  onMove: (speaker: MediaSpeaker, drop: Drop) => void;
}) {
  return (
    <Dialog
      open={speaker !== null}
      onOpenChange={(open: boolean) => {
        if (!open) onClose();
      }}
    >
      <DialogContent initialFocus={(openType) => openType === "keyboard"}>
        {speaker && (
          <DialogBody className="flex flex-col gap-3">
            <div>
              <DialogTitle>Move {speaker.name}</DialogTitle>
              <DialogDescription>
                Into a group to play along with it, or out to stop.
              </DialogDescription>
            </div>
            <div className="flex flex-col gap-1">
              {destinations(view, speaker.id).map(({ label, drop }) => (
                <Button
                  key={label}
                  variant="ghost"
                  className="h-auto justify-start py-2 text-left whitespace-normal"
                  onClick={() => onMove(speaker, drop)}
                >
                  {label}
                </Button>
              ))}
            </div>
          </DialogBody>
        )}
      </DialogContent>
    </Dialog>
  );
}
