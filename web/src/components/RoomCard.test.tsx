import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { RoomCard } from "@/components/RoomCard";
import { roomsOf } from "@/lib/rooms";
import type { Device, DeviceState } from "@/lib/api";

function state(partial: Partial<DeviceState> = {}): DeviceState {
  return {
    on: null,
    brightness: null,
    color_temp_kelvin: null,
    rgb: null,
    temperature_celsius: null,
    humidity_percent: null,
    battery_percent: null,
    ...partial,
  };
}

function device(name: string, klass: string, reported: DeviceState): Device {
  return {
    id: `z2m:kitchen/${name}`,
    source: "z2m",
    room: "kitchen",
    name,
    class: klass,
    supports_rgb: false,
    supports_color_temp: false,
    state: reported,
  };
}

function light(name: string, on: boolean | null): Device {
  return device(name, "light", state({ on }));
}

function sensor(name: string, celsius: number, humidity: number): Device {
  return device(
    name,
    "sensor",
    state({ temperature_celsius: celsius, humidity_percent: humidity }),
  );
}

// The card asks the viewport whether it is in a hand. jsdom always
// says no (see test-setup), so a test about the phone has to say so.
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

afterEach(() => {
  vi.unstubAllGlobals();
});

function setup(devices: Device[]) {
  const onSetRoom = vi.fn();
  const onSetLight = vi.fn();
  const [room] = roomsOf(devices);
  render(
    <RoomCard room={room} onSetRoom={onSetRoom} onSetLight={onSetLight} />,
  );
  return { onSetRoom, onSetLight };
}

describe("RoomCard", () => {
  it("turns the room off when something in it is on", () => {
    const { onSetRoom } = setup([light("ceiling", true), light("counter", false)]);

    fireEvent.click(screen.getByRole("button", { name: /^Kitchen, 1 of 2 on/ }));
    expect(onSetRoom).toHaveBeenCalledWith({ on: false });
  });

  it("turns the room on when it is dark", () => {
    const { onSetRoom } = setup([light("ceiling", false), light("counter", false)]);

    fireEvent.click(screen.getByRole("button", { name: /^Kitchen, 2 lights, all off/ }));
    expect(onSetRoom).toHaveBeenCalledWith({ on: true });
  });

  it("says what pressing it will do", () => {
    // The card is the switch, so the label has to carry the action —
    // "Kitchen" alone gives a screen reader nothing to go on.
    setup([light("ceiling", true)]);
    expect(
      screen.getByRole("button", { name: "Kitchen, On. Turn all off." }),
    ).toBeInTheDocument();
  });

  it("keeps opening the room separate from switching it", () => {
    // One press is the whole room; the finer controls are a different
    // target, not a long-press a mouse can't perform.
    const { onSetRoom } = setup([light("ceiling", true), light("counter", true)]);

    fireEvent.click(screen.getByRole("button", { name: "Adjust 2 lights" }));
    expect(onSetRoom).not.toHaveBeenCalled();
  });

  it("closes with a button on a desktop", () => {
    setup([light("ceiling", true), light("counter", true)]);

    fireEvent.click(screen.getByRole("button", { name: "Adjust 2 lights" }));
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  it("closes by being swiped away on a phone", () => {
    // The drawer advertises the gesture with a handle, so a close
    // button beside it would be a second way to do the same thing.
    onAPhone();
    setup([light("ceiling", true), light("counter", true)]);

    fireEvent.click(screen.getByRole("button", { name: "Adjust 2 lights" }));
    expect(
      screen.getByRole("button", { name: /^All lights in Kitchen/ }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
  });

  it("shows what the room is reporting alongside its lights", () => {
    setup([light("ceiling", true), sensor("thermometer", 21.42, 54.3)]);
    expect(screen.getByText("21.4°C")).toBeInTheDocument();
    expect(screen.getByText("54%")).toBeInTheDocument();
  });
});
