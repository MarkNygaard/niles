import { describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { TrainingPlanView } from "@/components/TrainingPlanView";
import { kindOf } from "@/lib/artifacts";
import type { Plan } from "@/lib/trainingPlan";

const PLAN: Plan = {
  goal: { race: "Copenhagen Half", distance_km: 21.1, date: "2026-12-13", target_time: "1:55:00" },
  weeks: [
    {
      starts: "2026-10-05",
      focus: "Base",
      sessions: [
        {
          date: "2026-10-06",
          type: "easy",
          title: "Easy run",
          distance_km: 6,
          status: "done",
          activity: "123",
          actual: { distance_km: 6.2, duration_min: 39, pace: "6:17" },
        },
        { date: "2026-10-11", type: "long", title: "Long run", distance_km: 12, status: "missed" },
      ],
    },
    {
      starts: "2026-10-12",
      focus: "Build",
      sessions: [
        { date: "2026-10-14", type: "tempo", title: "Tempo 5 km", distance_km: 8, pace: "5:20", status: "done" },
        { date: "2026-10-16", type: "intervals", title: "6 × 800 m", duration_min: 50, status: "planned" },
        { date: "2026-10-18", type: "long", title: "Long run", distance_km: 14, status: "planned" },
      ],
    },
    {
      starts: "2026-10-19",
      sessions: [{ date: "2026-10-25", type: "long", title: "Long run", distance_km: 15, status: "planned" }],
    },
  ],
  notes: "Assumed **four** runs a week.",
};

function show() {
  render(<TrainingPlanView content={PLAN} today="2026-10-16" />);
  return screen.getByRole("list", { name: "Weeks" });
}

describe("TrainingPlanView", () => {
  it("heads the plan with the race and how long there is to go", () => {
    show();
    expect(screen.getByText("Copenhagen Half")).toBeInTheDocument();
    expect(screen.getByText("8 weeks to go")).toBeInTheDocument();
    expect(screen.getByText("Sun 13 Dec · 21.1 km · goal 1:55:00")).toBeInTheDocument();
  });

  it("opens this week with today marked and folds the others", () => {
    const weeks = show();
    const [past, current, next] = within(weeks).getAllByRole("button");
    expect(past).toHaveAttribute("aria-expanded", "false");
    expect(current).toHaveAttribute("aria-expanded", "true");
    expect(next).toHaveAttribute("aria-expanded", "false");
    expect(current).toHaveTextContent("This week");
    expect(current).toHaveTextContent("8 / 22 km");

    const today = weeks.querySelector('[aria-current="date"]');
    expect(today).toHaveTextContent("6 × 800 m");
    expect(today).toHaveTextContent("Today");
    expect(screen.queryByText("Easy run")).not.toBeInTheDocument();
  });

  it("shows what was run and what was missed in a past week", () => {
    const weeks = show();
    fireEvent.click(within(weeks).getAllByRole("button")[0]);
    expect(screen.getByText("Done · 6.2 km · 39 min · 6:17 /km")).toBeInTheDocument();
    expect(screen.getByText("Missed")).toBeInTheDocument();
    expect(within(weeks).getAllByRole("button")[0]).toHaveTextContent("6.2 / 18 km");
  });

  it("keeps the notes under the weeks, as markdown", () => {
    show();
    expect(screen.getByText("four").tagName).toBe("STRONG");
  });

  it("says so when the content is not a plan", () => {
    render(<TrainingPlanView content={{ markdown: "- a" }} today="2026-10-16" />);
    expect(screen.getByText("This plan cannot be read.")).toBeInTheDocument();
  });

  it("is the training plan kind's view, copied as text", () => {
    const kind = kindOf("training_plan");
    expect(kind.label).toBe("Training plan");
    expect(kind.text(PLAN)).toContain("Week 2 · 12–18 Oct · Build · 22 km");
  });
});
