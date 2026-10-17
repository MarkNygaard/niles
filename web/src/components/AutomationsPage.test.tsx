import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AutomationsPage } from "./AutomationsPage";
import { api } from "@/lib/api";
import type { AutomationView, Rule, Run } from "@/lib/api";

const garden: Rule = {
  id: "4f1c",
  description: "Garden door",
  enabled: true,
  triggers: [{ id: "t1", type: "contact", device: "office/door", open: true }],
  conditions: [],
  actions: [{ do: "set_room", room: "office", on: false }],
};

const stored: AutomationView = {
  rule: garden,
  source: "stored",
  sentence: "When the office door opens, turn off the office lights.",
  in_full: "",
  warnings: [],
  error: null,
};

const fromConfig: AutomationView = {
  rule: {
    id: "config-0",
    description: "",
    enabled: true,
    triggers: [{ id: "t1", type: "presence", state: "away" }],
    conditions: [],
    actions: [{ do: "set_room", room: "hall", on: false }],
  },
  source: "config",
  sentence: "When everybody leaves, turn off the hall lights.",
  in_full: "",
  warnings: [],
  error: null,
};

function show(rules: AutomationView[], runs: Run[] = []) {
  vi.spyOn(api, "automations").mockResolvedValue({ rules });
  vi.spyOn(api, "automationRuns").mockResolvedValue({ runs });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <AutomationsPage />
    </QueryClientProvider>,
  );
}

describe("AutomationsPage", () => {
  afterEach(() => vi.restoreAllMocks());

  it("lists each rule as its sentence, under its name", async () => {
    show([stored]);
    expect(await screen.findByText(stored.sentence)).toBeInTheDocument();
    expect(screen.getByText("Garden door")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /office door opens/ })).toHaveAttribute(
      "href",
      "#/automations/4f1c",
    );
  });

  it("lists a motion rule by what the server says of it", async () => {
    const hallway: AutomationView = {
      rule: {
        id: "9a2b",
        description: "Hallway light with motion",
        enabled: true,
        triggers: [
          { id: "t1", type: "motion", device: "hallway/motion", detected: true },
          {
            id: "t2",
            type: "motion",
            device: "hallway/motion",
            detected: false,
            for: { minutes: 2 },
          },
        ],
        conditions: [
          { kind: "numeric", device: "hallway/motion", measure: "illuminance", below: 30 },
        ],
        actions: [{ do: "set_room", room: "hallway", on: true }],
      },
      source: "stored",
      sentence:
        "When the hallway motion sees motion, or when the hallway motion has seen no " +
        "motion for 2 minutes, if the light level of the hallway motion is below 30, " +
        "turn on the hallway lights.",
      in_full: "",
      warnings: [],
      error: null,
    };
    show([hallway]);
    expect(await screen.findByText(hallway.sentence)).toBeInTheDocument();
    expect(screen.getByText("Hallway light with motion")).toBeInTheDocument();
  });

  it("pauses a rule with its switch", async () => {
    show([stored]);
    // Never answers: what the switch shows meanwhile is the point.
    const update = vi.spyOn(api, "updateAutomation").mockReturnValue(new Promise(() => {}));
    const toggle = await screen.findByRole("switch", { name: "Run Garden door" });
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    await waitFor(() => expect(update).toHaveBeenCalledWith({ ...garden, enabled: false }));
    expect(screen.getByRole("switch", { name: "Run Garden door" })).not.toBeChecked();
  });

  it("marks a rule from the config, and offers it no switch or link", async () => {
    show([fromConfig]);
    expect(await screen.findByText(fromConfig.sentence)).toBeInTheDocument();
    expect(screen.getByText("From config")).toBeInTheDocument();
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.queryByRole("link", { name: /everybody leaves/ })).toBeNull();
  });

  it("lists what a rule will not do the way it reads, under it", async () => {
    const warning =
      "A run started by trigger 3 checks that the kitchen motion sees no motion only once its " +
      "wait is over, before it will turn off the hallway lights.";
    show([{ ...stored, warnings: [warning] }]);
    const listed = await screen.findByRole("list", { name: "Warnings" });
    expect(listed).toHaveTextContent(warning);
  });

  it("says why a rule is not running", async () => {
    show([{ ...stored, error: "office/door is not a door or window" }]);
    expect(await screen.findByText("office/door is not a door or window")).toBeInTheDocument();
  });

  it("says when a stored rule last ran, and how it ended", async () => {
    const today = new Date();
    today.setHours(14, 2, 0, 0);
    show(
      [stored, fromConfig],
      [
        {
          started_at: today.toISOString(),
          trigger: "t1",
          outcome: "stopped",
          reason: "it was before 07:00",
          steps: [],
        },
      ],
    );
    expect(
      await screen.findByText("Last run today 14:02 — stopped: it was before 07:00"),
    ).toBeInTheDocument();
    // A rule from the config file has no runs of its own to ask for.
    expect(api.automationRuns).toHaveBeenCalledTimes(1);
    expect(api.automationRuns).toHaveBeenCalledWith("4f1c");
  });

  it("says nothing of runs for a rule that has not run", async () => {
    show([stored]);
    expect(await screen.findByText(stored.sentence)).toBeInTheDocument();
    await waitFor(() => expect(api.automationRuns).toHaveBeenCalled());
    expect(screen.queryByText(/Last run/)).toBeNull();
  });

  it("says a paused rule is paused", async () => {
    show([{ ...stored, rule: { ...garden, enabled: false } }]);
    expect(await screen.findByText("Paused")).toBeInTheDocument();
  });

  it("explains, with nothing yet, that Niles can be asked", async () => {
    show([]);
    expect(await screen.findByText(/Niles, when the office door opens/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "New automation" })).toHaveAttribute(
      "href",
      "#/automations/new",
    );
  });
});
