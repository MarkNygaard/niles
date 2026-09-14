import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
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
});
