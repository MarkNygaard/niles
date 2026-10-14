import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AutomationEditor } from "./AutomationEditor";
import { api } from "@/lib/api";
import type { AutomationView, Device, DeviceState, Rule, Run } from "@/lib/api";

const quiet: DeviceState = {
  on: null,
  brightness: null,
  color_temp_kelvin: null,
  rgb: null,
  temperature_celsius: null,
  humidity_percent: null,
  battery_percent: null,
  open: null,
  motion: null,
  illuminance_lux: null,
};

function device(id: string, cls: string, state: Partial<DeviceState> = {}): Device {
  const [source, rest] = id.split(":");
  const [room, name] = rest.split("/");
  return {
    id,
    source,
    room,
    name,
    class: cls,
    state: { ...quiet, ...state },
    supports_rgb: false,
    supports_color_temp: false,
  };
}

const house = [
  device("z2m:office/door", "unknown", { open: false }),
  device("z2m:office/desk_lamp", "light"),
];

const garden: Rule = {
  id: "4f1c",
  description: "Garden door",
  enabled: true,
  triggers: [{ id: "t1", type: "contact", device: "office/door", open: true }],
  conditions: [],
  actions: [
    { do: "set_room", room: "office", on: false },
    { do: "notify", body: "The garden door is open" },
  ],
};

const stored: AutomationView = {
  rule: garden,
  source: "stored",
  sentence: "When the office door opens, turn off the office lights and say so.",
  error: null,
};

function open(
  id: string | undefined,
  rules: AutomationView[] = [stored],
  devices = house,
  runs: Run[] = [],
) {
  vi.spyOn(api, "automations").mockResolvedValue({ rules });
  vi.spyOn(api, "automationRuns").mockResolvedValue({ runs });
  vi.spyOn(api, "devices").mockResolvedValue(devices);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <AutomationEditor id={id} />
    </QueryClientProvider>,
  );
}

function onAPhone() {
  vi.stubGlobal(
    "matchMedia",
    vi.fn().mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  );
}

describe("AutomationEditor", () => {
  beforeEach(() => {
    window.location.hash = "#/automations/new";
  });
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("starts a new rule as the door turning its room's lights off", async () => {
    open(undefined);
    expect(await screen.findByText("Office door opens")).toBeInTheDocument();
    expect(screen.getByText("Turn off Office lights")).toBeInTheDocument();
  });

  it("saves a new rule with its name, and goes back to the list", async () => {
    open(undefined);
    const create = vi.spyOn(api, "createAutomation").mockResolvedValue(stored);
    fireEvent.change(await screen.findByRole("textbox", { name: "Name" }), {
      target: { value: "Garden door" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith({
        description: "Garden door",
        enabled: true,
        triggers: [{ id: "t1", type: "contact", device: "office/door", open: true }],
        conditions: [],
        actions: [{ do: "set_room", room: "office", on: false }],
      }),
    );
    await waitFor(() => expect(window.location.hash).toBe("#/automations"));
  });

  it("shows why the server refused it", async () => {
    open(undefined);
    vi.spyOn(api, "createAutomation").mockRejectedValue(
      new Error("office/door is not a door or window"),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "office/door is not a door or window",
    );
  });

  it("will not save without a door to start from, and says so", async () => {
    open(undefined, [], [device("z2m:office/desk_lamp", "light")]);
    expect(await screen.findByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByText("Trigger 1 needs a device.")).toBeInTheDocument();
  });

  it("adds a condition from the palette, between the trigger and the actions", async () => {
    open(undefined);
    const create = vi.spyOn(api, "createAutomation").mockResolvedValue(stored);
    fireEvent.click(await screen.findByRole("button", { name: "Add condition" }));
    fireEvent.click(await screen.findByRole("button", { name: "Time of day" }));
    expect(await screen.findByText("Between 07:00 and 23:00")).toBeInTheDocument();
    // Selected as it is added, so its fields are the ones showing.
    fireEvent.change(screen.getByLabelText("Before"), { target: { value: "22:00" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({
          conditions: [{ kind: "time_of_day", after: "07:00", before: "22:00" }],
        }),
      ),
    );
  });

  it("removes an action from its node, and will not save with none", async () => {
    open(undefined);
    fireEvent.click(await screen.findByRole("button", { name: "Remove Turn off Office lights" }));
    expect(screen.queryByText("Turn off Office lights")).toBeNull();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByText("An automation needs at least one action.")).toBeInTheDocument();
  });

  it("changes a stored rule and puts it back whole", async () => {
    open("4f1c");
    const update = vi.spyOn(api, "updateAutomation").mockResolvedValue(stored);
    // The trigger is selected to begin with.
    fireEvent.click(await screen.findByRole("button", { name: "Closes" }));
    expect(screen.getByText("Office door closes")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        ...garden,
        triggers: [{ id: "t1", type: "contact", device: "office/door", open: false }],
      }),
    );
  });

  it("reorders actions from the side panel", async () => {
    open("4f1c");
    const update = vi.spyOn(api, "updateAutomation").mockResolvedValue(stored);
    fireEvent.click(await screen.findByText('Say "The garden door is open"'));
    fireEvent.click(screen.getByRole("button", { name: "Move up" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        ...garden,
        actions: [garden.actions[1], garden.actions[0]],
      }),
    );
  });

  it("deletes a rule once it is confirmed", async () => {
    open("4f1c");
    const remove = vi.spyOn(api, "deleteAutomation").mockResolvedValue(undefined);
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));
    const dialog = await screen.findByRole("dialog");
    expect(remove).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("4f1c"));
  });

  it("only shows the flow on a phone, with the switch and delete still there", async () => {
    onAPhone();
    open("4f1c");
    expect(await screen.findByText("Open on a wider screen to edit.")).toBeInTheDocument();
    expect(screen.getByText("Office door opens")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Add condition" })).toBeNull();
    expect(screen.getByRole("switch", { name: "Run Garden door" })).toBeChecked();
    expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument();
  });

  it("only shows a rule from the config", async () => {
    open("config-0", [{ ...stored, rule: { ...garden, id: "config-0" }, source: "config" }]);
    expect(await screen.findByText("From config")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(screen.queryByRole("switch")).toBeNull();
  });

  it("says when the rule is gone", async () => {
    open("gone");
    expect(await screen.findByText(/no such automation/)).toBeInTheDocument();
  });

  it("adds a second trigger, named by its place, and saves both with ids", async () => {
    open(undefined);
    const create = vi.spyOn(api, "createAutomation").mockResolvedValue(stored);
    fireEvent.click(await screen.findByRole("button", { name: "Add trigger" }));
    fireEvent.click(await screen.findByRole("button", { name: "Home or away" }));
    expect(await screen.findByText("Trigger 2 · Home or away")).toBeInTheDocument();
    // Selected as it is added: its own fields are the ones showing.
    expect(
      screen.getByRole("button", { name: "Somebody comes home", pressed: true }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({
          triggers: [
            { id: "t1", type: "contact", device: "office/door", open: true },
            { id: "t2", type: "presence", state: "home" },
          ],
        }),
      ),
    );
  });

  it("waits until a door has stayed open", async () => {
    open(undefined);
    // The trigger is selected to begin with.
    fireEvent.change(await screen.findByRole("spinbutton", { name: "Minutes" }), {
      target: { value: "10" },
    });
    expect(screen.getByText("Office door has been open for 10 minutes")).toBeInTheDocument();
  });

  it("follows a motion sensor until it has seen nobody for a while", async () => {
    open(undefined, [], [...house, device("z2m:hallway/motion", "motion", { motion: false })]);
    const create = vi.spyOn(api, "createAutomation").mockResolvedValue(stored);
    fireEvent.click(await screen.findByRole("button", { name: "Add trigger" }));
    fireEvent.click(await screen.findByRole("button", { name: "Motion" }));
    expect(await screen.findByText("Hallway motion: sees motion")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Stops seeing motion" }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "Minutes" }), {
      target: { value: "2" },
    });
    expect(screen.getByText("Hallway motion: no motion for 2 minutes")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({
          triggers: [
            { id: "t1", type: "contact", device: "office/door", open: true },
            {
              id: "t2",
              type: "motion",
              device: "hallway/motion",
              detected: false,
              for: { minutes: 2 },
            },
          ],
        }),
      ),
    );
  });

  it("checks whether a motion sensor sees anybody", async () => {
    open(undefined, [], [...house, device("z2m:hallway/motion", "motion", { motion: false })]);
    fireEvent.click(await screen.findByRole("button", { name: "Add condition" }));
    fireEvent.click(await screen.findByRole("button", { name: "Motion sensor sees" }));
    expect(await screen.findByText("Hallway motion sees motion")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sees no motion" }));
    expect(screen.getByText("Hallway motion sees no motion")).toBeInTheDocument();
  });

  it("says under a light level what counts as dim", async () => {
    const dark: Rule = {
      ...garden,
      conditions: [
        { kind: "numeric", device: "hallway/motion", measure: "illuminance", below: 30 },
      ],
    };
    open("4f1c", [{ ...stored, rule: dark }], [
      ...house,
      device("z2m:hallway/motion", "motion", { illuminance_lux: 12 }),
    ]);
    fireEvent.click(await screen.findByText("Hallway motion light level is below 30 lux"));
    expect(screen.getByText("Below about 30 is dim.")).toBeInTheDocument();
  });

  it("picks weekdays with toggles", async () => {
    open(undefined);
    fireEvent.click(await screen.findByRole("button", { name: "Add trigger" }));
    fireEvent.click(await screen.findByRole("button", { name: "Time of day" }));
    const days = within(await screen.findByRole("group", { name: "Days" }));
    for (const day of ["Mon", "Tue", "Wed", "Thu", "Fri"]) {
      fireEvent.click(days.getByRole("button", { name: day }));
    }
    expect(days.getByRole("button", { name: "Fri" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("At 07:00 on weekdays")).toBeInTheDocument();
  });

  it("adds a wait as a step of its own", async () => {
    open(undefined);
    fireEvent.click(await screen.findByRole("button", { name: "Add action" }));
    fireEvent.click(await screen.findByRole("button", { name: "Wait" }));
    expect(await screen.findByText("Wait 5 minutes")).toBeInTheDocument();
    fireEvent.change(screen.getByRole("spinbutton", { name: "Minutes" }), {
      target: { value: "" },
    });
    fireEvent.change(screen.getByRole("spinbutton", { name: "Seconds" }), {
      target: { value: "30" },
    });
    expect(screen.getByText("Wait 30 seconds")).toBeInTheDocument();
  });

  it("branches on which trigger started it, and saves the branches nested", async () => {
    open(undefined);
    const create = vi.spyOn(api, "createAutomation").mockResolvedValue(stored);
    fireEvent.click(await screen.findByRole("button", { name: "Add trigger" }));
    fireEvent.click(await screen.findByRole("button", { name: "Home or away" }));
    fireEvent.click(screen.getByRole("button", { name: "Add action" }));
    fireEvent.click(await screen.findByRole("button", { name: "If, then, else" }));

    // With two triggers, a new if starts out asking whether the first
    // started it — shown by place and sentence, never by id.
    const panel = within(screen.getByRole("complementary", { name: "Selected step" }));
    expect(panel.getByRole("checkbox", { name: "Trigger 1 — Office door opens" })).toBeChecked();
    expect(
      panel.getByRole("checkbox", { name: "Trigger 2 — Somebody comes home" }),
    ).not.toBeChecked();

    fireEvent.click(panel.getByRole("button", { name: "Add to else" }));
    fireEvent.click(await screen.findByRole("button", { name: "Say something" }));
    // The new step is the one selected, so its fields are showing.
    fireEvent.change(await screen.findByRole("textbox", { name: "Say" }), {
      target: { value: "Welcome home" },
    });
    expect(screen.getByText('Say "Welcome home"')).toBeInTheDocument();
    expect(screen.getByText(/^Else ·/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({
          actions: [
            { do: "set_room", room: "office", on: false },
            {
              do: "if",
              conditions: [{ kind: "triggered_by", triggers: ["t1"] }],
              then: [],
              else: [{ do: "notify", body: "Welcome home" }],
            },
          ],
        }),
      ),
    );
  });

  it("will not save a which-trigger check whose trigger was removed", async () => {
    open(undefined);
    fireEvent.click(await screen.findByRole("button", { name: "Add trigger" }));
    fireEvent.click(await screen.findByRole("button", { name: "Home or away" }));
    fireEvent.click(screen.getByRole("button", { name: "Add condition" }));
    fireEvent.click(await screen.findByRole("button", { name: "Which trigger started it" }));
    expect(await screen.findByText("Only if started by Trigger 1")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Remove Office door opens" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByText("Condition 1 needs a trigger to have started it.")).toBeInTheDocument();
  });

  it("shows a stored rule's recent runs beside the flow", async () => {
    const today = new Date();
    today.setHours(14, 2, 0, 0);
    open("4f1c", [stored], house, [
      {
        started_at: today.toISOString(),
        trigger: "t1",
        outcome: "done",
        reason: null,
        steps: ["turned off the office lights"],
      },
    ]);
    const runs = within(await screen.findByRole("region", { name: "Recent runs" }));
    expect(await runs.findByText("Today 14:02")).toBeInTheDocument();
    expect(runs.getByText("Trigger 1")).toBeInTheDocument();
    expect(runs.getByText("Done")).toBeInTheDocument();
    expect(runs.getByText("turned off the office lights")).toBeInTheDocument();
    expect(api.automationRuns).toHaveBeenCalledWith("4f1c");
  });

  it("says why a run stopped, on a phone too", async () => {
    onAPhone();
    open("4f1c", [stored], house, [
      {
        started_at: new Date().toISOString(),
        trigger: "t1",
        outcome: "stopped",
        reason: "it was before 07:00",
        steps: [],
      },
    ]);
    expect(await screen.findByText("it was before 07:00")).toBeInTheDocument();
    expect(screen.getByText("Stopped")).toBeInTheDocument();
  });

  it("asks for no runs of a rule not saved yet", async () => {
    open(undefined);
    expect(await screen.findByText("Office door opens")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Recent runs" })).toBeNull();
    expect(api.automationRuns).not.toHaveBeenCalled();
  });
});
