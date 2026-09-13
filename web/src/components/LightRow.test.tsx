import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { LightRow } from "@/components/LightRow";
import type { Device, DeviceState } from "@/lib/api";

function light(overrides: Partial<Device> = {}, reported: Partial<DeviceState> = {}): Device {
  return {
    id: "z2m:kitchen/ceiling_light",
    source: "z2m",
    room: "kitchen",
    name: "ceiling_light",
    class: "light",
    supports_rgb: false,
    supports_color_temp: false,
    ...overrides,
    state: {
      on: null,
      brightness: null,
      color_temp_kelvin: null,
      rgb: null,
      temperature_celsius: null,
      humidity_percent: null,
      battery_percent: null,
    open: null,
      ...reported,
    },
  };
}

function setup(device: Device) {
  const onSet = vi.fn();
  render(<LightRow light={device} onSet={onSet} />);
  return { onSet };
}

describe("LightRow", () => {
  it("asks for the opposite of what the light reports", () => {
    const { onSet } = setup(light({}, { on: true }));
    fireEvent.click(screen.getByRole("button", { name: "Ceiling light — on" }));
    expect(onSet).toHaveBeenCalledWith({ on: false });
  });

  it("does not claim a light it has never heard from is off", () => {
    // Drawing it as off is a claim about a lamp that might well be lit.
    const { onSet } = setup(light());
    const power = screen.getByRole("button", { name: "Ceiling light — state unknown" });
    expect(screen.getByText("Not heard from yet")).toBeInTheDocument();

    fireEvent.click(power);
    expect(onSet).toHaveBeenCalledWith({ on: true });
  });

  it("offers only what this light can act on", () => {
    setup(light({ supports_rgb: true }, { on: true }));
    expect(screen.getByRole("button", { name: "Colour" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Colour temperature" }),
    ).not.toBeInTheDocument();
  });

  it("offers a white where there is a white channel", () => {
    setup(light({ supports_color_temp: true }, { on: true }));
    expect(
      screen.getByRole("button", { name: "Colour temperature" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Colour" })).not.toBeInTheDocument();
  });

  it("shows the level the light is at", () => {
    setup(light({}, { on: true, brightness: 65 }));
    expect(screen.getByText("On · 65%")).toBeInTheDocument();
    expect(
      screen.getByRole("slider", { name: "Ceiling light brightness" }),
    ).toBeInTheDocument();
  });

  it("gives a plug a switch and nothing it can't act on", () => {
    setup(light({ class: "outlet" }, { on: true }));
    expect(
      screen.getByRole("button", { name: "Ceiling light — on" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("slider")).not.toBeInTheDocument();
    expect(screen.getByText(/plug/)).toBeInTheDocument();
  });

  it("names the source when it isn't the usual one", () => {
    // Two lights can share a name across sources; which one this row
    // is has to be readable without opening anything.
    setup(light({ source: "wled" }, { on: false }));
    expect(screen.getByText(/wled/)).toBeInTheDocument();
  });
});
