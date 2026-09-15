import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ClimatePanel, summarise } from "@/components/ClimatePanel";
import { fractionOf, settingAt } from "@/components/ThermostatDial";
import type { Zone } from "@/lib/api";

function zone(overrides: Partial<Zone> = {}): Zone {
  return {
    id: 6,
    name: "Living Room",
    room: "living_room",
    temperature: 23.9,
    humidity: 60,
    target: null,
    on: false,
    overridden: true,
    reachable: true,
    until: null,
    placed_by: "name",
    ...overrides,
  };
}

function setup(z: Zone) {
  const onHeat = vi.fn();
  const onOff = vi.fn();
  const onResume = vi.fn();
  render(
    <ClimatePanel
      zone={z}
      onHeat={onHeat}
      onOff={onOff}
      onResume={onResume}
    />,
  );
  return { onHeat, onOff, onResume };
}

// The dial is dragged, which jsdom has no geometry for — so the two
// conversions it is built on are tested directly.
describe("the dial's range", () => {
  it("keeps off at the very bottom and the warmest at the top", () => {
    expect(fractionOf(null)).toBe(0);
    expect(fractionOf(25)).toBe(1);
  });

  it("puts the coldest temperature above off, not on it", () => {
    // Off is not a colder temperature, it is the absence of one, so it
    // has a place of its own rather than being what 5° turns into.
    expect(fractionOf(5)).toBeGreaterThan(0);
    expect(settingAt(0)).toBeNull();
    expect(settingAt(fractionOf(5))).toBe(5);
  });

  it("clamps rather than running past tado's range", () => {
    expect(settingAt(2)).toBe(25);
  });

  it("lands on half degrees", () => {
    // The resolution tado takes. A thermostat you can set to 20.37°
    // is one that rounds your answer without telling you.
    for (const fraction of [0.3, 0.45, 0.6, 0.77, 0.9]) {
      const setting = settingAt(fraction)!;
      expect(setting * 2).toBe(Math.round(setting * 2));
    }
  });

  it("reaches every temperature between the two ends", () => {
    expect(settingAt(OFF_EDGE)).toBe(5);
    expect(settingAt(1)).toBe(25);
  });
});

/** Just inside the temperature part of the column. */
const OFF_EDGE = fractionOf(5);

describe("summarise", () => {
  it("names the override that nothing will end", () => {
    // The one worth pointing at: it lasts until somebody remembers it.
    expect(summarise(zone({ overridden: true, until: null }))).toBe(
      "Set by hand, until you resume the schedule",
    );
  });

  it("says when a timed override ends", () => {
    expect(
      summarise(zone({ overridden: true, until: "2026-09-15T18:30:00Z" })),
    ).toMatch(/^Set by hand, until \d/);
  });

  it("says when nobody has interfered", () => {
    expect(summarise(zone({ overridden: false }))).toBe(
      "Following the schedule",
    );
  });
});

describe("ClimatePanel", () => {
  it("says off, and what tado means by it", () => {
    // A zone that is off is not doing nothing — it still heats below
    // about 5° so the pipes survive.
    setup(zone({ on: false }));
    expect(screen.getByText("Off")).toBeInTheDocument();
    expect(screen.getByText("Frost protection")).toBeInTheDocument();
  });

  it("keeps the dial when the zone is off", () => {
    // Turning the heating off and turning it down are the same motion.
    // Swapping the control for a block of text at the end of that
    // motion would break the gesture halfway through.
    setup(zone({ on: false }));
    const dial = screen.getByRole("slider", { name: "Target temperature" });
    expect(dial).toHaveAttribute("aria-valuetext", "Off");
  });

  it("shows the dial at the temperature it is holding", () => {
    setup(zone({ on: true, target: 21 }));
    expect(
      screen.getByRole("slider", { name: "Target temperature" }),
    ).toHaveAttribute("aria-valuenow", "21");
  });

  it("turns the zone off by taking the dial to the bottom", () => {
    const { onOff } = setup(zone({ on: true, target: 5 }));
    fireEvent.keyDown(
      screen.getByRole("slider", { name: "Target temperature" }),
      { key: "ArrowDown" },
    );
    expect(onOff).toHaveBeenCalled();
  });

  it("hands the zone back to the schedule", () => {
    const { onResume } = setup(zone({ overridden: true }));
    fireEvent.click(screen.getByRole("button", { name: /Resume schedule/ }));
    expect(onResume).toHaveBeenCalled();
  });

  it("offers nothing to resume when nothing is overridden", () => {
    setup(zone({ overridden: false }));
    expect(screen.queryByRole("button", { name: /Resume schedule/ })).toBeNull();
  });

  it("refuses to show a reading it cannot vouch for", () => {
    // tado keeps sending the last temperature it heard from an offline
    // valve. Showing it beside a live one is the lie this avoids.
    setup(zone({ reachable: false, temperature: null, humidity: null }));
    expect(screen.getByText("Not answering")).toBeInTheDocument();
    expect(screen.queryByRole("slider")).toBeNull();
  });
});
