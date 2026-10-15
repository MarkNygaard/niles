import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RoomDashboard } from "@/components/RoomDashboard";
import { api } from "@/lib/api";
import type { Device } from "@/lib/api";

vi.mock("@/hooks/useDeviceStream", () => ({ useDeviceStream: () => {} }));

function light(id: string): Device {
  const [source, rest] = id.split(":");
  const [room, name] = rest.split("/");
  return {
    id,
    source,
    room,
    name,
    class: "light",
    supports_rgb: false,
    supports_color_temp: false,
    available: true,
    state: {
      on: true,
      brightness: 100,
      color_temp_kelvin: null,
      rgb: null,
      temperature_celsius: null,
      humidity_percent: null,
      battery_percent: null,
      open: null,
      motion: null,
      illuminance_lux: null,
      position: null,
    },
  };
}

function renderDashboard() {
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <RoomDashboard />
    </QueryClientProvider>,
  );
}

describe("RoomDashboard", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("survives the devices arriving", async () => {
    // The bug this exists for: a `useQuery` added below the
    // `isLoading` early return is called on the second render and not
    // the first. React counts hooks per render, so that is not a
    // warning — it unmounts the whole page the moment the devices
    // land, and the dashboard goes blank.
    vi.spyOn(api, "scenes").mockResolvedValue([]);
    vi.spyOn(api, "devices").mockResolvedValue([light("z2m:kitchen/bulb_1")]);

    renderDashboard();

    expect(await screen.findByText("Kitchen")).toBeInTheDocument();
  });

  it("shows a scene once one is saved", async () => {
    vi.spyOn(api, "scenes").mockResolvedValue(["cosy"]);
    vi.spyOn(api, "devices").mockResolvedValue([light("z2m:kitchen/bulb_1")]);

    renderDashboard();

    expect(
      await screen.findByRole("button", { name: /Cosy/ }),
    ).toBeInTheDocument();
  });

  it("carries on when scenes cannot be read", async () => {
    // An older server has no /scenes route. The rooms still matter.
    vi.spyOn(api, "scenes").mockRejectedValue(new Error("404"));
    vi.spyOn(api, "devices").mockResolvedValue([light("z2m:kitchen/bulb_1")]);

    renderDashboard();

    await waitFor(() =>
      expect(screen.getByText("Kitchen")).toBeInTheDocument(),
    );
  });

  it("moves a blind by its name, and shows it moving before it answers", async () => {
    const shade: Device = {
      ...light("z2m:kitchen/blind"),
      class: "cover",
      state: { ...light("z2m:kitchen/blind").state, on: null, brightness: null, position: 0 },
    };
    vi.spyOn(api, "scenes").mockResolvedValue([]);
    vi.spyOn(api, "devices").mockResolvedValue([light("z2m:kitchen/bulb_1"), shade]);
    // Never answers, so what shows is the guess and not a refetch.
    const setPosition = vi
      .spyOn(api, "setPosition")
      .mockReturnValue(new Promise<null>(() => {}));

    renderDashboard();
    fireEvent.click(await screen.findByRole("button", { name: "Blinds in Kitchen" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("Closed");
    fireEvent.click(screen.getByRole("button", { name: "Open Blind" }));

    await waitFor(() => expect(setPosition).toHaveBeenCalledWith("kitchen", "blind", 100));
    expect(await screen.findByText("Open", { selector: "div" })).toBeInTheDocument();
  });
});

describe("RoomDashboard with hidden rooms", () => {
  beforeEach(() => vi.restoreAllMocks());

  function hiding(hidden: string[]) {
    vi.spyOn(api, "getConfig").mockResolvedValue({
      effective: { rooms: { hidden } },
      overrides: {},
      sections: [],
      persistent: true,
    });
  }

  it("leaves a hidden room off the grid", async () => {
    vi.spyOn(api, "scenes").mockResolvedValue([]);
    vi.spyOn(api, "devices").mockResolvedValue([
      light("z2m:kitchen/bulb_1"),
      light("z2m:garage/bulb_1"),
    ]);
    hiding(["garage"]);

    renderDashboard();

    expect(await screen.findByText("Kitchen")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Garage")).toBeNull());
  });

  it("still counts a hidden room's lights in the house bar", async () => {
    // Hiding is about the grid. "Turn everything off" still reaches
    // the garage, so the bar has to say its light is on.
    vi.spyOn(api, "scenes").mockResolvedValue([]);
    vi.spyOn(api, "devices").mockResolvedValue([
      light("z2m:kitchen/bulb_1"),
      light("z2m:garage/bulb_1"),
    ]);
    hiding(["garage"]);

    renderDashboard();

    expect(await screen.findByText("Kitchen")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Garage")).toBeNull());
    expect(screen.getByRole("button", { name: /^The whole house, All 2 on/ })).toBeInTheDocument();
  });

  it("points to Settings when every room is hidden", async () => {
    vi.spyOn(api, "scenes").mockResolvedValue([]);
    vi.spyOn(api, "devices").mockResolvedValue([light("z2m:kitchen/bulb_1")]);
    hiding(["kitchen"]);

    renderDashboard();

    expect(await screen.findByText(/Every room is hidden/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Settings → Rooms/ })).toHaveAttribute(
      "href",
      "#/me/settings/rooms",
    );
    expect(screen.queryByText("Kitchen")).toBeNull();
  });

  it("has no such note while a room is showing", async () => {
    vi.spyOn(api, "scenes").mockResolvedValue([]);
    vi.spyOn(api, "devices").mockResolvedValue([light("z2m:kitchen/bulb_1")]);
    hiding([]);

    renderDashboard();

    expect(await screen.findByText("Kitchen")).toBeInTheDocument();
    expect(screen.queryByText(/Every room is hidden/)).toBeNull();
  });
});

describe("RoomDashboard with plugins' widgets", () => {
  beforeEach(() => vi.restoreAllMocks());

  const plugin = {
    id: "spotprice",
    name: "Spot prices",
    version: "0.1.0",
    description: "",
    authors: [],
    homepage: null,
    group: { id: "custom", label: "Custom" },
    status: "running" as const,
    status_line: "Working",
    error: null,
    dropped_events: 0,
    enabled: true,
    repo: null,
    release_tag: null,
    release_url: null,
    installed_at: null,
    updated_at: null,
    logo: null,
    previous_version: null,
    update: null,
    settings: [],
    menu: [],
    widgets: [
      { id: "price_now", title: "Electricity now", room: null, refresh_every: null },
      { id: "kitchen_price", title: "Kitchen price", room: "kitchen", refresh_every: null },
    ],
    interface: "0.1",
    older_interface: false,
  };

  it("draws a front-page widget as a card, and a room's where the room opens", async () => {
    vi.spyOn(api, "scenes").mockResolvedValue([]);
    vi.spyOn(api, "devices").mockResolvedValue([light("z2m:kitchen/bulb_1")]);
    vi.spyOn(api, "plugins").mockResolvedValue([plugin]);
    const surface = vi.spyOn(api, "surface").mockImplementation(async (_p, id) => ({
      nodes: [{ type: "text", value: `drawn ${id}`, muted: false, children: [] }],
    }));

    renderDashboard();

    expect(await screen.findByText("Electricity now")).toBeInTheDocument();
    expect(await screen.findByText("drawn price_now")).toBeInTheDocument();
    expect(screen.queryByText("Kitchen price")).toBeNull();
    expect(surface).toHaveBeenCalledWith("spotprice", "price_now", null);

    fireEvent.click(screen.getByRole("button", { name: "Lights in Kitchen" }));
    expect(await screen.findByRole("region", { name: "Kitchen price" })).toBeInTheDocument();
    expect(await screen.findByText("drawn kitchen_price")).toBeInTheDocument();
    expect(surface).toHaveBeenCalledWith("spotprice", "kitchen_price", "kitchen");
  });

  it("keeps a hidden room's widget in that room rather than the front page", async () => {
    vi.spyOn(api, "scenes").mockResolvedValue([]);
    vi.spyOn(api, "devices").mockResolvedValue([
      light("z2m:kitchen/bulb_1"),
      light("z2m:office/bulb_1"),
    ]);
    vi.spyOn(api, "plugins").mockResolvedValue([plugin]);
    vi.spyOn(api, "getConfig").mockResolvedValue({
      effective: { rooms: { hidden: ["kitchen"] } },
      overrides: {},
      sections: [],
      persistent: true,
    });
    vi.spyOn(api, "surface").mockImplementation(async (_p, id) => ({
      nodes: [{ type: "text", value: `drawn ${id}`, muted: false, children: [] }],
    }));

    renderDashboard();

    expect(await screen.findByText("Electricity now")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Kitchen")).toBeNull());
    expect(screen.getByText("Office")).toBeInTheDocument();
    expect(screen.queryByText("Kitchen price")).toBeNull();
  });
});
