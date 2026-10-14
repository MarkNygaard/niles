import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { BlindRow } from "@/components/BlindRow";
import type { Device, DeviceState } from "@/lib/api";

function blind(position: number | null): Device {
  const state: DeviceState = {
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
    position,
  };
  return {
    id: "z2m:office/blind",
    source: "z2m",
    room: "office",
    name: "blind",
    class: "cover",
    supports_rgb: false,
    supports_color_temp: false,
    state,
  };
}

function setup(position: number | null) {
  const onSet = vi.fn();
  render(<BlindRow blind={blind(position)} onSet={onSet} />);
  return { onSet };
}

describe("BlindRow", () => {
  it("says how far open it is", () => {
    setup(40);
    expect(screen.getByText("40% open")).toBeInTheDocument();
  });

  it("names the two ends rather than giving them a number", () => {
    const { unmount } = render(<BlindRow blind={blind(0)} onSet={vi.fn()} />);
    expect(screen.getByText("Closed", { selector: "div" })).toBeInTheDocument();
    unmount();
    setup(100);
    expect(screen.getByText("Open", { selector: "div" })).toBeInTheDocument();
  });

  it("does not claim a blind it has never heard from is closed", () => {
    setup(null);
    expect(screen.getByText("Not heard from yet")).toBeInTheDocument();
  });

  it("opens all the way and closes all the way", () => {
    const { onSet } = setup(40);
    fireEvent.click(screen.getByRole("button", { name: "Open Blind" }));
    expect(onSet).toHaveBeenLastCalledWith(100);
    fireEvent.click(screen.getByRole("button", { name: "Close Blind" }));
    expect(onSet).toHaveBeenLastCalledWith(0);
  });

  it("moves the blind once, when the slider is let go", () => {
    // Every value under a moving thumb would be a command to a motor
    // that takes seconds to get anywhere. jsdom has no layout, so the
    // control is told how wide it is; the thumb sits at 0.
    const { onSet } = setup(0);
    const input = screen.getByRole("slider", { name: "Blind position" });
    const thumb = input.closest<HTMLElement>("[data-slot=slider-thumb]")!;
    const control = thumb.parentElement!;
    control.getBoundingClientRect = () =>
      ({ left: 0, right: 200, width: 200, top: 0, bottom: 16, height: 16 }) as DOMRect;
    thumb.getBoundingClientRect = () =>
      ({ left: -8, right: 8, width: 16, top: 0, bottom: 16, height: 16 }) as DOMRect;
    control.setPointerCapture = () => {};
    control.hasPointerCapture = () => false;

    fireEvent.pointerDown(thumb, { button: 0, buttons: 1, pointerId: 1, clientX: 0 });
    fireEvent.pointerMove(document, { buttons: 1, pointerId: 1, clientX: 60 });
    fireEvent.pointerMove(document, { buttons: 1, pointerId: 1, clientX: 100 });
    fireEvent.pointerMove(document, { buttons: 1, pointerId: 1, clientX: 120 });
    expect(onSet).not.toHaveBeenCalled();

    fireEvent.pointerUp(document, { pointerId: 1, clientX: 120 });
    expect(onSet).toHaveBeenCalledTimes(1);
    expect(onSet).toHaveBeenCalledWith(60);
  });
});
