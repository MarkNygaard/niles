import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  CurveChart,
  brightnessAt,
  colorTempAt,
  isPaused,
  readCurve,
} from "@/components/CurveChart";

/**
 * The same config `curve.rs` tests against, so the expectations below
 * can be read straight off that file. A chart that drew something other
 * than what Niles runs would be worse than no chart.
 */
const LIGHTING = {
  morning_start: "05:45",
  morning_end: "06:30",
  sunset_start: "21:30",
  sunset_end: "23:00",
  night_floor_brightness: 15,
  daytime_brightness: 100,
  color_temp_anchors: [
    { time: "00:00", kelvin: 2000 },
    { time: "05:45", kelvin: 2000 },
    { time: "06:30", kelvin: 2700 },
    { time: "12:00", kelvin: 4500 },
    { time: "23:00", kelvin: 2000 },
  ],
};

const curve = readCurve(LIGHTING)!;
const at = (hours: number, minutes: number) => hours * 60 + minutes;

describe("brightnessAt", () => {
  it("holds the night floor outside the ramps", () => {
    expect(brightnessAt(curve, at(3, 0))).toBe(15);
    expect(brightnessAt(curve, at(0, 0))).toBe(15);
    expect(brightnessAt(curve, at(23, 30))).toBe(15);
  });

  it("does not jump at the start of the morning ramp", () => {
    // The architecture's continuity invariant.
    expect(brightnessAt(curve, at(5, 45))).toBe(15);
  });

  it("climbs across the morning ramp and rests at daytime", () => {
    expect(brightnessAt(curve, at(6, 30))).toBe(100);
    expect(brightnessAt(curve, at(12, 0))).toBe(100);
    expect(brightnessAt(curve, at(6, 7))).toBeGreaterThan(15);
    expect(brightnessAt(curve, at(6, 7))).toBeLessThan(100);
  });

  it("falls back across the sunset ramp", () => {
    expect(brightnessAt(curve, at(21, 30))).toBe(100);
    expect(brightnessAt(curve, at(23, 0))).toBe(15);
  });
});

describe("colorTempAt", () => {
  it("holds the first and last anchor beyond their ends", () => {
    expect(colorTempAt(curve, at(0, 0))).toBe(2000);
    expect(colorTempAt(curve, at(23, 59))).toBe(2000);
  });

  it("interpolates between anchors", () => {
    expect(colorTempAt(curve, at(6, 30))).toBe(2700);
    const midMorning = colorTempAt(curve, at(9, 15));
    expect(midMorning).toBeGreaterThan(2700);
    expect(midMorning).toBeLessThan(4500);
  });
});

describe("isPaused", () => {
  const pause = readCurve({
    ...LIGHTING,
    curve_pause_start: "fri 12:00",
    curve_pause_end: "sun 12:00",
  })!.pause!;

  it("covers the window it names", () => {
    // 2026-09-12 is a Saturday; 2026-09-11 a Friday.
    expect(isPaused(pause, new Date("2026-09-12T18:00:00"))).toBe(true);
    expect(isPaused(pause, new Date("2026-09-11T12:00:00"))).toBe(true);
    expect(isPaused(pause, new Date("2026-09-11T11:59:00"))).toBe(false);
    expect(isPaused(pause, new Date("2026-09-13T12:00:00"))).toBe(false);
  });
});

describe("readCurve", () => {
  it("refuses a config missing part of the curve", () => {
    // Half a curve drawn as a whole one would misinform.
    const { sunset_end, ...incomplete } = LIGHTING;
    expect(sunset_end).toBe("23:00");
    expect(readCurve(incomplete)).toBeNull();
    expect(readCurve({ ...LIGHTING, color_temp_anchors: [] })).toBeNull();
    expect(readCurve(undefined)).toBeNull();
  });
});

describe("CurveChart", () => {
  it("draws nothing rather than a half-curve", () => {
    const { container } = render(<CurveChart lighting={{}} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("names the levels it holds between", () => {
    render(<CurveChart lighting={LIGHTING} />);
    expect(screen.getByText("15%")).toBeInTheDocument();
    expect(screen.getByText("100%")).toBeInTheDocument();
  });
});
