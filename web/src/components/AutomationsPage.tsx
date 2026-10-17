import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Plus, TriangleAlert } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { buttonVariants } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { useRuns } from "@/components/automations/RecentRuns";
import { api } from "@/lib/api";
import type { AutomationView, Rule } from "@/lib/api";
import { lastRunText } from "@/lib/automations";
import { cn } from "@/lib/utils";

export const AUTOMATIONS = ["automations"];

/** What a rule is called: its name, or failing that its sentence. */
export function nameOf(view: AutomationView): string {
  return view.rule.description.trim() || view.sentence;
}

/**
 * Pause or resume a rule: a PUT of the whole rule with `enabled`
 * flipped. The switch moves at once and the list catches up after.
 */
export function useSetEnabled() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ rule, enabled }: { rule: Rule; enabled: boolean }) =>
      api.updateAutomation({ ...rule, enabled }),
    onMutate: async ({ rule, enabled }) => {
      await queryClient.cancelQueries({ queryKey: AUTOMATIONS });
      queryClient.setQueryData<{ rules: AutomationView[] }>(
        AUTOMATIONS,
        (old) =>
          old && {
            rules: old.rules.map((v) =>
              v.rule.id === rule.id ? { ...v, rule: { ...v.rule, enabled } } : v,
            ),
          },
      );
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: AUTOMATIONS }),
  });
}

/**
 * Every rule, as the sentence it reads as.
 *
 * This is the page a phone gets: what is set up, what is paused, and a
 * switch to pause it. Making and changing rules happens on a wider
 * screen, or by asking Niles.
 */
export function AutomationsPage() {
  const list = useQuery({ queryKey: AUTOMATIONS, queryFn: api.automations });
  const setEnabled = useSetEnabled();

  if (list.isPending) {
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-20 w-full" />
        <Skeleton className="h-20 w-full" />
      </div>
    );
  }
  if (list.isError) {
    return <p className="text-destructive text-sm">{list.error.message}</p>;
  }

  const rules = list.data.rules;
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-3">
        <p className="text-muted-foreground text-sm">
          {rules.length === 0
            ? "Nothing runs on its own yet."
            : "Run in this order when they fire together."}
        </p>
        {/* Not on a phone: the flow is built on a wider screen, and
            there it would only say so. */}
        <a href="#/automations/new" className={cn(buttonVariants(), "max-sm:hidden")}>
          <Plus />
          New automation
        </a>
      </div>

      {rules.length === 0 ? (
        <div className="bg-card text-muted-foreground rounded-xl border px-4 py-6 text-sm">
          <p>An automation is "when this happens, do that". Make one here, or ask Niles:</p>
          <p className="text-foreground mt-2 italic">
            "Niles, when the office door opens, turn off the office lights."
          </p>
        </div>
      ) : (
        <ul className="flex flex-col gap-2">
          {rules.map((view) => (
            <AutomationCard
              key={view.rule.id}
              view={view}
              onEnabled={(enabled) => setEnabled.mutate({ rule: view.rule, enabled })}
            />
          ))}
        </ul>
      )}

      {setEnabled.error && <p className="text-destructive text-sm">{setEnabled.error.message}</p>}
    </div>
  );
}

/** What the server says a rule will not do the way it reads. A rule
    with one still runs: the check it is about may be meant. */
export function RuleWarnings({ warnings }: { warnings: string[] }) {
  if (warnings.length === 0) return null;
  return (
    <ul aria-label="Warnings" className="mt-1 flex flex-col gap-1">
      {warnings.map((warning) => (
        <li key={warning} className="text-muted-foreground flex gap-1.5 text-xs">
          <TriangleAlert aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          {warning}
        </li>
      ))}
    </ul>
  );
}

function AutomationCard({
  view,
  onEnabled,
}: {
  view: AutomationView;
  onEnabled: (enabled: boolean) => void;
}) {
  const { rule, source, sentence, error } = view;
  const stored = source === "stored";
  const body = (
    <>
      {rule.description.trim() && (
        <div className="text-muted-foreground truncate text-xs font-medium">{rule.description}</div>
      )}
      <div className={cn("text-sm", !rule.enabled && "text-muted-foreground")}>{sentence}</div>
      {(!rule.enabled || !stored) && (
        <div className="mt-1 flex flex-wrap gap-1.5">
          {!rule.enabled && <Badge variant="secondary">Paused</Badge>}
          {!stored && <Badge variant="outline">From config</Badge>}
        </div>
      )}
      {error && <p className="text-destructive mt-1 text-xs">{error}</p>}
      <RuleWarnings warnings={view.warnings} />
      {stored && <LastRun id={rule.id} />}
    </>
  );

  return (
    <li className="bg-card flex items-center gap-3 rounded-xl border px-4 py-3">
      {stored ? (
        <a
          href={`#/automations/${encodeURIComponent(rule.id)}`}
          className="focus-visible:ring-ring/50 -my-1 flex min-w-0 flex-1 items-center gap-2 rounded-md py-1 focus-visible:ring-3 focus-visible:outline-none"
        >
          <div className="min-w-0 flex-1">{body}</div>
          <ChevronRight className="text-muted-foreground size-4 shrink-0" />
        </a>
      ) : (
        // A rule from the config file is changed in the file: the page
        // writing to the database would only be overruled at the next
        // load.
        <div className="min-w-0 flex-1">{body}</div>
      )}
      {stored && (
        <Switch
          aria-label={`Run ${nameOf(view)}`}
          checked={rule.enabled}
          onCheckedChange={onEnabled}
        />
      )}
    </li>
  );
}

/** The newest run in one muted line, or nothing until there is one:
    the card is about the rule, and this is only a hint that it ran. */
function LastRun({ id }: { id: string }) {
  const runs = useRuns(id);
  const last = runs.data?.runs[0];
  if (!last) return null;
  return <p className="text-muted-foreground mt-1 truncate text-xs">{lastRunText(last)}</p>;
}
