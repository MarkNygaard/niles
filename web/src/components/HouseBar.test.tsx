import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { HouseBar } from "@/components/HouseBar";
import { roomsOf } from "@/lib/rooms";
import type { Device, DeviceState } from "@/lib/api";

function light(room: string, on: boolean): Device {
  const state: DeviceState = {
    on,
    brightness: null,
    color_temp_kelvin: null,
    rgb: null,
    temperature_celsius: null,
    humidity_percent: null,
    battery_percent: null,
  };
  return {
    id: `z2m:${room}/light`,
    source: "z2m",
    room,
    name: "light",
    class: "light",
    supports_rgb: false,
    supports_color_temp: false,
    state,
  };
}

function setup(devices: Device[]) {
  const onToggle = vi.fn();
  render(<HouseBar rooms={roomsOf(devices)} onToggle={onToggle} />);
  return { onToggle };
}

describe("HouseBar", () => {
  it("says what it will do before you press it", () => {
    setup([light("kitchen", true), light("office", false)]);
    expect(screen.getByText("Turn everything off")).toBeInTheDocument();
    expect(screen.getByText("1 of 2 on")).toBeInTheDocument();
  });

  it("offers to turn everything on when the house is dark", () => {
    setup([light("kitchen", false), light("office", false)]);
    expect(screen.getByText("Turn everything on")).toBeInTheDocument();
    expect(screen.getByText("Nothing on, 2 lights")).toBeInTheDocument();
  });

  it("carries the action in its label, not just the name", () => {
    setup([light("kitchen", true)]);
    expect(
      screen.getByRole("button", { name: /Turn everything off\.$/ }),
    ).toBeInTheDocument();
  });

  it("presses once for the whole house", () => {
    const { onToggle } = setup([light("kitchen", true), light("office", true)]);
    fireEvent.click(screen.getByRole("button"));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });
});
