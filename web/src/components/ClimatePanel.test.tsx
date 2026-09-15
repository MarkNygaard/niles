import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ClimatePanel, summarise } from "@/components/ClimatePanel";
import { celsiusAt, fractionOf } from "@/components/ThermostatDial";
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
  it("puts the coldest setting at the bottom", () => {
    expect(fractionOf(5)).toBe(0);
    expect(fractionOf(25)).toBe(1);
    expect(fractionOf(15)).toBe(0.5);
  });

  it("clamps rather than running past tado's range", () => {
    expect(celsiusAt(-1)).toBe(5);
    expect(celsiusAt(2)).toBe(25);
  });

  it("lands on half degrees", () => {
    // The resolution tado takes. A thermostat you can set to 20.37°
    // is one that rounds your answer without telling you.
    expect(celsiusAt(0.5)).toBe(15);
    // 15.2 rounds down to 15, 15.4 rounds up to 15.5 — never to a
    // third decimal nobody asked for.
    expect(celsiusAt(0.51)).toBe(15);
    expect(celsiusAt(0.52)).toBe(15.5);
  });
});

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

  it("offers the dial only when the zone is heating", () => {
    setup(zone({ on: true, target: 21 }));
    expect(
      screen.getByRole("slider", { name: "Target temperature" }),
    ).toBeInTheDocument();
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
