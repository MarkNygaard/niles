import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import type { Schedule, ScheduleDraft, ScheduleRun, ScheduleWhen, Weekday } from "@/lib/api";

export const MY_SCHEDULES = ["me-schedules"];

export const WEEK: Weekday[] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** Every N hours, evenly over the day: what the server takes. */
export const HOURS = [1, 2, 3, 4, 6, 8, 12];

const PATTERNS: { kind: ScheduleWhen["kind"]; label: string }[] = [
  { kind: "daily", label: "Every day" },
  { kind: "weekdays", label: "Weekdays" },
  { kind: "days", label: "Chosen days" },
  { kind: "every_hours", label: "Every few hours" },
];

/** When a schedule runs, in words: "Mon and Wed at 08:00". */
export function describeWhen(when: ScheduleWhen): string {
  switch (when.kind) {
    case "daily":
      return `Every day at ${when.at}`;
    case "weekdays":
      return `Weekdays at ${when.at}`;
    case "days": {
      const days = when.days;
      const listed =
        days.length > 1 ? `${days.slice(0, -1).join(", ")} and ${days[days.length - 1]}` : days[0];
      return `${listed} at ${when.at}`;
    }
    case "every_hours":
      return when.hours === 1 ? "Every hour" : `Every ${when.hours} hours`;
  }
}

/** What became of a run, in words. */
export const OUTCOME: Record<ScheduleRun["outcome"], string> = {
  running: "Running",
  done: "Done",
  waiting: "Waiting for your yes",
  failed: "Failed",
  missed: "Missed",
};

/** The form's state, whichever pattern is picked. */
export interface Form {
  name: string;
  instruction: string;
  kind: ScheduleWhen["kind"];
  at: string;
  days: Weekday[];
  hours: number;
}

export function formOf(schedule?: Schedule): Form {
  const when = schedule?.when;
  return {
    name: schedule?.name ?? "",
    instruction: schedule?.instruction ?? "",
    kind: when?.kind ?? "daily",
    at: when && "at" in when ? when.at : "08:00",
    days: when?.kind === "days" ? when.days : ["Mon"],
    hours: when?.kind === "every_hours" ? when.hours : 6,
  };
}

/** The form as the server takes it. */
export function draftOf(form: Form): ScheduleDraft {
  const when: ScheduleWhen =
    form.kind === "every_hours"
      ? { kind: "every_hours", hours: form.hours }
      : form.kind === "days"
        ? { kind: "days", days: WEEK.filter((d) => form.days.includes(d)), at: form.at }
        : { kind: form.kind, at: form.at };
  return { name: form.name.trim(), instruction: form.instruction.trim(), when };
}

/** A time as the person reads it: "Mon 17 Nov, 08:00". */
export function shownAt(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * My profile → Schedules: the person's own agent tasks, which the
 * assistant answers on time, as them, in a new conversation in their
 * chat. Theirs alone; never spoken on a satellite.
 */
export function SchedulesPanel() {
  const queryClient = useQueryClient();
  const list = useQuery({ queryKey: MY_SCHEDULES, queryFn: api.schedules, retry: false });
  const settle = () => queryClient.invalidateQueries({ queryKey: MY_SCHEDULES });
  const [editing, setEditing] = useState<Schedule | "new" | null>(null);
  const [removing, setRemoving] = useState<Schedule | null>(null);
  const [history, setHistory] = useState<Schedule | null>(null);
  const [started, setStarted] = useState<number | null>(null);

  const pause = useMutation({
    mutationFn: (v: { id: number; paused: boolean }) => api.pauseSchedule(v.id, v.paused),
    onSettled: settle,
  });
  const run = useMutation({
    mutationFn: api.runSchedule,
    onSuccess: (_, id) => setStarted(id),
  });
  const remove = useMutation({
    mutationFn: api.removeSchedule,
    onSuccess: () => setRemoving(null),
    onSettled: settle,
  });

  if (list.isPending) return <Skeleton className="h-10 w-full" />;
  if (list.isError) return <p className="text-destructive text-sm">{list.error.message}</p>;

  const { schedules, assistant } = list.data;
  return (
    <>
      <div className="flex flex-col gap-4">
        {!assistant && (
          <p className="text-sm">
            Schedules run on Claude or ChatGPT. Until one is switched on under Settings → Voice
            &amp; AI, they don't run.
          </p>
        )}
        {(run.error ?? pause.error) && (
          <p className="text-destructive text-sm">{(run.error ?? pause.error)?.message}</p>
        )}
        {schedules.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            No schedules yet. Add one here, or ask in the chat: "every Monday at 8, my open Linear
            issues by priority".
          </p>
        ) : (
          <ul aria-label="Your schedules" className="flex flex-col gap-5">
            {schedules.map((s) => (
              <li key={s.id} className="flex flex-col gap-1.5">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{s.name}</span>
                  {s.paused && <Badge variant="outline">Paused</Badge>}
                </div>
                <p className="text-sm">{describeWhen(s.when)}</p>
                <p className="text-muted-foreground text-sm whitespace-pre-line">
                  {s.instruction}
                </p>
                {s.next_due && (
                  <p className="text-muted-foreground text-xs">Next: {shownAt(s.next_due)}</p>
                )}
                {started === s.id && (
                  <p className="text-xs">Running now. It lands as a new conversation in the chat.</p>
                )}
                <div className="flex flex-wrap gap-2 pt-1">
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`Run ${s.name} now`}
                    disabled={!assistant || run.isPending}
                    onClick={() => run.mutate(s.id)}
                  >
                    Run now
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`${s.paused ? "Resume" : "Pause"} ${s.name}`}
                    disabled={pause.isPending}
                    onClick={() => pause.mutate({ id: s.id, paused: !s.paused })}
                  >
                    {s.paused ? "Resume" : "Pause"}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`Runs of ${s.name}`}
                    onClick={() => setHistory(s)}
                  >
                    Runs
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`Edit ${s.name}`}
                    onClick={() => setEditing(s)}
                  >
                    Edit
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`Remove ${s.name}`}
                    onClick={() => setRemoving(s)}
                  >
                    Remove
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
        <div>
          <Button onClick={() => setEditing("new")}>Add a schedule</Button>
        </div>
      </div>
      <Dialog
        open={editing !== null}
        onOpenChange={(next: boolean) => {
          if (!next) setEditing(null);
        }}
      >
        <DialogContent className="gap-0 p-0">
          <div className="border-border border-b px-4 py-3">
            <DialogTitle>{editing === "new" ? "Add a schedule" : `Edit ${editing?.name}`}</DialogTitle>
            <DialogDescription>
              Niles's assistant answers it as you, with your own tools, and tells you when it's
              done.
            </DialogDescription>
          </div>
          <DialogBody className="flex flex-col gap-3">
            {editing !== null && (
              <ScheduleForm
                schedule={editing === "new" ? undefined : editing}
                onSaved={() => {
                  setEditing(null);
                  void settle();
                }}
                onCancel={() => setEditing(null)}
              />
            )}
          </DialogBody>
        </DialogContent>
      </Dialog>
      <Dialog
        open={history !== null}
        onOpenChange={(next: boolean) => {
          if (!next) setHistory(null);
        }}
      >
        <DialogContent className="gap-0 p-0">
          <div className="border-border border-b px-4 py-3">
            <DialogTitle>Runs of {history?.name}</DialogTitle>
            <DialogDescription>The latest first.</DialogDescription>
          </div>
          <DialogBody>{history && <Runs schedule={history} />}</DialogBody>
        </DialogContent>
      </Dialog>
      <Dialog
        open={removing !== null}
        onOpenChange={(next: boolean) => {
          if (!next) setRemoving(null);
        }}
      >
        <DialogContent className="gap-0 p-0">
          <div className="border-border border-b px-4 py-3">
            <DialogTitle>Remove {removing?.name}?</DialogTitle>
            <DialogDescription>
              It stops running, and its runs go with it. The conversations it started stay in the
              chat.
            </DialogDescription>
          </div>
          <DialogBody className="flex justify-end gap-2">
            <Button variant="outline" onClick={() => setRemoving(null)}>
              Keep it
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => removing && remove.mutate(removing.id)}
            >
              Remove
            </Button>
          </DialogBody>
        </DialogContent>
      </Dialog>
    </>
  );
}

function ScheduleForm({
  schedule,
  onSaved,
  onCancel,
}: {
  schedule?: Schedule;
  onSaved: () => void;
  onCancel: () => void;
}) {
  const [form, setForm] = useState<Form>(() => formOf(schedule));
  const set = (patch: Partial<Form>) => setForm((f) => ({ ...f, ...patch }));
  const save = useMutation({
    mutationFn: (draft: ScheduleDraft) =>
      schedule ? api.updateSchedule(schedule.id, draft) : api.addSchedule(draft),
    onSuccess: onSaved,
  });
  const ready =
    form.name.trim() !== "" &&
    form.instruction.trim() !== "" &&
    (form.kind !== "days" || form.days.length > 0);
  const toggle = (day: Weekday) =>
    set({ days: form.days.includes(day) ? form.days.filter((d) => d !== day) : [...form.days, day] });

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate(draftOf(form));
      }}
    >
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="schedule-name">Name</Label>
        <Input
          id="schedule-name"
          value={form.name}
          maxLength={60}
          placeholder="Linear on Monday"
          onChange={(e) => set({ name: e.target.value })}
        />
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="schedule-instruction">What Niles should do</Label>
        <Textarea
          id="schedule-instruction"
          value={form.instruction}
          maxLength={2000}
          rows={4}
          placeholder="My open Linear issues, by priority"
          onChange={(e) => set({ instruction: e.target.value })}
        />
      </div>
      <div role="group" aria-label="Repeats" className="flex flex-wrap gap-1">
        {PATTERNS.map(({ kind, label }) => (
          <Button
            key={kind}
            type="button"
            size="sm"
            variant={form.kind === kind ? "secondary" : "ghost"}
            aria-pressed={form.kind === kind}
            onClick={() => set({ kind })}
          >
            {label}
          </Button>
        ))}
      </div>
      {form.kind === "days" && (
        <div role="group" aria-label="Days" className="flex flex-wrap gap-1">
          {WEEK.map((day) => (
            <Button
              key={day}
              type="button"
              size="sm"
              variant={form.days.includes(day) ? "secondary" : "ghost"}
              aria-pressed={form.days.includes(day)}
              onClick={() => toggle(day)}
            >
              {day}
            </Button>
          ))}
        </div>
      )}
      {form.kind === "every_hours" ? (
        <div role="group" aria-label="Every how many hours" className="flex flex-wrap gap-1">
          {HOURS.map((hours) => (
            <Button
              key={hours}
              type="button"
              size="sm"
              variant={form.hours === hours ? "secondary" : "ghost"}
              aria-pressed={form.hours === hours}
              onClick={() => set({ hours })}
            >
              {hours}h
            </Button>
          ))}
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="schedule-at">At</Label>
          <Input
            id="schedule-at"
            type="time"
            value={form.at}
            className="w-32"
            onChange={(e) => set({ at: e.target.value })}
          />
        </div>
      )}
      {save.error && <p className="text-destructive text-sm">{save.error.message}</p>}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" disabled={!ready || save.isPending}>
          {schedule ? "Save" : "Add"}
        </Button>
      </div>
    </form>
  );
}

function Runs({ schedule }: { schedule: Schedule }) {
  const runs = useQuery({
    queryKey: [...MY_SCHEDULES, schedule.id, "runs"],
    queryFn: () => api.scheduleRuns(schedule.id),
  });
  if (runs.isPending) return <Skeleton className="h-10 w-full" />;
  if (runs.isError) return <p className="text-destructive text-sm">{runs.error.message}</p>;
  if (runs.data.runs.length === 0) {
    return <p className="text-muted-foreground text-sm">It hasn't run yet.</p>;
  }
  return (
    <ul aria-label={`Runs of ${schedule.name}`} className="flex flex-col gap-3">
      {runs.data.runs.map((r) => (
        <li key={r.id} className="flex flex-col gap-0.5">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm">{shownAt(r.due)}</span>
            <Badge variant={r.outcome === "failed" ? "destructive" : "outline"}>
              {OUTCOME[r.outcome]}
            </Badge>
            {r.conversation !== null && (
              <a className="text-sm underline" href={`#/chat?conversation=${r.conversation}`}>
                Open
              </a>
            )}
          </div>
          {r.reason && <p className="text-muted-foreground text-xs">{r.reason}</p>}
        </li>
      ))}
    </ul>
  );
}
