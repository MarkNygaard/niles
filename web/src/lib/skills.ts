import type { Skill, SkillStatus } from "@/lib/api";

/** "evening-wind-down" as it is said: "Evening wind down". */
export function skillTitle(name: string): string {
  const spaced = name.replace(/-/g, " ");
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

export const STATUS_LABEL: Record<SkillStatus, string> = {
  active: "Active",
  stale: "Stale",
  archived: "Archived",
};

export const PROVENANCE_LABEL: Record<Skill["provenance"], string> = {
  "agent-created": "Learned by Niles",
  "user-created": "Asked for",
};

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

/**
 * How long ago, the way it is said: "just now", "5 minutes ago",
 * "yesterday", "3 weeks ago". A time a little ahead of this clock is
 * the server's clock disagreeing, and is "just now" too.
 */
export function relativeTime(iso: string, now: Date = new Date()): string {
  const seconds = Math.floor((now.getTime() - new Date(iso).getTime()) / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${plural(minutes, "minute")} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${plural(hours, "hour")} ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return "yesterday";
  if (days < 7) return `${days} days ago`;
  if (days < 30) return `${plural(Math.floor(days / 7), "week")} ago`;
  if (days < 365) return `${plural(Math.floor(days / 30), "month")} ago`;
  return `${plural(Math.floor(days / 365), "year")} ago`;
}

/** "Used 4 times · last 3 days ago", or "Never used · viewed
    yesterday": whether Niles reaches for it, which is what the curator
    goes by. */
export function activityText(skill: Skill, now: Date = new Date()): string {
  if (skill.last_used_at) {
    const n = Math.max(skill.usage_count, 1);
    const times = n === 1 ? "once" : `${n} times`;
    return `Used ${times} · last ${relativeTime(skill.last_used_at, now)}`;
  }
  const viewed = skill.last_viewed_at
    ? ` · viewed ${relativeTime(skill.last_viewed_at, now)}`
    : "";
  return `Never used${viewed}`;
}

/** The skills in use, and the archived ones, which the page keeps
    apart. Both keep the order they came in. */
export function splitArchived(skills: Skill[]): { current: Skill[]; archived: Skill[] } {
  return {
    current: skills.filter((s) => s.status !== "archived"),
    archived: skills.filter((s) => s.status === "archived"),
  };
}
