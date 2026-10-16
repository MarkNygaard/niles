/**
 * A training plan's content, as crates/niles-artifacts/src/training_plan.rs
 * keeps it, and the date arithmetic its view needs: which week is this
 * one, how long to the race, how far each week goes.
 *
 * Days are "YYYY-MM-DD" throughout and counted in UTC, so a day is a day
 * whatever the clock does in between; "today" comes from the caller.
 */

export type SessionType =
  | "easy"
  | "long"
  | "tempo"
  | "intervals"
  | "recovery"
  | "race"
  | "rest"
  | "strength"
  | "cross";

export type SessionStatus = "planned" | "done" | "missed" | "skipped" | "moved";

export interface Measures {
  distance_km?: number;
  duration_min?: number;
  /** Per km: "5:40", or "5:30-5:45". */
  pace?: string;
}

/** What a run that did a session came to. */
export interface Actual extends Measures {
  /** The day it was run, when that was not the session's. */
  date?: string;
}

export interface Session extends Measures {
  date: string;
  type: SessionType;
  title: string;
  details?: string;
  status: SessionStatus;
  /** The Strava activity it was done as. */
  activity?: string;
  /** What the run that did it came to. */
  actual?: Actual;
}

export interface Week {
  starts: string;
  focus?: string;
  sessions: Session[];
}

/** A note in the coach's log: what it changed after a run, and why. */
export interface Update {
  date: string;
  note: string;
}

export interface Plan {
  goal: { race: string; distance_km: number; date: string; target_time?: string };
  weeks: Week[];
  notes?: string;
  /** Newest first. */
  updates?: Update[];
}

/** The plan in `content`, or undefined when it is not one: Niles
    validates every version, so this is only a guard. */
export function planOf(content: unknown): Plan | undefined {
  if (!content || typeof content !== "object") return undefined;
  const c = content as Partial<Plan>;
  if (!c.goal || typeof c.goal.date !== "string" || !Array.isArray(c.weeks)) return undefined;
  return {
    ...c,
    goal: c.goal,
    weeks: c.weeks.map((w) => ({ ...w, sessions: Array.isArray(w.sessions) ? w.sessions : [] })),
    updates: Array.isArray(c.updates) ? c.updates : undefined,
  };
}

const DAY_MS = 24 * 60 * 60 * 1000;
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function toUtc(day: string): Date {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d));
}

function fromUtc(date: Date): string {
  return date.toISOString().slice(0, 10);
}

/** The day it is on this device. */
export function localDay(now: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

export function addDays(day: string, n: number): string {
  return fromUtc(new Date(toUtc(day).getTime() + n * DAY_MS));
}

/** From `a` to `b` in whole days: positive when `b` is later. */
export function daysBetween(a: string, b: string): number {
  return Math.round((toUtc(b).getTime() - toUtc(a).getTime()) / DAY_MS);
}

/** The Sunday a week reaches: the first may start on any day, and
    every week runs to the end of its calendar week. */
export function weekEnds(week: Week): string {
  const monday = (toUtc(week.starts).getUTCDay() + 6) % 7;
  return addDays(week.starts, 6 - monday);
}

/** The week `today` falls in; before the plan the first, after it the
    last. */
export function currentWeek(plan: Plan, today: string): number {
  const last = plan.weeks.length - 1;
  if (last < 0) return 0;
  if (today < plan.weeks[0].starts) return 0;
  const i = plan.weeks.findIndex((w) => today >= w.starts && today <= weekEnds(w));
  return i === -1 ? last : i;
}

/** "9 weeks to go", "10 days to go", "Tomorrow", "Race day". */
export function countdown(raceDay: string, today: string): string {
  const days = daysBetween(today, raceDay);
  if (days < 0) return "Race done";
  if (days === 0) return "Race day";
  if (days === 1) return "Tomorrow";
  if (days < 14) return `${days} days to go`;
  return `${Math.floor(days / 7)} weeks to go`;
}

/** How far a week was planned to go, and how far has been run of it:
    a done session counts what was run when that is known. */
export function weekTotals(week: Week): { planned: number; done: number } {
  let planned = 0;
  let done = 0;
  for (const s of week.sessions) {
    planned += s.distance_km ?? 0;
    if (s.status === "done") done += s.actual?.distance_km ?? s.distance_km ?? 0;
  }
  return { planned: round1(planned), done: round1(done) };
}

function round1(x: number): number {
  return Math.round(x * 10) / 10;
}

/** 6 as "6 km", 21.1 as "21.1 km". */
export function km(x: number): string {
  return `${round1(x)} km`;
}

/** 40 as "40 min", 95 as "1 h 35 min". */
export function minutes(x: number): string {
  const total = Math.round(x);
  if (total < 60) return `${total} min`;
  const h = Math.floor(total / 60);
  const m = total % 60;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** "6 km · 40 min · 6:30 /km", leaving out what is not there. */
export function measures(m: Measures): string {
  return [
    m.distance_km !== undefined && km(m.distance_km),
    m.duration_min !== undefined && minutes(m.duration_min),
    m.pace && `${m.pace} /km`,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** "Tue". */
export function weekday(day: string): string {
  return WEEKDAYS[toUtc(day).getUTCDay()];
}

/** "Sun 13 Dec". */
export function dayLabel(day: string): string {
  const d = toUtc(day);
  return `${WEEKDAYS[d.getUTCDay()]} ${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}`;
}

/** "19–25 Oct", or "27 Oct – 2 Nov" across a month. */
export function weekRange(week: Week): string {
  const a = toUtc(week.starts);
  const b = toUtc(weekEnds(week));
  if (a.getUTCMonth() === b.getUTCMonth()) {
    return a.getUTCDate() === b.getUTCDate()
      ? `${a.getUTCDate()} ${MONTHS[a.getUTCMonth()]}`
      : `${a.getUTCDate()}–${b.getUTCDate()} ${MONTHS[b.getUTCMonth()]}`;
  }
  return `${a.getUTCDate()} ${MONTHS[a.getUTCMonth()]} – ${b.getUTCDate()} ${MONTHS[b.getUTCMonth()]}`;
}

export const TYPE_LABELS: Record<SessionType, string> = {
  easy: "Easy",
  long: "Long",
  tempo: "Tempo",
  intervals: "Intervals",
  recovery: "Recovery",
  race: "Race",
  rest: "Rest",
  strength: "Strength",
  cross: "Cross",
};

/** The plan as text, for Copy. */
export function planText(plan: Plan): string {
  const g = plan.goal;
  const head = [g.race, dayLabel(g.date), km(g.distance_km), g.target_time && `goal ${g.target_time}`]
    .filter(Boolean)
    .join(" · ");
  const lines = [head];
  plan.weeks.forEach((w, i) => {
    const total = weekTotals(w).planned;
    lines.push(
      "",
      [`Week ${i + 1}`, weekRange(w), w.focus, total > 0 && km(total)].filter(Boolean).join(" · "),
    );
    for (const s of w.sessions) {
      const done = s.status === "planned" ? "" : ` (${s.status})`;
      const m = measures(s);
      lines.push(`- ${dayLabel(s.date)}: ${s.title}${m ? ` · ${m}` : ""}${done}`);
    }
  });
  if (plan.notes) lines.push("", plan.notes);
  return lines.join("\n");
}

/** What the person can say about a session with a tap: done or skipped,
    or back to planned. */
export type Mark = "done" | "skipped" | "planned";

/** The marks a session offers: done once its day has come, skipped
    whenever, and undoing either. Nothing for a rest day, nor for one
    that was moved, which is the coach's to change. */
export function marksFor(s: Session, today: string): Mark[] {
  if (s.type === "rest") return [];
  switch (s.status) {
    case "planned":
    case "missed":
      return s.date <= today ? ["done", "skipped"] : ["skipped"];
    case "done":
    case "skipped":
      return ["planned"];
    default:
      return [];
  }
}

const MARKED: Record<Mark, string> = {
  done: "done",
  skipped: "skipped",
  planned: "back to planned",
};

/** The plan with session `n` of week `week` marked, and the reason for
    the version it saves: "Easy run on Tue 20 Oct: done". Only a done
    session keeps a run, so one that is not done any more loses it. */
export function withMark(
  plan: Plan,
  week: number,
  n: number,
  mark: Mark,
): { plan: Plan; reason: string } {
  const was = plan.weeks[week].sessions[n];
  const next: Session = { ...was, status: mark };
  if (mark !== "done") {
    delete next.activity;
    delete next.actual;
  }
  const weeks = plan.weeks.map((w, i) =>
    i === week ? { ...w, sessions: w.sessions.map((s, j) => (j === n ? next : s)) } : w,
  );
  return {
    plan: { ...plan, weeks },
    reason: `${was.title} on ${dayLabel(was.date)}: ${MARKED[mark]}`,
  };
}

/** The week a day is in, so far, as the dashboard's card shows it. */
export interface WeekToday {
  focus: string | null;
  planned_km: number;
  run_km: number;
  /** Sessions from today on still to run, rest days not counted. */
  sessions_left: number;
}

/**
 * The person's current plan read for today: `GET /me/training/today`,
 * the twin of `PlanToday` in crates/niles-api/src/artifacts.rs. Sessions
 * are as the plan keeps them.
 */
export interface PlanToday {
  id: number;
  title: string;
  race: Plan["goal"];
  /** The house's today. */
  today: string;
  days_to_go: number;
  sessions: Session[];
  next: Session | null;
  week: WeekToday | null;
  /** The coach's newest note. */
  update: Update | null;
}

/** No plan with a race still to come is `plan: null`. */
export interface TrainingToday {
  plan: PlanToday | null;
}

/** How old a coach's note may be and still be news on the dashboard:
    one written after a run, read in the days after it. */
export const RECENT_NOTE_DAYS = 3;

/** The coach's note, while it is recent. */
export function recentNote(update: Update | null | undefined, today: string): Update | undefined {
  if (!update) return undefined;
  const age = daysBetween(update.date, today);
  return age >= 0 && age <= RECENT_NOTE_DAYS ? update : undefined;
}

/** Whether a day has nothing to run: no session, or only rest. */
export function isRestDay(sessions: Session[]): boolean {
  return sessions.every((s) => s.type === "rest");
}

/** What came of a session that is not to run any more: "Done ✓ 6.2 km
    at 5:41", "Skipped". Undefined while it is still planned. */
export function outcome(s: Session): string | undefined {
  switch (s.status) {
    case "done": {
      const ran = [
        s.actual?.distance_km !== undefined && km(s.actual.distance_km),
        s.actual?.pace && `at ${s.actual.pace}`,
      ]
        .filter(Boolean)
        .join(" ");
      return ran ? `Done ✓ ${ran}` : "Done ✓";
    }
    case "missed":
      return "Missed";
    case "skipped":
      return "Skipped";
    case "moved":
      return "Moved";
    default:
      return undefined;
  }
}

/** How much of the week has been run, 0 to 1. */
export function weekShare(week: WeekToday): number {
  if (week.planned_km <= 0) return 0;
  return Math.min(1, Math.max(0, week.run_km / week.planned_km));
}

/** Where `s` is in `plan`, found by its day and title: the plan may have
    changed since `s` was read, and a mark is saved on the newest one. */
export function locate(plan: Plan, s: Session): { week: number; n: number } | undefined {
  for (let week = 0; week < plan.weeks.length; week++) {
    const n = plan.weeks[week].sessions.findIndex((x) => x.date === s.date && x.title === s.title);
    if (n !== -1) return { week, n };
  }
  return undefined;
}
