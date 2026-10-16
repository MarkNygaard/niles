import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  BedDouble,
  Check,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  MapPin,
  Plane,
  StickyNote,
  Ticket,
  Users,
  Utensils,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { ArtifactPicture } from "@/lib/artifactImages";
import type { KindViewProps } from "@/lib/artifacts";
import { Markdown } from "@/lib/markdown";
import {
  ITEM_LABELS,
  costTotals,
  countdown,
  dayLabel,
  days,
  isOn,
  localDay,
  mapsLink,
  money,
  nights,
  openDay,
  packingGroups,
  packingKey,
  packingReason,
  span,
  travellers,
  tripLength,
  tripOf,
  unsaved,
  weekday,
  withTicks,
} from "@/lib/trip";
import type { Day, Item, ItemType, PackingItem, Stay, Trip } from "@/lib/trip";
import { cn } from "@/lib/utils";

const ICONS: Record<ItemType, LucideIcon> = {
  travel: Plane,
  stay: BedDouble,
  activity: Ticket,
  food: Utensils,
  note: StickyNote,
};

/**
 * How long ticks wait before they are saved. Every save is a version,
 * and only the last 50 are kept: packing for a trip is thirty ticks in
 * a few minutes, and one version each would push the plan Claude wrote
 * out of the history. So the ticks of one go at the list are saved
 * together, a moment after the last, as one version: "Packed 6 things".
 */
export const SAVE_TICKS_AFTER_MS = 1500;

export interface TripViewProps extends KindViewProps {
  /** "YYYY-MM-DD"; this device's today when left out. */
  today?: string;
}

/**
 * A trip, open in Niles's panel: where and when, how long until it
 * starts or which day of it this is, the stays, then the days, today's
 * open while the trip is on and the others folded. The packing list is
 * the one part the person changes here, by ticking it; the rest changes
 * by asking for it. Pictures are the ones the trip keeps: the trip's
 * at the top, a day's as it opens, an item's under it.
 */
export function TripView({ content, onChange, busy, today = localDay() }: TripViewProps) {
  const trip = tripOf(content);
  const ticks = usePackingTicks(trip, onChange, busy);
  const [open, setOpen] = useState<ReadonlySet<number>>(() => {
    const first = trip ? openDay(trip, today) : -1;
    return new Set(first === -1 ? [] : [first]);
  });

  if (!trip) {
    return <p className="text-muted-foreground text-sm">This trip cannot be read.</p>;
  }
  const toggle = (i: number) =>
    setOpen((was) => {
      const next = new Set(was);
      if (!next.delete(i)) next.add(i);
      return next;
    });
  const on = isOn(trip, today);

  return (
    <div className="flex flex-col gap-3">
      <ArtifactPicture name={trip.image} />
      <div className="bg-muted/60 flex flex-col gap-1 rounded-xl px-3 py-2.5">
        <div className="flex items-start justify-between gap-2">
          <span className="font-medium break-words">{trip.destination}</span>
          <Badge variant={on ? "default" : "secondary"} className="shrink-0">
            {countdown(trip, today)}
          </Badge>
        </div>
        <span className="text-muted-foreground text-sm">
          {span(trip.start, trip.end)} · {days(tripLength(trip))}
        </span>
        {trip.travellers && trip.travellers.length > 0 && (
          <span className="text-muted-foreground flex items-center gap-1.5 text-sm">
            <Users aria-hidden className="size-3.5" />
            {travellers(trip.travellers)}
          </span>
        )}
      </div>

      {trip.stays && trip.stays.length > 0 && (
        <Section title="Stays">
          <ul aria-label="Stays" className="flex flex-col gap-1.5">
            {trip.stays.map((s, i) => (
              <StayRow key={`${s.check_in}-${i}`} stay={s} />
            ))}
          </ul>
        </Section>
      )}

      <ol aria-label="Days" className="flex flex-col gap-1.5">
        {trip.days.map((d, i) => (
          <DayRow
            key={d.date}
            day={d}
            today={d.date === today}
            past={on && d.date < today}
            open={open.has(i)}
            onToggle={() => toggle(i)}
          />
        ))}
      </ol>

      {trip.packing && trip.packing.length > 0 && (
        <Packing trip={trip} ticks={ticks.current} onTick={onChange && ticks.tick} />
      )}

      {trip.budget && <Budget trip={trip} />}

      {trip.notes && (
        <Section title="Notes">
          <Markdown source={trip.notes} />
        </Section>
      )}
    </div>
  );
}

/**
 * The person's ticks, shown at once and saved together a moment after
 * the last (see `SAVE_TICKS_AFTER_MS`), or straight away when the
 * view goes, the page is hidden, or an older version is opened over it.
 * A tick stays here until the content it was saved into comes back, so
 * the box does not flicker while the save is on its way.
 */
function usePackingTicks(
  trip: Trip | undefined,
  onChange: KindViewProps["onChange"],
  busy: boolean | undefined,
) {
  const [ticks, setTicks] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const live = useRef({ trip, onChange, busy, ticks });
  // What a save goes into is the content being edited: once an older
  // version is open in its place, the last editable one still is.
  useLayoutEffect(() => {
    live.current = onChange ? { trip, onChange, busy, ticks } : { ...live.current, busy, ticks };
  });

  const save = useCallback(() => {
    clearTimeout(timer.current);
    timer.current = undefined;
    const { trip, onChange, busy, ticks } = live.current;
    if (!trip || !onChange) return;
    if (busy) {
      timer.current = setTimeout(save, SAVE_TICKS_AFTER_MS / 3);
      return;
    }
    const changed = unsaved(trip, ticks);
    if (changed.length > 0) onChange(withTicks(trip, ticks), packingReason(changed));
  }, []);

  const kept = JSON.stringify(trip?.packing ?? []);
  useEffect(() => {
    const packing = JSON.parse(kept) as PackingItem[];
    setTicks((was) => {
      const left = new Map(was);
      for (const p of packing) {
        if (left.get(packingKey(p)) === p.packed) left.delete(packingKey(p));
      }
      return left.size === was.size ? was : left;
    });
  }, [kept]);

  const editable = onChange !== undefined;
  useEffect(() => {
    if (!editable) save();
  }, [editable, save]);

  useEffect(() => {
    const hidden = () => {
      if (document.visibilityState === "hidden") save();
    };
    document.addEventListener("visibilitychange", hidden);
    return () => {
      document.removeEventListener("visibilitychange", hidden);
      save();
    };
  }, [save]);

  const tick = useCallback(
    (key: string, packed: boolean) => {
      const next = new Map(live.current.ticks);
      next.set(key, packed);
      live.current = { ...live.current, ticks: next };
      setTicks(next);
      clearTimeout(timer.current);
      timer.current = setTimeout(save, SAVE_TICKS_AFTER_MS);
    },
    [save],
  );

  return { current: editable ? ticks : new Map<string, boolean>(), tick };
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-border flex flex-col gap-1.5 border-t pt-3">
      <h4 className="text-muted-foreground text-xs font-medium tracking-wide uppercase">{title}</h4>
      {children}
    </section>
  );
}

function BookedBadge() {
  return (
    <Badge variant="outline" className="text-ok h-4 gap-0.5 px-1.5 text-[10px]">
      <Check aria-hidden className="size-3" />
      Booked
    </Badge>
  );
}

function Outbound({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer nofollow"
      className="text-primary inline-flex items-center gap-1 underline-offset-2 hover:underline"
    >
      {children}
    </a>
  );
}

function StayRow({ stay: s }: { stay: Stay }) {
  return (
    <li className="bg-muted/30 flex items-start gap-2.5 rounded-lg px-2.5 py-2">
      <BedDouble aria-hidden className="text-muted-foreground mt-0.5 size-4 shrink-0" />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex flex-wrap items-center gap-1.5 text-sm font-medium">
          {s.link ? (
            <Outbound href={s.link}>
              {s.name}
              <ExternalLink aria-hidden className="size-3" />
            </Outbound>
          ) : (
            s.name
          )}
          {s.booked && <BookedBadge />}
        </span>
        <span className="text-muted-foreground text-xs">
          {[s.place, span(s.check_in, s.check_out), nights(s)].join(" · ")}
        </span>
        {s.confirmation && (
          <span className="text-muted-foreground text-xs">
            Ref <span className="font-mono">{s.confirmation}</span>
          </span>
        )}
      </span>
    </li>
  );
}

function DayRow({
  day: d,
  today,
  past,
  open,
  onToggle,
}: {
  day: Day;
  today: boolean;
  past: boolean;
  open: boolean;
  onToggle: () => void;
}) {
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <li
      aria-current={today ? "date" : undefined}
      className={cn(
        "rounded-xl border",
        today ? "border-ring/50 bg-card" : "border-border",
        past && !open && "opacity-70",
      )}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={onToggle}
        className="flex w-full items-center gap-3 px-3 py-2.5 text-left"
      >
        <span className="flex w-8 shrink-0 flex-col items-center leading-tight">
          <span className="text-muted-foreground text-[11px]">{weekday(d.date)}</span>
          <span className="text-sm font-medium tabular-nums">{Number(d.date.slice(8))}</span>
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="flex items-center gap-1.5 text-sm font-medium">
            <span className="truncate">{d.place}</span>
            {today && <Badge className="h-4 shrink-0 px-1.5 text-[10px]">Today</Badge>}
          </span>
          {d.title && <span className="text-muted-foreground truncate text-xs">{d.title}</span>}
        </span>
        <Chevron aria-hidden className="text-muted-foreground size-4 shrink-0" />
      </button>
      {open && (
        <ul aria-label={dayLabel(d.date)} className="flex flex-col gap-1.5 px-2 pb-2">
          {d.image && (
            <li className="list-none">
              <ArtifactPicture name={d.image} />
            </li>
          )}
          {d.items.length === 0 && (
            <li className="text-muted-foreground px-2 py-1 text-sm">Nothing planned.</li>
          )}
          {d.items.map((item, i) => (
            <ItemRow key={i} item={item} />
          ))}
        </ul>
      )}
    </li>
  );
}

function ItemRow({ item }: { item: Item }) {
  const Icon = ICONS[item.type] ?? StickyNote;
  return (
    <li className="bg-muted/30 flex gap-2.5 rounded-lg px-2.5 py-2">
      <span className="text-muted-foreground w-10 shrink-0 pt-0.5 text-xs tabular-nums">
        {item.time}
      </span>
      <Icon
        role="img"
        aria-label={ITEM_LABELS[item.type] ?? item.type}
        className="text-muted-foreground mt-0.5 size-4 shrink-0"
      />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex flex-wrap items-center gap-1.5 text-sm font-medium">
          {item.link ? (
            <Outbound href={item.link}>
              {item.title}
              <ExternalLink aria-hidden className="size-3" />
            </Outbound>
          ) : (
            item.title
          )}
          {item.booked && <BookedBadge />}
        </span>
        {item.details && (
          <span className="text-muted-foreground text-xs whitespace-pre-line">{item.details}</span>
        )}
        <ArtifactPicture name={item.image} className="mt-1" />
        {(item.location || item.cost) && (
          <span className="text-muted-foreground flex flex-wrap items-center gap-x-2 text-xs">
            {item.location && (
              <Outbound href={mapsLink(item.location)}>
                <MapPin aria-hidden className="size-3" />
                {item.location}
              </Outbound>
            )}
            {item.cost && (
              <span className="tabular-nums">
                {item.booked ? "" : "≈ "}
                {money(item.cost.amount, item.cost.currency)}
              </span>
            )}
          </span>
        )}
      </span>
    </li>
  );
}

function Packing({
  trip,
  ticks,
  onTick,
}: {
  trip: Trip;
  ticks: ReadonlyMap<string, boolean>;
  onTick?: (key: string, packed: boolean) => void;
}) {
  const packing = withTicks(trip, ticks).packing ?? [];
  const packed = packing.filter((p) => p.packed).length;
  return (
    <Section title={`Packing · ${packed} of ${packing.length}`}>
      {packingGroups(packing).map(([group, items]) => (
        <div key={group} className="flex flex-col gap-1">
          {group && <span className="text-xs font-medium">{group}</span>}
          <ul aria-label={group || "Packing"} className="flex flex-col">
            {items.map((p) => {
              const key = packingKey(p);
              return (
                <li key={key}>
                  <label className="flex min-h-9 items-center gap-2.5 text-sm">
                    <input
                      type="checkbox"
                      checked={p.packed}
                      disabled={!onTick}
                      onChange={(e) => onTick?.(key, e.target.checked)}
                      className="accent-primary size-4 shrink-0"
                    />
                    <span className={cn(p.packed && "text-muted-foreground line-through")}>
                      {p.item}
                    </span>
                  </label>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </Section>
  );
}

function Budget({ trip }: { trip: Trip }) {
  const totals = costTotals(trip);
  return (
    <Section title={`Budget · ${trip.budget?.currency}`}>
      {totals.length > 0 && (
        <p className="text-sm">
          Costs listed:{" "}
          <span className="tabular-nums">
            {totals.map((t) => money(t.amount, t.currency)).join(" · ")}
          </span>
        </p>
      )}
      {trip.budget?.notes && (
        <p className="text-muted-foreground text-sm whitespace-pre-line">{trip.budget.notes}</p>
      )}
    </Section>
  );
}
