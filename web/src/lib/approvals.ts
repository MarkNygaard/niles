import type { DecidedApproval, PendingApproval } from "@/lib/api";

/** Everything about approvals: what waits, and what was decided. A
    change on the stream marks both stale. */
export const APPROVALS = ["approvals"];
export const PENDING = [...APPROVALS, "pending"];
export const HISTORY = [...APPROVALS, "history"];

/** What became of one, as the history says it. */
export function decidedLabel(entry: DecidedApproval): string {
  switch (entry.status) {
    case "allowed":
      return entry.decision === "conversation" ? "Allowed for the conversation" : "Allowed once";
    case "failed":
      return "Allowed, but it didn't work";
    case "denied":
      return "Denied";
    case "lapsed":
      return "Lapsed";
  }
}

/** A time today, or the day for anything older: "15:34", "12 Oct". */
export function shortTime(iso: string, now: Date = new Date()): string {
  const at = new Date(iso);
  return at.toDateString() === now.toDateString()
    ? at.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
    : at.toLocaleDateString([], { day: "numeric", month: "short" });
}

/** "Allowed once · 15:34", "Denied by voice · 12 Oct". */
export function historyLine(entry: DecidedApproval, now: Date = new Date()): string {
  const by = entry.decided_via === "voice" ? " by voice" : "";
  return `${decidedLabel(entry)}${by} · ${shortTime(entry.decided_at ?? entry.asked_at, now)}`;
}

/** Where it was asked and how long it waits: "Asked by voice · waits until 15:39". */
export function waitingLine(approval: PendingApproval, now: Date = new Date()): string {
  const asked = approval.via === "voice" ? "Asked by voice" : `Asked ${shortTime(approval.asked_at, now)}`;
  return `${asked} · waits until ${shortTime(approval.expires_at, now)}`;
}

/** What the badge on the tab says: a count, until it stops being read as one. */
export function badgeText(count: number): string {
  return count > 9 ? "9+" : String(count);
}

/** The tag of an approval's notifications on a device, as Niles pushes
    them (crates/niles-bin/src/approvals/push.rs `tag`). */
export function approvalTag(id: string): string {
  return `approval-${id}`;
}
