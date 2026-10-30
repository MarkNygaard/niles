/** The restart answer, and the waiting that follows asking for one. */
import { useSyncExternalStore } from "react";
import { ApiError, type RestartStatus } from "@/lib/api";

/** TanStack Query key for `GET /system/restart`. */
export const RESTART = ["restart"];

export const POLL_MS = 2_000;
export const GIVE_UP_MS = 120_000;
const ANSWER_MS = 5_000;

/** Whether the app may offer Restart now: something starts Niles again,
    the overrides would survive, and nobody asked yet. */
export function canRestart(status: RestartStatus | undefined): boolean {
  return status !== undefined && Boolean(status.supervisor) && status.persistent && !status.restarting;
}

/** What a restart cuts short, in the order the confirmation names them.
    Under Kubernetes the pod may also come back as a newer image. */
export function restartNotes(status: RestartStatus): string[] {
  const notes: string[] = [];
  if (status.cut_short.jobs === 1) {
    notes.push("A background job the assistant is doing stops before it finishes.");
  } else if (status.cut_short.jobs > 1) {
    notes.push(`${status.cut_short.jobs} background jobs the assistant is doing stop before they finish.`);
  }
  if (status.cut_short.approvals === 1) {
    notes.push("An approval waiting for a yes lapses.");
  } else if (status.cut_short.approvals > 1) {
    notes.push(`${status.cut_short.approvals} approvals waiting for a yes lapse.`);
  }
  if (status.cut_short.wake_ups === 1) {
    notes.push("A wake-up ramp under way pauses, and carries on when Niles is back.");
  } else if (status.cut_short.wake_ups > 1) {
    notes.push(
      `${status.cut_short.wake_ups} wake-up ramps under way pause, and carry on when Niles is back.`,
    );
  }
  if (status.supervisor === "kubernetes") {
    notes.push(
      "Kubernetes may start a newer image of Niles, if one was published since this one started.",
    );
  }
  return notes;
}

/** Where to look when Niles does not come back. The command part is
    rendered in `<code>`; the text says what to do with it. */
export function logHint(supervisor: string | null): { text: string; command?: string } {
  switch (supervisor) {
    case "kubernetes":
      return { text: "See what it says with", command: "kubectl logs deploy/niles" };
    case "docker":
      return { text: "See what it says with", command: "docker compose logs niles" };
    case "systemd":
      return { text: "See what it says with", command: "journalctl -u niles" };
    default:
      return { text: "Look at its log where it runs." };
  }
}

export type Outcome = "back" | "gave_up" | "aborted";

interface WaitOptions {
  status: () => Promise<RestartStatus>;
  sleep?: (ms: number) => Promise<void>;
  now?: () => number;
  signal?: AbortSignal;
  giveUpMs?: number;
}

const defaultSleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Poll until a *different* Niles answers. The same boot id, a 502 from
    the ingress, a network error, or five silent seconds: keep waiting.
    A 401 means a new Niles answered and the session is gone — that is
    "back" too, and the reload shows sign-in. */
export async function waitForNewBoot(before: string, options: WaitOptions): Promise<Outcome> {
  const { status, signal, giveUpMs = GIVE_UP_MS } = options;
  const sleep = options.sleep ?? defaultSleep;
  const now = options.now ?? Date.now;
  const start = now();
  for (;;) {
    if (signal?.aborted) return "aborted";
    if (now() - start >= giveUpMs) return "gave_up";
    const answer = await Promise.race([
      status().then(
        (s) => s,
        (e: unknown) => e,
      ),
      sleep(ANSWER_MS).then(() => undefined),
    ]);
    if (signal?.aborted) return "aborted";
    if (isStatus(answer)) {
      if (answer.boot.id !== before) return "back";
    } else if (answer instanceof ApiError && answer.status === 401) {
      return "back";
    }
    await sleep(POLL_MS);
  }
}

function isStatus(value: unknown): value is RestartStatus {
  return (
    typeof value === "object" &&
    value !== null &&
    "boot" in value &&
    typeof (value as RestartStatus).boot?.id === "string"
  );
}

/** What the waiting screen needs to know: which boot id is going away,
    and what started it (for the log hint if the next one never
    answers). */
export interface Waiting {
  before: string;
  supervisor: string | null;
}

type Listener = () => void;

let waiting: Waiting | null = null;
const listeners = new Set<Listener>();

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Start waiting for the next Niles. While one wait is under way another
    start is ignored: every open app waits, but on the first ask. */
export function startWaiting(next: Waiting): void {
  if (waiting) return;
  waiting = next;
  listeners.forEach((listener) => listener());
}

/** Stop waiting. Used by tests; a real wait ends in a reload. */
export function stopWaiting(): void {
  waiting = null;
  listeners.forEach((listener) => listener());
}

export function useRestarting(): Waiting | null {
  return useSyncExternalStore(subscribe, () => waiting);
}
