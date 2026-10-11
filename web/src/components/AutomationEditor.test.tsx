import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AutomationEditor } from "./AutomationEditor";
import { api } from "@/lib/api";
import type { AutomationView, Device, DeviceState, Rule } from "@/lib/api";

const quiet: DeviceState = {
  on: null,
  brightness: null,
  color_temp_kelvin: null,
  rgb: null,
  temperature_celsius: null,
  humidity_percent: null,
  battery_percent: null,
  open: null,
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
  trigger: { type: "contact", device: "office/door", open: true },
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

function open(id: string | undefined, rules: AutomationView[] = [stored], devices = house) {
  vi.spyOn(api, "automations").mockResolvedValue({ rules });
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
        trigger: { type: "contact", device: "office/door", open: true },
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
    expect(screen.getByText("The trigger needs a device.")).toBeInTheDocument();
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
        trigger: { type: "contact", device: "office/door", open: false },
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
});
