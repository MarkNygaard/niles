import { describe, expect, it } from "vitest";
import {
  countdown,
  currentWeek,
  dayLabel,
  daysBetween,
  localDay,
  marksFor,
  measures,
  minutes,
  planOf,
  planText,
  weekEnds,
  weekRange,
  weekTotals,
  withMark,
} from "@/lib/trainingPlan";
import type { Plan } from "@/lib/trainingPlan";

/** Three weeks to a half on Sunday 8 November; the first starts on a
    Thursday. */
const PLAN: Plan = {
  goal: { race: "Copenhagen Half", distance_km: 21.1, date: "2026-11-08", target_time: "1:55:00" },
  weeks: [
    {
      starts: "2026-10-22",
      focus: "Base",
      sessions: [
        {
          date: "2026-10-22",
          type: "easy",
          title: "Easy run",
          distance_km: 6,
          status: "done",
          actual: { distance_km: 6.4 },
        },
        { date: "2026-10-24", type: "rest", title: "Rest", status: "planned" },
        { date: "2026-10-25", type: "long", title: "Long run", distance_km: 14, status: "missed" },
      ],
    },
    {
      starts: "2026-10-26",
      sessions: [
        { date: "2026-10-27", type: "tempo", title: "Tempo", distance_km: 8, status: "done" },
        { date: "2026-11-01", type: "long", title: "Long run", distance_km: 16, status: "planned" },
      ],
    },
    {
      starts: "2026-11-02",
      focus: "Taper",
      sessions: [
        { date: "2026-11-08", type: "race", title: "Race", distance_km: 21.1, status: "planned" },
      ],
    },
  ],
  notes: "Assumed four runs a week.",
};

describe("weeks", () => {
  it("end on the Sunday they reach", () => {
    expect(weekEnds(PLAN.weeks[0])).toBe("2026-10-25");
    expect(weekEnds(PLAN.weeks[1])).toBe("2026-11-01");
    expect(weekEnds({ starts: "2026-11-08", sessions: [] })).toBe("2026-11-08");
  });

  it("are named by their days, across a month too", () => {
    expect(weekRange(PLAN.weeks[0])).toBe("22–25 Oct");
    expect(weekRange(PLAN.weeks[1])).toBe("26 Oct – 1 Nov");
    expect(weekRange({ starts: "2026-11-08", sessions: [] })).toBe("8 Nov");
  });
});

describe("currentWeek", () => {
  it("is the week today is in", () => {
    expect(currentWeek(PLAN, "2026-10-22")).toBe(0);
    expect(currentWeek(PLAN, "2026-10-25")).toBe(0);
    expect(currentWeek(PLAN, "2026-10-26")).toBe(1);
    expect(currentWeek(PLAN, "2026-11-08")).toBe(2);
  });

  it("is the first before the plan and the last after it", () => {
    expect(currentWeek(PLAN, "2026-10-01")).toBe(0);
    expect(currentWeek(PLAN, "2026-12-01")).toBe(2);
  });
});

describe("countdown", () => {
  it("counts weeks while there are two or more, then days", () => {
    expect(countdown("2026-12-13", "2026-10-16")).toBe("8 weeks to go");
    expect(countdown("2026-11-08", "2026-10-26")).toBe("13 days to go");
    expect(countdown("2026-11-08", "2026-11-07")).toBe("Tomorrow");
    expect(countdown("2026-11-08", "2026-11-08")).toBe("Race day");
    expect(countdown("2026-11-08", "2026-11-09")).toBe("Race done");
  });

  it("is not thrown by a change of clocks", () => {
    expect(daysBetween("2026-10-24", "2026-10-26")).toBe(2);
    expect(daysBetween("2026-03-28", "2026-03-30")).toBe(2);
  });
});

describe("weekTotals", () => {
  it("adds up what was planned and what was run", () => {
    expect(weekTotals(PLAN.weeks[0])).toEqual({ planned: 20, done: 6.4 });
    expect(weekTotals(PLAN.weeks[1])).toEqual({ planned: 24, done: 8 });
    expect(weekTotals({ starts: "2026-11-02", sessions: [] })).toEqual({ planned: 0, done: 0 });
  });
});

describe("labels", () => {
  it("say a day, a time and a session's numbers the way a runner does", () => {
    expect(dayLabel("2026-12-13")).toBe("Sun 13 Dec");
    expect(minutes(40)).toBe("40 min");
    expect(minutes(95)).toBe("1 h 35 min");
    expect(minutes(120)).toBe("2 h");
    expect(measures({ distance_km: 6, duration_min: 40, pace: "6:30" })).toBe(
      "6 km · 40 min · 6:30 /km",
    );
    expect(measures({ pace: "5:30-5:45" })).toBe("5:30-5:45 /km");
    expect(measures({})).toBe("");
  });

  it("take today from the device's own calendar", () => {
    expect(localDay(new Date(2026, 9, 16, 23, 30))).toBe("2026-10-16");
  });
});

describe("planOf", () => {
  it("reads a plan and refuses what is not one", () => {
    expect(planOf(PLAN)?.weeks).toHaveLength(3);
    expect(planOf({ markdown: "- a" })).toBeUndefined();
    expect(planOf(null)).toBeUndefined();
    expect(planOf({ goal: { date: "2026-11-08" }, weeks: [{ starts: "2026-11-02" }] })?.weeks[0].sessions).toEqual(
      [],
    );
  });
});

describe("planText", () => {
  it("is the goal, then each week with its sessions, then the notes", () => {
    const text = planText(PLAN);
    expect(text.split("\n").slice(0, 6)).toEqual([
      "Copenhagen Half · Sun 8 Nov · 21.1 km · goal 1:55:00",
      "",
      "Week 1 · 22–25 Oct · Base · 20 km",
      "- Thu 22 Oct: Easy run · 6 km (done)",
      "- Sat 24 Oct: Rest",
      "- Sun 25 Oct: Long run · 14 km (missed)",
    ]);
    expect(text.endsWith("Assumed four runs a week.")).toBe(true);
  });
});

describe("marking a session", () => {
  const easy = PLAN.weeks[0].sessions[0];
  const rest = PLAN.weeks[0].sessions[1];
  const long = PLAN.weeks[1].sessions[1];

  it("offers done once its day has come, and skipped whenever", () => {
    expect(marksFor({ ...easy, status: "planned" }, "2026-10-22")).toEqual(["done", "skipped"]);
    expect(marksFor(long, "2026-10-30")).toEqual(["skipped"]);
    expect(marksFor(PLAN.weeks[0].sessions[2], "2026-10-30")).toEqual(["done", "skipped"]);
  });

  it("offers undoing what was marked, and nothing for a rest day or a moved one", () => {
    expect(marksFor(easy, "2026-10-30")).toEqual(["planned"]);
    expect(marksFor(rest, "2026-10-30")).toEqual([]);
    expect(marksFor({ ...long, status: "moved" }, "2026-10-30")).toEqual([]);
  });

  it("changes that one session and says which in the reason", () => {
    const { plan, reason } = withMark(PLAN, 0, 2, "done");
    expect(reason).toBe("Long run on Sun 25 Oct: done");
    expect(plan.weeks[0].sessions[2].status).toBe("done");
    expect(plan.weeks[0].sessions[0]).toBe(PLAN.weeks[0].sessions[0]);
    expect(plan.weeks[1]).toBe(PLAN.weeks[1]);
    expect(PLAN.weeks[0].sessions[2].status).toBe("missed");
  });

  it("drops the run of a session that is not done any more", () => {
    const { plan, reason } = withMark(
      { ...PLAN, weeks: [{ ...PLAN.weeks[0], sessions: [{ ...easy, activity: "123" }] }] },
      0,
      0,
      "planned",
    );
    expect(reason).toBe("Easy run on Thu 22 Oct: back to planned");
    expect(plan.weeks[0].sessions[0]).toEqual({
      date: "2026-10-22",
      type: "easy",
      title: "Easy run",
      distance_km: 6,
      status: "planned",
    });
  });
});

describe("planOf", () => {
  it("keeps the coach's log", () => {
    const updates = [{ date: "2026-10-23", note: "Easier next week." }];
    expect(planOf({ ...PLAN, updates })?.updates).toEqual(updates);
  });
});
