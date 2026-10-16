import { useState } from "react";
import { Check, ChevronDown, ChevronRight, CircleX } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { KindViewProps } from "@/lib/artifacts";
import { Markdown } from "@/lib/markdown";
import {
  TYPE_LABELS,
  countdown,
  currentWeek,
  dayLabel,
  km,
  localDay,
  measures,
  planOf,
  weekRange,
  weekTotals,
  weekday,
} from "@/lib/trainingPlan";
import type { Session, SessionType, Week } from "@/lib/trainingPlan";
import { cn } from "@/lib/utils";

/** Each type's colour, down the left of its row. */
const COLOURS: Record<SessionType, string> = {
  easy: "border-series-3",
  long: "border-series-1",
  tempo: "border-series-4",
  intervals: "border-series-2",
  recovery: "border-moss",
  race: "border-primary",
  rest: "border-border",
  strength: "border-plum",
  cross: "border-sky",
};

export interface TrainingPlanViewProps extends KindViewProps {
  /** "YYYY-MM-DD"; this device's today when left out. */
  today?: string;
}

/**
 * A training plan, open in Niles's panel: the race and how long there
 * is to go, then the weeks, this one open with today marked and the
 * others folded. Read only: the plan changes by asking for it, and
 * after each run by Niles.
 */
export function TrainingPlanView({ content, today = localDay() }: TrainingPlanViewProps) {
  const plan = planOf(content);
  const current = plan ? currentWeek(plan, today) : 0;
  const [open, setOpen] = useState<ReadonlySet<number>>(() => new Set([current]));

  if (!plan) {
    return <p className="text-muted-foreground text-sm">This plan cannot be read.</p>;
  }
  const goal = plan.goal;
  const toggle = (i: number) =>
    setOpen((was) => {
      const next = new Set(was);
      if (!next.delete(i)) next.add(i);
      return next;
    });

  return (
    <div className="flex flex-col gap-3">
      <div className="bg-muted/60 flex flex-col gap-1 rounded-xl px-3 py-2.5">
        <div className="flex items-start justify-between gap-2">
          <span className="font-medium break-words">{goal.race}</span>
          <Badge variant="secondary" className="shrink-0">
            {countdown(goal.date, today)}
          </Badge>
        </div>
        <span className="text-muted-foreground text-sm">
          {[dayLabel(goal.date), km(goal.distance_km), goal.target_time && `goal ${goal.target_time}`]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </div>

      <ol aria-label="Weeks" className="flex flex-col gap-1.5">
        {plan.weeks.map((week, i) => (
          <WeekRow
            key={week.starts}
            week={week}
            n={i + 1}
            today={today}
            current={i === current}
            past={i < current}
            open={open.has(i)}
            onToggle={() => toggle(i)}
          />
        ))}
      </ol>

      {plan.notes && (
        <section className="border-border flex flex-col gap-1 border-t pt-3">
          <h4 className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
            Notes
          </h4>
          <Markdown source={plan.notes} />
        </section>
      )}
    </div>
  );
}

function WeekRow({
  week,
  n,
  today,
  current,
  past,
  open,
  onToggle,
}: {
  week: Week;
  n: number;
  today: string;
  current: boolean;
  past: boolean;
  open: boolean;
  onToggle: () => void;
}) {
  const totals = weekTotals(week);
  const volume =
    totals.planned === 0
      ? undefined
      : current || past
        ? `${totals.done} / ${km(totals.planned)}`
        : km(totals.planned);
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <li
      className={cn(
        "rounded-xl border",
        current ? "border-ring/50 bg-card" : "border-border",
        past && !open && "opacity-70",
      )}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={onToggle}
        className="flex w-full items-center gap-2 px-3 py-2.5 text-left"
      >
        <Chevron aria-hidden className="text-muted-foreground size-4 shrink-0" />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="flex items-center gap-1.5 text-sm font-medium">
            Week {n}
            {current && (
              <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
                This week
              </Badge>
            )}
          </span>
          <span className="text-muted-foreground truncate text-xs">
            {[weekRange(week), week.focus].filter(Boolean).join(" · ")}
          </span>
        </span>
        {volume && <span className="text-muted-foreground shrink-0 text-xs tabular-nums">{volume}</span>}
      </button>
      {open && (
        <ul className="flex flex-col gap-1.5 px-2 pb-2">
          {week.sessions.length === 0 && (
            <li className="text-muted-foreground px-2 py-1 text-sm">Nothing planned.</li>
          )}
          {week.sessions.map((s, i) => (
            <SessionRow key={`${s.date}-${i}`} session={s} today={s.date === today} />
          ))}
        </ul>
      )}
    </li>
  );
}

function SessionRow({ session: s, today }: { session: Session; today: boolean }) {
  const planned = measures(s);
  const ran = s.actual ? measures(s.actual) : "";
  const off = s.status === "missed" || s.status === "skipped" || s.status === "moved";
  return (
    <li
      aria-current={today ? "date" : undefined}
      className={cn(
        "flex gap-3 rounded-lg border-l-4 px-2.5 py-2",
        COLOURS[s.type] ?? "border-border",
        today ? "bg-accent" : "bg-muted/30",
      )}
    >
      <span className="flex w-8 shrink-0 flex-col items-center leading-tight">
        <span className="text-muted-foreground text-[11px]">{weekday(s.date)}</span>
        <span className="text-sm font-medium tabular-nums">{Number(s.date.slice(8))}</span>
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex flex-wrap items-center gap-1.5">
          <span className={cn("text-sm font-medium", off && "text-muted-foreground line-through")}>
            {s.title}
          </span>
          <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
            {TYPE_LABELS[s.type] ?? s.type}
          </Badge>
          {today && <Badge className="h-4 px-1.5 text-[10px]">Today</Badge>}
        </span>
        {planned && <span className="text-muted-foreground text-xs">{planned}</span>}
        {s.details && (
          <span className="text-muted-foreground text-xs whitespace-pre-line">{s.details}</span>
        )}
        {s.status === "done" && (
          <span className="text-ok flex items-center gap-1 text-xs font-medium">
            <Check aria-hidden className="size-3.5" />
            Done{ran && ` · ${ran}`}
          </span>
        )}
        {off && (
          <span
            className={cn(
              "flex items-center gap-1 text-xs",
              s.status === "missed" ? "text-destructive" : "text-muted-foreground",
            )}
          >
            {s.status === "missed" && <CircleX aria-hidden className="size-3.5" />}
            {s.status === "missed" ? "Missed" : s.status === "skipped" ? "Skipped" : "Moved"}
          </span>
        )}
      </span>
    </li>
  );
}
