import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { DevicePicker, deviceOptions } from "@/components/DevicePicker";

const REGISTRY = [
  {
    id: "z2m:office/desk_lamp",
    room: "office",
    name: "desk_lamp",
    source: "z2m",
    class: "light",
    supports_rgb: true,
    supports_color_temp: true,
  },
  {
    id: "wled:living_room/tv_light",
    room: "living_room",
    name: "tv_light",
    source: "wled",
    class: "light",
    supports_rgb: true,
    supports_color_temp: false,
  },
  {
    id: "z2m:office/switch",
    room: "office",
    name: "switch",
    source: "z2m",
    class: "switch",
    supports_rgb: false,
    supports_color_temp: false,
  },
];

describe("deviceOptions", () => {
  it("offers lights only — a switch can't be an ambient light", () => {
    const options = deviceOptions(REGISTRY);
    expect(options.map((o) => o.value)).toEqual([
      "wled:living_room/tv_light",
      "z2m:office/desk_lamp",
    ]);
  });

  it("names devices the way a person would, and groups by room", () => {
    const [first] = deviceOptions(REGISTRY);
    expect(first.label).toBe("Tv light");
    expect(first.room).toBe("Living room");
  });

  it("keeps the source in the stored value", () => {
    // A bare `room/name` means Zigbee to the config parser, so a WLED
    // strip has to carry its source or it silently never goes ambient.
    const options = deviceOptions(REGISTRY);
    expect(options[0].value).toBe("wled:living_room/tv_light");
  });
});

function setup(value: string[]) {
  const onChange = vi.fn();
  render(
    <DevicePicker
      id="ambient_lights.devices"
      aria-label="Ambient lights"
      value={value}
      options={deviceOptions(REGISTRY)}
      emptyMessage="Niles has no lights registered yet."
      onChange={onChange}
    />,
  );
  return { onChange };
}

// Opening the popup can't be driven here: Base UI's positioning never
// settles under jsdom, and the test hangs rather than failing. Picking
// from the list is verified by hand instead — which is how the bug
// below got in, so it is worth saying out loud.
describe("DevicePicker", () => {
  it("names every chip by its room and its light", () => {
    // Every house has more than one "Ceiling", and a chip that only
    // said "Ceiling" named nothing.
    setup(["wled:living_room/tv_light", "z2m:office/desk_lamp"]);
    expect(screen.getByText("Tv light")).toBeInTheDocument();
    expect(screen.getByText("Living room")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Remove Office Desk lamp" }),
    ).toBeInTheDocument();
  });

  it("removes one light without touching the others", () => {
    const { onChange } = setup([
      "wled:living_room/tv_light",
      "z2m:office/desk_lamp",
    ]);
    fireEvent.click(
      screen.getByRole("button", { name: "Remove Living room Tv light" }),
    );
    expect(onChange).toHaveBeenCalledWith(["z2m:office/desk_lamp"]);
  });

  it("still shows a configured device the registry has never mentioned", () => {
    // Dropping it on sight would quietly rewrite the config every time
    // a light happened to be offline.
    setup(["z2m:garage/ghost"]);
    expect(screen.getByText("z2m:garage/ghost")).toBeInTheDocument();
  });
});
