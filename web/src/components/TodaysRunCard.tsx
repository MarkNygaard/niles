import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Footprints } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { ARTIFACTS } from "@/lib/artifacts";
import {
  TYPE_LABELS,
  countdown,
  dayLabel,
  isRestDay,
  km,
  locate,
  marksFor,
  measures,
  outcome,
  planOf,
  recentNote,
  weekShare,
  withMark,
} from "@/lib/trainingPlan";
import type { Mark, Session, WeekToday } from "@/lib/trainingPlan";
import { cn } from "@/lib/utils";

/** Under the artifacts' key, so whatever refreshes the Saved tab — a
    save, an `artifact_changed` on the stream — refreshes this too. */
export const TRAINING_TODAY = [...ARTIFACTS, "training-today"];

/**
 * Today's run from the signed-in person's training plan, on the
 * dashboard: the race and how long to go, today's session large, the
 * next one small, how far the week has gone, and the coach's note while
 * it is news. A tap opens the plan in Niles's panel. Somebody with no
 * plan, or a Niles without a database, gets no card at all.
 */
export function TodaysRunCard() {
  const queryClient = useQueryClient();
  const today = useQuery({
    queryKey: TRAINING_TODAY,
    queryFn: api.trainingToday,
    retry: false,
    // A page left open overnight wakes up on the new day's session.
    refetchInterval: 30 * 60_000,
  });
  // Saved on the plan as it is now, not as this card last read it: a
  // run may have marked another session in between.
  const mark = useMutation({
    mutationFn: async ({ id, session, mark }: { id: number; session: Session; mark: Mark }) => {
      const plan = planOf((await api.artifact(id)).content);
      const at = plan && locate(plan, session);
      if (!plan || !at) throw new Error("The plan has changed. Open it to mark this session.");
      const next = withMark(plan, at.week, at.n, mark);
      return api.patchArtifact(id, { content: next.plan, reason: next.reason });
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: ARTIFACTS }),
  });

  const plan = today.data?.plan;
  if (!plan) return null;
  const runs = plan.sessions.filter((s) => s.type !== "rest");
  const note = recentNote(plan.update, plan.today);
  const open = runs.filter((s) => marksFor(s, plan.today).includes("done"));
  const failure = mark.error;

  return (
    <section
      aria-label="Today's run"
      className="bg-card ring-foreground/10 flex flex-col gap-3 rounded-xl px-4 py-3 ring-1"
    >
      <a
        href={`#/chat?artifact=${plan.id}`}
        aria-label={`Open ${plan.title}`}
        className="flex flex-col gap-3"
      >
        <span className="flex items-center gap-2">
          <Footprints aria-hidden className="text-primary size-4 shrink-0" />
          <span className="text-muted-foreground min-w-0 flex-1 truncate text-xs font-medium">
            {plan.race.race}
          </span>
          <Badge variant="secondary" className="shrink-0">
            {countdown(plan.race.date, plan.today)}
          </Badge>
        </span>
        {isRestDay(plan.sessions) ? (
          <span className="text-lg leading-tight font-semibold">Rest day</span>
        ) : (
          runs.map((s, i) => <Today key={`${s.title}-${i}`} session={s} />)
        )}
        {plan.next && <Next session={plan.next} />}
        {plan.week && plan.week.planned_km > 0 && <WeekBar week={plan.week} />}
        {note && (
          <span className="border-primary/40 text-muted-foreground border-l-2 pl-2 text-xs break-words">
            <span className="text-foreground font-medium">Coach: </span>
            {note.note}
          </span>
        )}
      </a>
      {open.map((s, i) => (
        <div key={`${s.title}-${i}`} className="flex flex-wrap items-center gap-2">
          {open.length > 1 && (
            <span className="text-muted-foreground min-w-0 flex-1 truncate text-xs">{s.title}</span>
          )}
          {(["done", "skipped"] as const).map((m) => (
            <Button
              key={m}
              size="sm"
              variant={m === "done" ? "default" : "outline"}
              disabled={mark.isPending}
              aria-label={`${m === "done" ? "Done" : "Skipped"}: ${s.title}`}
              onClick={() => mark.mutate({ id: plan.id, session: s, mark: m })}
            >
              {m === "done" && <Check />}
              {m === "done" ? "Done" : "Skipped"}
            </Button>
          ))}
        </div>
      ))}
      {failure && (
        <p role="alert" className="text-destructive text-xs">
          {failure.message}
        </p>
      )}
    </section>
  );
}

/** Today's session, large: what it is, how far and how fast, and what
    came of it once it is not to run any more. */
function Today({ session: s }: { session: Session }) {
  const planned = measures(s);
  const result = outcome(s);
  const off = s.status === "missed" || s.status === "skipped" || s.status === "moved";
  return (
    <span className="flex flex-col gap-0.5">
      <span className="flex flex-wrap items-center gap-2">
        <span
          className={cn(
            "text-lg leading-tight font-semibold break-words",
            off && "text-muted-foreground line-through",
          )}
        >
          {s.title}
        </span>
        <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
          {TYPE_LABELS[s.type] ?? s.type}
        </Badge>
      </span>
      {result ? (
        <span
          className={cn(
            "text-sm font-medium tabular-nums",
            s.status === "done" ? "text-ok" : "text-muted-foreground",
          )}
        >
          {result}
        </span>
      ) : (
        <>
          {planned && <span className="text-sm tabular-nums">{planned}</span>}
          {s.details && (
            <span className="text-muted-foreground line-clamp-2 text-xs whitespace-pre-line">
              {s.details}
            </span>
          )}
        </>
      )}
    </span>
  );
}

function Next({ session: s }: { session: Session }) {
  const m = s.distance_km !== undefined ? km(s.distance_km) : measures(s);
  return (
    <span className="text-muted-foreground text-xs">
      Next: {[dayLabel(s.date), s.title, m].filter(Boolean).join(" · ")}
    </span>
  );
}

function WeekBar({ week }: { week: WeekToday }) {
  return (
    <span className="flex flex-col gap-1">
      <span className="text-muted-foreground flex justify-between gap-2 text-xs">
        <span className="truncate">{["This week", week.focus].filter(Boolean).join(" · ")}</span>
        <span className="shrink-0 tabular-nums">
          {week.run_km} / {km(week.planned_km)}
        </span>
      </span>
      <span
        role="progressbar"
        aria-label="Run this week"
        aria-valuemin={0}
        aria-valuemax={week.planned_km}
        aria-valuenow={week.run_km}
        className="bg-muted block h-1.5 overflow-hidden rounded-full"
      >
        <span
          className="bg-primary block h-full rounded-full"
          style={{ width: `${Math.round(weekShare(week) * 100)}%` }}
        />
      </span>
    </span>
  );
}
