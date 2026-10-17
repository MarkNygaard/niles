import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { AUTOMATIONS, RuleWarnings, nameOf, useSetEnabled } from "@/components/AutomationsPage";
import { FlowCanvas } from "@/components/automations/FlowCanvas";
import { Inspector } from "@/components/automations/Inspector";
import { Palette } from "@/components/automations/Palette";
import { RecentRuns } from "@/components/automations/RecentRuns";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { api } from "@/lib/api";
import type { AutomationView, Device, Rule } from "@/lib/api";
import {
  addAction,
  addCondition,
  addTrigger,
  defaultAction,
  defaultCondition,
  defaultTrigger,
  newRule,
  problems,
  withTriggerIds,
  withoutStep,
} from "@/lib/automations";
import type { Draft } from "@/lib/automations";
import { cn } from "@/lib/utils";

const LIST = "#/automations";

function backToList() {
  window.location.hash = LIST;
}

export interface AutomationEditorProps {
  /** The rule to change; left out, a new one. */
  id?: string;
}

/**
 * One rule, drawn as a flow and edited beside it.
 *
 * On a phone the flow is only shown: building it wants a pointer and
 * room for the side panel, and the list already pauses and the voice
 * already makes. Pausing and removing are still here, since this is
 * where a tap on the list lands.
 */
export function AutomationEditor({ id }: AutomationEditorProps) {
  const list = useQuery({ queryKey: AUTOMATIONS, queryFn: api.automations });
  const devices = useQuery({ queryKey: ["devices"], queryFn: api.devices });
  const phone = useMediaQuery("(max-width: 639px)");

  if (list.isPending || (!id && devices.isPending)) {
    return <Skeleton className="h-96 w-full" />;
  }
  if (list.isError) {
    return <p className="text-destructive text-sm">{list.error.message}</p>;
  }
  const view = id ? list.data.rules.find((v) => v.rule.id === id) : undefined;
  if (id && !view) {
    return (
      <p className="text-muted-foreground text-sm">
        There is no such automation any more.{" "}
        <a href={LIST} className="text-foreground underline underline-offset-4">
          Back to the list
        </a>
      </p>
    );
  }
  if (phone || view?.source === "config") {
    return <Viewer view={view} />;
  }
  // Keyed, so moving from one rule to another starts from that rule
  // rather than carrying the last one's edits over.
  return <Builder key={id ?? "new"} view={view} devices={devices.data ?? []} />;
}

/** The flow, looked at but not changed. */
function Viewer({ view }: { view?: AutomationView }) {
  const setEnabled = useSetEnabled();
  const [removing, setRemoving] = useState(false);
  if (!view) {
    return (
      <p className="text-muted-foreground text-sm">
        Automations are built on a wider screen, or by asking Niles: "Niles, when the office door
        opens, turn off the office lights."
      </p>
    );
  }
  const stored = view.source === "stored";
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1">
        {view.rule.description.trim() && (
          <h2 className="text-sm font-medium">{view.rule.description}</h2>
        )}
        <p className="text-sm">{view.sentence}</p>
        {!stored && <Badge variant="outline">From config</Badge>}
        {view.error && <p className="text-destructive text-xs">{view.error}</p>}
        <RuleWarnings warnings={view.warnings} />
      </div>
      <div className="bg-card h-[55dvh] min-h-72 overflow-hidden rounded-xl border">
        <FlowCanvas rule={view.rule} />
      </div>
      <p className="text-muted-foreground text-xs">
        {stored
          ? "Open on a wider screen to edit."
          : "This one is written in the config file, and is changed there."}
      </p>
      {stored && <RecentRuns id={view.rule.id} triggers={view.rule.triggers} />}
      {stored && (
        <div className="flex items-center gap-3">
          {/* Not a <label>: Base UI names a switch after the label it
              sits in, and "Running" says less than the rule's name. */}
          <div className="flex flex-1 items-center gap-2 text-sm">
            <Switch
              aria-label={`Run ${nameOf(view)}`}
              checked={view.rule.enabled}
              onCheckedChange={(enabled) => setEnabled.mutate({ rule: view.rule, enabled })}
            />
            {view.rule.enabled ? "Running" : "Paused"}
          </div>
          <Button variant="destructive" onClick={() => setRemoving(true)}>
            <Trash2 />
            Delete
          </Button>
          <ConfirmDelete view={removing ? view : undefined} onClose={() => setRemoving(false)} />
        </div>
      )}
      {setEnabled.error && <p className="text-destructive text-sm">{setEnabled.error.message}</p>}
    </div>
  );
}

function Builder({ view, devices }: { view?: AutomationView; devices: Device[] }) {
  const queryClient = useQueryClient();
  // Taken once: a refetch of the list while somebody edits must not
  // throw their edits away.
  const [rule, setRule] = useState<Draft>(() => withTriggerIds(view?.rule ?? newRule(devices)));
  const [selected, setSelected] = useState<string | null>("trigger-0");
  const [removing, setRemoving] = useState(false);

  const save = useMutation({
    mutationFn: (draft: Draft) =>
      draft.id ? api.updateAutomation(draft as Rule) : api.createAutomation(draft),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: AUTOMATIONS });
      backToList();
    },
  });

  const change = (next: Draft, select: string | null) => {
    setRule(next);
    setSelected(select);
    save.reset();
  };
  const found = problems(rule);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-end gap-3">
        <label className="flex min-w-48 flex-1 flex-col gap-1.5">
          <span className="text-muted-foreground text-xs">Name</span>
          <Input
            aria-label="Name"
            placeholder="Office lights off when the garden door opens"
            value={rule.description}
            onChange={(e) => change({ ...rule, description: e.target.value }, selected)}
          />
        </label>
        <label className="flex h-8 items-center gap-2 text-sm">
          <Switch
            aria-label="Enabled"
            checked={rule.enabled}
            onCheckedChange={(enabled) => change({ ...rule, enabled }, selected)}
          />
          Enabled
        </label>
        <div className="flex gap-2">
          {view && (
            <Button variant="destructive" onClick={() => setRemoving(true)}>
              <Trash2 />
              Delete
            </Button>
          )}
          <a href={LIST} className={cn(buttonVariants({ variant: "outline" }))}>
            Cancel
          </a>
          <Button disabled={found.length > 0 || save.isPending} onClick={() => save.mutate(rule)}>
            {save.isPending ? "Saving…" : "Save"}
          </Button>
        </div>
      </div>

      {save.error ? (
        <p role="alert" className="text-destructive text-sm">
          {save.error.message}
        </p>
      ) : (
        found.length > 0 && <p className="text-muted-foreground text-sm">{found[0]}</p>
      )}
      {/* The saved rule's: the server says them of what it has. */}
      {view && <RuleWarnings warnings={view.warnings} />}

      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_20rem]">
        <div className="bg-card relative h-[60vh] min-h-96 overflow-hidden rounded-xl border">
          <FlowCanvas
            rule={rule}
            selected={selected}
            onSelect={setSelected}
            onRemove={(id) => change(withoutStep(rule, id), null)}
          />
          <div className="absolute top-3 left-3 z-10">
            <Palette
              onAddTrigger={(kind) => {
                const added = addTrigger(rule, defaultTrigger(kind, devices));
                change(added.rule, added.id);
              }}
              onAddCondition={(kind) => {
                const added = addCondition(rule, defaultCondition(kind, devices, rule.triggers));
                change(added.rule, added.id);
              }}
              onAddAction={(kind) => {
                const added = addAction(rule, defaultAction(kind, devices, rule.triggers));
                change(added.rule, added.id);
              }}
            />
          </div>
        </div>
        <div className="flex flex-col gap-4">
          <aside aria-label="Selected step" className="bg-card rounded-xl border p-4">
            <Inspector rule={rule} selected={selected} devices={devices} onChange={change} />
          </aside>
          {/* The saved rule's triggers, not the draft's: those are the
              ones its runs name. */}
          {view && (
            <div className="bg-card rounded-xl border p-4">
              <RecentRuns id={view.rule.id} triggers={view.rule.triggers} />
            </div>
          )}
        </div>
      </div>

      {view && (
        <ConfirmDelete view={removing ? view : undefined} onClose={() => setRemoving(false)} />
      )}
    </div>
  );
}

function ConfirmDelete({ view, onClose }: { view?: AutomationView; onClose: () => void }) {
  const queryClient = useQueryClient();
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteAutomation(id),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: AUTOMATIONS });
      onClose();
      backToList();
    },
  });
  return (
    <Dialog
      open={view !== undefined}
      onOpenChange={(open: boolean) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="gap-0 p-0">
        {view && (
          <>
            <div className="border-border border-b px-4 py-3">
              <DialogTitle>Delete this automation?</DialogTitle>
              <DialogDescription>{view.sentence}</DialogDescription>
            </div>
            <DialogBody className="flex flex-col gap-3">
              {remove.error && <p className="text-destructive text-sm">{remove.error.message}</p>}
              <div className="flex justify-end gap-2">
                <Button variant="outline" onClick={onClose}>
                  Keep it
                </Button>
                <Button
                  variant="destructive"
                  disabled={remove.isPending}
                  onClick={() => remove.mutate(view.rule.id)}
                >
                  Delete
                </Button>
              </div>
            </DialogBody>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
