import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SchedulesPanel, describeWhen, draftOf, formOf } from "@/components/SchedulesPanel";
import { api } from "@/lib/api";
import type { Schedule } from "@/lib/api";

const LINEAR: Schedule = {
  id: 1,
  name: "Linear on Monday",
  instruction: "my open Linear issues by priority",
  when: { kind: "days", days: ["Mon"], at: "08:00" },
  paused: false,
  next_due: "2026-11-23T07:00:00Z",
};

function renderPanel() {
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <SchedulesPanel />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("SchedulesPanel", () => {
  it("lists each schedule with when it runs", async () => {
    vi.spyOn(api, "schedules").mockResolvedValue({ schedules: [LINEAR], assistant: true });
    renderPanel();

    const list = await screen.findByRole("list", { name: "Your schedules" });
    expect(list).toHaveTextContent("Linear on Monday");
    expect(list).toHaveTextContent("Mon at 08:00");
    expect(screen.getByRole("button", { name: "Run Linear on Monday now" })).toBeEnabled();
  });

  it("says nothing runs without an assistant", async () => {
    vi.spyOn(api, "schedules").mockResolvedValue({ schedules: [LINEAR], assistant: false });
    renderPanel();

    expect(await screen.findByText(/Schedules run on Claude or ChatGPT/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run Linear on Monday now" })).toBeDisabled();
  });

  it("adds a schedule on chosen days", async () => {
    vi.spyOn(api, "schedules").mockResolvedValue({ schedules: [], assistant: true });
    const add = vi.spyOn(api, "addSchedule").mockResolvedValue(LINEAR);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Add a schedule" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Linear on Monday" } });
    fireEvent.change(screen.getByLabelText("What Niles should do"), {
      target: { value: "my open Linear issues by priority" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Chosen days" }));
    const days = screen.getByRole("group", { name: "Days" });
    fireEvent.click(days.querySelector("button:nth-child(3)")!);
    fireEvent.change(screen.getByLabelText("At"), { target: { value: "07:30" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));

    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add.mock.calls[0][0]).toEqual({
      name: "Linear on Monday",
      instruction: "my open Linear issues by priority",
      when: { kind: "days", days: ["Mon", "Wed"], at: "07:30" },
    });
  });

  it("pauses and runs one now", async () => {
    vi.spyOn(api, "schedules").mockResolvedValue({ schedules: [LINEAR], assistant: true });
    const pause = vi.spyOn(api, "pauseSchedule").mockResolvedValue(undefined);
    const run = vi.spyOn(api, "runSchedule").mockResolvedValue(undefined);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Pause Linear on Monday" }));
    await waitFor(() => expect(pause).toHaveBeenCalledWith(1, true));
    fireEvent.click(screen.getByRole("button", { name: "Run Linear on Monday now" }));
    await waitFor(() => expect(run).toHaveBeenCalledWith(1, expect.anything()));
    expect(await screen.findByText(/Running now/)).toBeInTheDocument();
  });

  it("shows a run's outcome and opens its conversation", async () => {
    vi.spyOn(api, "schedules").mockResolvedValue({ schedules: [LINEAR], assistant: true });
    vi.spyOn(api, "scheduleRuns").mockResolvedValue({
      runs: [
        {
          id: 9,
          due: "2026-11-16T07:00:00Z",
          finished: "2026-11-16T07:00:40Z",
          outcome: "waiting",
          conversation: 12,
          reason: null,
        },
        {
          id: 8,
          due: "2026-11-09T07:00:00Z",
          finished: null,
          outcome: "missed",
          conversation: null,
          reason: null,
        },
      ],
    });
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Runs of Linear on Monday" }));
    const runs = await screen.findByRole("list", { name: "Runs of Linear on Monday" });
    expect(runs).toHaveTextContent("Waiting for your yes");
    expect(runs).toHaveTextContent("Missed");
    expect(screen.getByRole("link", { name: "Open" })).toHaveAttribute(
      "href",
      "#/chat?conversation=12",
    );
  });
});

describe("describeWhen", () => {
  it("says each pattern in words", () => {
    expect(describeWhen({ kind: "daily", at: "08:00" })).toBe("Every day at 08:00");
    expect(describeWhen({ kind: "weekdays", at: "07:30" })).toBe("Weekdays at 07:30");
    expect(describeWhen({ kind: "days", days: ["Mon", "Wed", "Fri"], at: "18:05" })).toBe(
      "Mon, Wed and Fri at 18:05",
    );
    expect(describeWhen({ kind: "every_hours", hours: 1 })).toBe("Every hour");
    expect(describeWhen({ kind: "every_hours", hours: 6 })).toBe("Every 6 hours");
  });
});

describe("draftOf", () => {
  it("sends only what the chosen pattern needs, days in week order", () => {
    const form = { ...formOf(), name: " Mail ", instruction: "anything new?", kind: "days" as const };
    expect(draftOf({ ...form, days: ["Fri", "Mon"] }).when).toEqual({
      kind: "days",
      days: ["Mon", "Fri"],
      at: "08:00",
    });
    expect(draftOf({ ...form, kind: "every_hours", hours: 4 })).toEqual({
      name: "Mail",
      instruction: "anything new?",
      when: { kind: "every_hours", hours: 4 },
    });
  });

  it("reads a schedule back into the form", () => {
    expect(draftOf(formOf(LINEAR))).toEqual({
      name: LINEAR.name,
      instruction: LINEAR.instruction,
      when: LINEAR.when,
    });
  });
});
