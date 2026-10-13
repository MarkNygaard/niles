import { describe, expect, it } from "vitest";
import { activityText, relativeTime, skillTitle, splitArchived } from "./skills";
import type { Skill } from "./api";

const now = new Date("2026-10-13T12:00:00Z");

function ago(ms: number): string {
  return new Date(now.getTime() - ms).toISOString();
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

const skill: Skill = {
  name: "evening-wind-down",
  description: "Dim the house in steps before bed.",
  version: "0.1.0",
  status: "active",
  provenance: "agent-created",
  pinned: false,
  created_at: ago(20 * DAY),
  last_used_at: null,
  last_viewed_at: null,
  last_patched_at: null,
  usage_count: 0,
  view_count: 0,
  patch_count: 0,
};

describe("skillTitle", () => {
  it("turns hyphens into spaces and capitalises the first letter", () => {
    expect(skillTitle("evening-wind-down")).toBe("Evening wind down");
    expect(skillTitle("tv")).toBe("Tv");
  });
});

describe("relativeTime", () => {
  it("says how long ago, in the largest whole unit", () => {
    expect(relativeTime(ago(10_000), now)).toBe("just now");
    expect(relativeTime(ago(MINUTE), now)).toBe("1 minute ago");
    expect(relativeTime(ago(5 * MINUTE), now)).toBe("5 minutes ago");
    expect(relativeTime(ago(3 * HOUR), now)).toBe("3 hours ago");
    expect(relativeTime(ago(DAY), now)).toBe("yesterday");
    expect(relativeTime(ago(4 * DAY), now)).toBe("4 days ago");
    expect(relativeTime(ago(15 * DAY), now)).toBe("2 weeks ago");
    expect(relativeTime(ago(95 * DAY), now)).toBe("3 months ago");
    expect(relativeTime(ago(800 * DAY), now)).toBe("2 years ago");
  });

  it("takes a time slightly ahead as just now", () => {
    expect(relativeTime(ago(-30_000), now)).toBe("just now");
  });
});

describe("activityText", () => {
  it("says how often and when a skill was last used", () => {
    expect(activityText({ ...skill, last_used_at: ago(3 * DAY), usage_count: 4 }, now)).toBe(
      "Used 4 times · last 3 days ago",
    );
    expect(activityText({ ...skill, last_used_at: ago(2 * HOUR), usage_count: 1 }, now)).toBe(
      "Used once · last 2 hours ago",
    );
  });

  it("says never used, and when it was last looked at", () => {
    expect(activityText(skill, now)).toBe("Never used");
    expect(activityText({ ...skill, last_viewed_at: ago(DAY) }, now)).toBe(
      "Never used · viewed yesterday",
    );
  });
});

describe("splitArchived", () => {
  it("keeps archived skills apart, in order", () => {
    const stale: Skill = { ...skill, name: "b", status: "stale" };
    const archived: Skill = { ...skill, name: "c", status: "archived" };
    expect(splitArchived([skill, archived, stale])).toEqual({
      current: [skill, stale],
      archived: [archived],
    });
  });
});
