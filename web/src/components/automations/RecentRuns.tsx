import { useQuery } from "@tanstack/react-query";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { Run, Trigger } from "@/lib/api";
import { OUTCOME_LABEL, ranAt, runTrigger } from "@/lib/automations";

/** Its own root rather than under `["automations"]`, so pausing one
    rule — which refreshes the list — does not refetch every rule's
    runs as well. */
export function runsKey(id: string) {
  return ["automation-runs", id];
}

/** The list page asks for every stored rule's runs, one request each:
    a minute old is new enough for one muted line, and saves asking
    again each time the page comes back into view. */
export const RUNS_STALE_MS = 60_000;

export function useRuns(id: string) {
  return useQuery({
    queryKey: runsKey(id),
    queryFn: () => api.automationRuns(id),
    staleTime: RUNS_STALE_MS,
  });
}

const OUTCOME_VARIANT = {
  running: "running",
  done: "success",
  stopped: "skipped",
  replaced: "skipped",
  failed: "failed",
} as const satisfies Record<Run["outcome"], string>;

/**
 * What a rule did lately, newest first: when, which trigger, how it
 * ended and why. A run whose conditions did not hold is here too — it
 * is the answer to "why didn't it turn off?".
 *
 * `triggers` are the saved rule's, which are the ones its runs name.
 */
export function RecentRuns({ id, triggers }: { id: string; triggers: Trigger[] }) {
  const runs = useRuns(id);
  return (
    <section aria-label="Recent runs" className="flex flex-col gap-3">
      <h2 className="text-sm font-medium">Recent runs</h2>
      {runs.isPending ? (
        <Skeleton className="h-16 w-full" />
      ) : runs.isError ? (
        <p className="text-muted-foreground text-sm">{runs.error.message}</p>
      ) : runs.data.runs.length === 0 ? (
        <p className="text-muted-foreground text-sm">It has not run since Niles last started.</p>
      ) : (
        <ol className="flex flex-col gap-3">
          {runs.data.runs.map((run) => (
            <RunEntry key={run.started_at} run={run} triggers={triggers} />
          ))}
        </ol>
      )}
    </section>
  );
}

function RunEntry({ run, triggers }: { run: Run; triggers: Trigger[] }) {
  const by = runTrigger(run, triggers);
  return (
    <li className="flex flex-col gap-1 border-b pb-3 last:border-b-0 last:pb-0">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <time dateTime={run.started_at} className="font-medium tabular-nums">
          {ranAt(run.started_at)}
        </time>
        {by && <span className="text-muted-foreground">{by}</span>}
        <Badge variant={OUTCOME_VARIANT[run.outcome]} className="ml-auto">
          {OUTCOME_LABEL[run.outcome]}
        </Badge>
      </div>
      {run.reason && (
        <p
          className={
            run.outcome === "failed" ? "text-destructive text-xs" : "text-muted-foreground text-xs"
          }
        >
          {run.reason}
        </p>
      )}
      {run.steps.length > 0 && (
        <ul className="text-muted-foreground list-disc pl-4 text-xs">
          {run.steps.map((step, i) => (
            <li key={i}>{step}</li>
          ))}
        </ul>
      )}
    </li>
  );
}
