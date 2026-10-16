import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TodaysRunCard } from "@/components/TodaysRunCard";
import { ApiError, api } from "@/lib/api";
import type { ArtifactDetail } from "@/lib/api";
import type { Plan, PlanToday, Session } from "@/lib/trainingPlan";

const INTERVALS: Session = {
  date: "2026-10-16",
  type: "intervals",
  title: "6 × 400 m",
  distance_km: 7,
  pace: "4:45-5:00",
  details: "Warm up 10 min",
  status: "planned",
};
const LONG: Session = {
  date: "2026-10-18",
  type: "long",
  title: "Long run",
  distance_km: 12,
  status: "planned",
};

const TODAY: PlanToday = {
  id: 7,
  title: "Hellerup 10K plan",
  race: { race: "Hellerup 10K", distance_km: 10, date: "2026-10-25" },
  today: "2026-10-16",
  days_to_go: 9,
  sessions: [INTERVALS],
  next: LONG,
  week: { focus: "Build", planned_km: 25, run_km: 6.4, sessions_left: 2 },
  update: { date: "2026-10-15", note: "Eased Friday a little." },
};

const PLAN: Plan = {
  goal: TODAY.race,
  weeks: [
    {
      starts: "2026-10-12",
      sessions: [
        { date: "2026-10-13", type: "easy", title: "Easy run", status: "done" },
        INTERVALS,
        LONG,
      ],
    },
  ],
};

function show(plan: PlanToday | null | Error) {
  if (plan instanceof Error) vi.spyOn(api, "trainingToday").mockRejectedValue(plan);
  else vi.spyOn(api, "trainingToday").mockResolvedValue({ plan });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <TodaysRunCard />
    </QueryClientProvider>,
  );
}

describe("TodaysRunCard", () => {
  afterEach(() => vi.restoreAllMocks());

  it("shows today's session, the next one, the week and the coach's note", async () => {
    show(TODAY);
    expect(await screen.findByText("6 × 400 m")).toBeInTheDocument();
    expect(screen.getByText("Hellerup 10K")).toBeInTheDocument();
    expect(screen.getByText("9 days to go")).toBeInTheDocument();
    expect(screen.getByText("7 km · 4:45-5:00 /km")).toBeInTheDocument();
    expect(screen.getByText("Warm up 10 min")).toBeInTheDocument();
    expect(screen.getByText("Next: Sun 18 Oct · Long run · 12 km")).toBeInTheDocument();
    expect(screen.getByText("6.4 / 25 km")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Run this week" })).toHaveAttribute(
      "aria-valuenow",
      "6.4",
    );
    expect(screen.getByText("Eased Friday a little.")).toBeInTheDocument();
  });

  it("opens the plan in Niles's panel", async () => {
    show(TODAY);
    const link = await screen.findByRole("link", { name: "Open Hellerup 10K plan" });
    expect(link).toHaveAttribute("href", "#/chat?artifact=7");
  });

  it("says a rest day is one, with nothing to mark", async () => {
    show({ ...TODAY, sessions: [{ ...LONG, type: "rest", title: "Rest" }] });
    expect(await screen.findByText("Rest day")).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("says what was run once today's run is done", async () => {
    show({
      ...TODAY,
      sessions: [{ ...INTERVALS, status: "done", actual: { distance_km: 6.2, pace: "5:41" } }],
    });
    expect(await screen.findByText("Done ✓ 6.2 km at 5:41")).toBeInTheDocument();
    expect(screen.queryByText("Warm up 10 min")).toBeNull();
    expect(screen.queryByRole("button", { name: /^Done/ })).toBeNull();
  });

  it("leaves out an old note from the coach", async () => {
    show({ ...TODAY, update: { date: "2026-10-01", note: "An old note." } });
    await screen.findByText("6 × 400 m");
    expect(screen.queryByText("An old note.")).toBeNull();
  });

  it("is not there without a plan", async () => {
    const { container } = show(null);
    await waitFor(() => expect(api.trainingToday).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("is not there on a Niles without a database", async () => {
    const { container } = show(new ApiError("no database", 501));
    await waitFor(() => expect(api.trainingToday).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("marks today's session done on the plan as it is now", async () => {
    show(TODAY);
    vi.spyOn(api, "artifact").mockResolvedValue({ content: PLAN } as ArtifactDetail);
    const patch = vi
      .spyOn(api, "patchArtifact")
      .mockResolvedValue({ content: PLAN } as ArtifactDetail);
    fireEvent.click(await screen.findByRole("button", { name: "Done: 6 × 400 m" }));
    await waitFor(() => expect(patch).toHaveBeenCalled());
    const [id, body] = patch.mock.calls[0];
    expect(id).toBe(7);
    expect(body.reason).toBe("6 × 400 m on Fri 16 Oct: done");
    const saved = body.content as Plan;
    expect(saved.weeks[0].sessions[1].status).toBe("done");
    expect(saved.weeks[0].sessions[0].status).toBe("done");
    expect(saved.weeks[0].sessions[2].status).toBe("planned");
  });

  it("says so when the session is no longer in the plan", async () => {
    show(TODAY);
    vi.spyOn(api, "artifact").mockResolvedValue({
      content: { ...PLAN, weeks: [{ ...PLAN.weeks[0], sessions: [LONG] }] },
    } as ArtifactDetail);
    const patch = vi.spyOn(api, "patchArtifact");
    fireEvent.click(await screen.findByRole("button", { name: "Skipped: 6 × 400 m" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The plan has changed");
    expect(patch).not.toHaveBeenCalled();
  });
});
