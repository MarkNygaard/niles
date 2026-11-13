/** The update answer, and the dot and the self-reload that follow it. */
import type { RestartStatus, UpdateStatus } from "@/lib/api";

/** TanStack Query key for `GET /system/update`. */
export const UPDATE = ["update"];

/** Where Niles's source lives; the Rust side names the same address. */
export const REPO_URL = "https://github.com/MarkNygaard/niles";

/** Why the Settings entry carries a dot, for a screen reader. */
export const RESTART_WAITING = "a restart is waiting";
export const UPDATE_AVAILABLE = "an update is available";

/** Why Settings carries a dot, if it does: a restart waits, an update
    is available, both, or neither. */
export function settingsDotFor(
  restart?: RestartStatus,
  update?: UpdateStatus,
): string | undefined {
  const restartWaiting = (restart?.pending.length ?? 0) > 0;
  const updateAvailable = update?.state === "update_available";
  if (restartWaiting && updateAvailable) {
    return "a restart is waiting and an update is available";
  }
  if (restartWaiting) return RESTART_WAITING;
  if (updateAvailable) return UPDATE_AVAILABLE;
  return undefined;
}

/** One step of "how to update where Niles runs": what to do, the
    command to copy when there is one, and whether it is the restart
    this page can start itself. */
export interface Step {
  text: string;
  command?: string;
  label?: string;
  restart?: boolean;
}

/** How to update where Niles runs, for the supervisor this install
    answers with. Empty unless an update is available. */
export function howToUpdate(update: UpdateStatus, supervisor: string | null): Step[] {
  if (update.state !== "update_available" || !update.newest) return [];
  const tag = update.newest.version ?? `sha-${update.newest.commit.slice(0, 7)}`;
  const pin: Step = {
    text: `On a pinned tag, change it to ${tag}, or pin the newest image by its digest, then apply.`,
    command: update.newest.image,
    label: "Newest image by digest",
  };
  switch (supervisor) {
    case "kubernetes": {
      const ns = update.namespace ? `-n ${update.namespace} ` : "";
      return [
        {
          text: "On the latest tag with imagePullPolicy Always, as the shipped manifests run Niles, a restart starts the newest image.",
          restart: true,
          command: `kubectl ${ns}rollout restart deployment/niles`,
          label: "Command to restart the deployment",
        },
        pin,
      ];
    }
    case "docker":
      return [
        {
          text: "Where compose.yaml is, pull the newest image and start Niles again.",
          command: "docker compose pull niles && docker compose up -d niles",
          label: "Commands to update with Docker Compose",
        },
        {
          text: `If NILES_IMAGE pins a tag, change it to ${tag} first, or pin the newest image by its digest.`,
          command: update.newest.image,
          label: "Newest image by digest",
        },
      ];
    default:
      return [
        {
          text: "Pull the newest image and start Niles again where it runs.",
          command: update.newest.image,
          label: "Newest image by digest",
        },
      ];
  }
}

/** The reload-once state. A page's script is the build that served it:
    when another build answers, the page loads that build's script,
    once. `undefined` means no answer yet; `null` is a real answer (a
    development build) and is adopted like any other. */
let seen: string | null | undefined;
let reloading = false;

/** The build answering now. The first answer is adopted; a different
    later one reloads, and the flag keeps exactly one reload per page. */
export function noticeBuild(commit: string | null, reload: () => void): void {
  if (seen === undefined) {
    seen = commit;
    return;
  }
  if (commit === seen || reloading) return;
  reloading = true;
  reload();
}

/** Forget which build was seen, for a fresh page. Used by tests; a
    real page forgets by reloading. */
export function forgetBuild(): void {
  seen = undefined;
  reloading = false;
}
