/**
 * A trip's content, as crates/niles-artifacts/src/trip.rs keeps it, and
 * what its view works out from it: how long until it starts or which
 * day of it this is, the nights at each stay, what the listed costs come
 * to, and the packing list with the person's own ticks.
 *
 * Days are "YYYY-MM-DD" throughout and counted in UTC, so a day is a day
 * whatever the clock does in between; "today" comes from the caller.
 */

export type ItemType = "travel" | "stay" | "activity" | "food" | "note";

export interface Cost {
  amount: number;
  /** ISO 4217: "THB". */
  currency: string;
}

export interface Item {
  /** "HH:MM". */
  time?: string;
  type: ItemType;
  title: string;
  details?: string;
  location?: string;
  /** Always https: Niles refuses anything else. */
  link?: string;
  booked?: boolean;
  /** A picture the trip keeps, `niles-image:12`. */
  image?: string;
  cost?: Cost;
}

export interface Day {
  date: string;
  place: string;
  title?: string;
  image?: string;
  items: Item[];
}

export interface Stay {
  name: string;
  place: string;
  check_in: string;
  check_out: string;
  link?: string;
  booked?: boolean;
  confirmation?: string;
}

export interface PackingItem {
  item: string;
  group?: string;
  packed: boolean;
}

export interface Trip {
  destination: string;
  start: string;
  end: string;
  travellers?: string[];
  image?: string;
  days: Day[];
  stays?: Stay[];
  packing?: PackingItem[];
  budget?: { currency: string; notes?: string };
  notes?: string;
}

/** The trip in `content`, or undefined when it is not one: Niles
    validates every version, so this is only a guard. */
export function tripOf(content: unknown): Trip | undefined {
  if (!content || typeof content !== "object") return undefined;
  const c = content as Partial<Trip>;
  if (typeof c.start !== "string" || typeof c.end !== "string" || !Array.isArray(c.days)) {
    return undefined;
  }
  return {
    ...c,
    destination: c.destination ?? "",
    start: c.start,
    end: c.end,
    days: c.days.map((d) => ({ ...d, items: Array.isArray(d.items) ? d.items : [] })),
  };
}

const DAY_MS = 24 * 60 * 60 * 1000;
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function toUtc(day: string): Date {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d));
}

/** The day it is on this device. */
export function localDay(now: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** From `a` to `b` in whole days: positive when `b` is later. */
export function daysBetween(a: string, b: string): number {
  return Math.round((toUtc(b).getTime() - toUtc(a).getTime()) / DAY_MS);
}

/** How many days the trip is, its first and last both counted. */
export function tripLength(trip: Trip): number {
  return daysBetween(trip.start, trip.end) + 1;
}

/** "in 12 weeks", "in 5 days", "tomorrow", "day 4 of 21", "done". */
export function countdown(trip: Trip, today: string): string {
  const until = daysBetween(today, trip.start);
  if (until > 0) {
    if (until === 1) return "Tomorrow";
    if (until < 14) return `In ${until} days`;
    return `In ${Math.floor(until / 7)} weeks`;
  }
  if (today > trip.end) return "Done";
  return `Day ${1 - until} of ${tripLength(trip)}`;
}

/** Whether the trip is going on today. */
export function isOn(trip: Trip, today: string): boolean {
  return today >= trip.start && today <= trip.end;
}

/** The day to open first: today's while the trip is on (or the last one
    before it, on a day with nothing planned), the first before it, none
    once it is over. */
export function openDay(trip: Trip, today: string): number {
  if (today > trip.end) return -1;
  if (today < trip.start) return 0;
  let at = 0;
  trip.days.forEach((d, i) => {
    if (d.date <= today) at = i;
  });
  return at;
}

/** "Tue". */
export function weekday(day: string): string {
  return WEEKDAYS[toUtc(day).getUTCDay()];
}

/** "Sun 3 Jan". */
export function dayLabel(day: string): string {
  const d = toUtc(day);
  return `${WEEKDAYS[d.getUTCDay()]} ${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}`;
}

/** "3 Jan", "3–24 Jan", or "30 Jan – 2 Feb" across a month. */
export function span(from: string, to: string): string {
  const a = toUtc(from);
  const b = toUtc(to);
  if (from === to) return `${a.getUTCDate()} ${MONTHS[a.getUTCMonth()]}`;
  if (a.getUTCFullYear() === b.getUTCFullYear() && a.getUTCMonth() === b.getUTCMonth()) {
    return `${a.getUTCDate()}–${b.getUTCDate()} ${MONTHS[b.getUTCMonth()]}`;
  }
  return `${a.getUTCDate()} ${MONTHS[a.getUTCMonth()]} – ${b.getUTCDate()} ${MONTHS[b.getUTCMonth()]}`;
}

/** "22 days", "1 day". */
export function days(n: number): string {
  return n === 1 ? "1 day" : `${n} days`;
}

/** "2 nights", "1 night". */
export function nights(stay: Stay): string {
  const n = daysBetween(stay.check_in, stay.check_out);
  return n === 1 ? "1 night" : `${n} nights`;
}

/** "Mark", "Mark & Majse", "Mark, Majse & Ida". */
export function travellers(names: string[]): string {
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} & ${names[names.length - 1]}`;
}

/** A search for the place on a map, which works for a name as well as
    an address, and opens the maps app on a phone. */
export function mapsLink(location: string): string {
  return `https://www.google.com/maps/search/?api=1&query=${encodeURIComponent(location)}`;
}

/** "500 THB", "5,400.50 DKK". */
export function money(amount: number, currency: string): string {
  const whole = Number.isInteger(amount);
  const n = amount.toLocaleString("en", {
    minimumFractionDigits: whole ? 0 : 2,
    maximumFractionDigits: whole ? 0 : 2,
  });
  return `${n} ${currency}`;
}

/** What every cost on the trip adds up to, one sum per currency, the
    budget's first. */
export function costTotals(trip: Trip): Cost[] {
  const sums = new Map<string, number>();
  if (trip.budget) sums.set(trip.budget.currency, 0);
  for (const d of trip.days) {
    for (const item of d.items) {
      if (!item.cost) continue;
      sums.set(item.cost.currency, (sums.get(item.cost.currency) ?? 0) + item.cost.amount);
    }
  }
  return [...sums]
    .filter(([, amount]) => amount > 0)
    .map(([currency, amount]) => ({ currency, amount: Math.round(amount * 100) / 100 }));
}

/** Which packing item this is, whatever its place in the list: Niles
    keeps each item once per group. */
export function packingKey(p: Pick<PackingItem, "item" | "group">): string {
  return `${(p.group ?? "").toLowerCase()}\u0000${p.item.toLowerCase()}`;
}

/** The packing list in its groups, in the order each first appears;
    an item without one is in the group "". */
export function packingGroups(packing: PackingItem[]): [string, PackingItem[]][] {
  const groups = new Map<string, PackingItem[]>();
  for (const p of packing) {
    const g = p.group ?? "";
    groups.set(g, [...(groups.get(g) ?? []), p]);
  }
  return [...groups];
}

/** The ticks in `ticks` that the trip does not have yet. */
export function unsaved(trip: Trip, ticks: ReadonlyMap<string, boolean>): PackingItem[] {
  return (trip.packing ?? [])
    .filter((p) => {
      const packed = ticks.get(packingKey(p));
      return packed !== undefined && packed !== p.packed;
    })
    .map((p) => ({ ...p, packed: !p.packed }));
}

/** The trip with `ticks` on its packing list. */
export function withTicks(trip: Trip, ticks: ReadonlyMap<string, boolean>): Trip {
  if (!trip.packing) return trip;
  return {
    ...trip,
    packing: trip.packing.map((p) => ({ ...p, packed: ticks.get(packingKey(p)) ?? p.packed })),
  };
}

/** Why a version of ticks was saved: "Packed “Sunscreen”", "Packed 4
    things", "Packing list: 3 packed, 1 unpacked". */
export function packingReason(changed: PackingItem[]): string {
  if (changed.length === 1) {
    const [p] = changed;
    return `${p.packed ? "Packed" : "Unpacked"} “${p.item}”`;
  }
  const packed = changed.filter((p) => p.packed).length;
  const unpacked = changed.length - packed;
  if (unpacked === 0) return `Packed ${packed} things`;
  if (packed === 0) return `Unpacked ${unpacked} things`;
  return `Packing list: ${packed} packed, ${unpacked} unpacked`;
}

export const ITEM_LABELS: Record<ItemType, string> = {
  travel: "Travel",
  stay: "Stay",
  activity: "Activity",
  food: "Food",
  note: "Note",
};

/** The trip as text, for Copy. */
export function tripText(trip: Trip): string {
  const lines = [
    [trip.destination, span(trip.start, trip.end), days(tripLength(trip))].filter(Boolean).join(" · "),
  ];
  if (trip.travellers?.length) lines.push(travellers(trip.travellers));
  if (trip.stays?.length) {
    lines.push("", "Stays");
    for (const s of trip.stays) {
      const extra = [s.booked && "booked", s.confirmation && `ref ${s.confirmation}`]
        .filter(Boolean)
        .join(", ");
      lines.push(
        `- ${s.name}, ${s.place}: ${span(s.check_in, s.check_out)} (${nights(s)})${extra ? `, ${extra}` : ""}`,
      );
    }
  }
  for (const d of trip.days) {
    lines.push("", [dayLabel(d.date), d.place, d.title].filter(Boolean).join(" · "));
    for (const item of d.items) {
      const tail = [
        item.location,
        item.cost && money(item.cost.amount, item.cost.currency),
        item.booked && "booked",
      ]
        .filter(Boolean)
        .join(", ");
      lines.push(`- ${item.time ? `${item.time} ` : ""}${item.title}${tail ? ` (${tail})` : ""}`);
      if (item.details) lines.push(`  ${item.details.replace(/\n/g, "\n  ")}`);
      if (item.link) lines.push(`  ${item.link}`);
    }
  }
  if (trip.packing?.length) {
    lines.push("", "Packing");
    for (const [group, items] of packingGroups(trip.packing)) {
      if (group) lines.push(`${group}:`);
      for (const p of items) lines.push(`- [${p.packed ? "x" : " "}] ${p.item}`);
    }
  }
  if (trip.budget?.notes) lines.push("", `Budget (${trip.budget.currency})`, trip.budget.notes);
  if (trip.notes) lines.push("", trip.notes);
  return lines.join("\n");
}
