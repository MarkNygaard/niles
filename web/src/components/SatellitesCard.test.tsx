import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { SatellitesCard, roomChoices } from "@/components/SatellitesCard";
import type { Satellite } from "@/components/SatellitesCard";

const KITCHEN: Satellite = {
  name: "kitchen_echo",
  ip: "192.168.42.30",
  room: "kitchen",
};

function setup(satellites: Satellite[], rooms = ["kitchen", "living_room"]) {
  const onChange = vi.fn();
  const onRemove = vi.fn();
  render(
    <SatellitesCard
      satellites={satellites}
      rooms={rooms}
      onChange={onChange}
      onRemove={onRemove}
    />,
  );
  return { onChange, onRemove };
}

// The dropdown itself cannot be opened in jsdom — Base UI's Select
// hangs it — so the rule that decides what it offers is tested here.
describe("roomChoices", () => {
  it("offers the rooms Niles knows about", () => {
    expect(roomChoices(["kitchen", "living_room"])).toEqual([
      "kitchen",
      "living_room",
    ]);
  });

  it("keeps a room already assigned that has no devices in it", () => {
    // Dropping it would make the box claim the satellite is somewhere
    // it is not — a room with no lights yet is still a room.
    expect(roomChoices(["kitchen"], "study")).toEqual(["kitchen", "study"]);
  });

  it("does not offer the assigned room twice", () => {
    expect(roomChoices(["kitchen"], "kitchen")).toEqual(["kitchen"]);
  });
});

describe("SatellitesCard", () => {
  it("says a satellite still works unlisted, rather than looking broken", () => {
    setup([]);
    expect(screen.getByText(/cannot be answered about/)).toBeInTheDocument();
  });

  it("writes an address when the field is left, not per keystroke", () => {
    const { onChange } = setup([KITCHEN]);
    const field = screen.getByLabelText("kitchen_echo address");
    fireEvent.change(field, { target: { value: "192.168.42.31" } });
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([
      { path: "satellites.kitchen_echo.ip", value: "192.168.42.31" },
    ]);
  });

  it("adds one already placed in a room", () => {
    // The config refuses a room that is not a canonical name, so an
    // unplaced satellite could not be saved at all.
    const { onChange } = setup([]);
    fireEvent.change(screen.getByLabelText("Satellite name"), {
      target: { value: "Study Echo" },
    });
    fireEvent.change(screen.getByLabelText("Satellite address"), {
      target: { value: "192.168.42.40" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Add/ }));
    expect(onChange).toHaveBeenCalledWith([
      {
        path: "satellites.study_echo",
        value: { ip: "192.168.42.40", room: "kitchen" },
      },
    ]);
  });

  it("refuses a name that would break the path it is stored under", () => {
    // Paths are split on dots, so a name with one lands somewhere else.
    const { onChange } = setup([]);
    fireEvent.change(screen.getByLabelText("Satellite name"), {
      target: { value: "kitchen.echo" },
    });
    fireEvent.change(screen.getByLabelText("Satellite address"), {
      target: { value: "192.168.42.40" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Add/ }));
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByText(/letters, numbers and underscores/)).toBeInTheDocument();
  });

  it("removes by dropping the whole entry", () => {
    // A map has no way to say "gone" in a patch: a smaller map merges
    // rather than replaces.
    const { onRemove, onChange } = setup([KITCHEN]);
    fireEvent.click(screen.getByRole("button", { name: "Remove kitchen_echo" }));
    expect(onRemove).toHaveBeenCalledWith("kitchen_echo");
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("volume", () => {
  it("shows the shipped 100% when the entry says nothing", () => {
    // An entry written before volume existed is not a silent one.
    setup([KITCHEN]);
    expect(screen.getByText("100%")).toBeInTheDocument();
  });

  it("shows what is configured", () => {
    setup([{ ...KITCHEN, volume: 40 }]);
    expect(screen.getByText("40%")).toBeInTheDocument();
  });

  it("gives the control a name that says which satellite it belongs to", () => {
    // There is one of these per satellite, so "Volume" alone would be
    // ambiguous to anybody not looking at the screen.
    setup([KITCHEN, { ...KITCHEN, name: "office_sat", ip: "192.168.42.31" }]);
    expect(
      screen.getByRole("slider", { name: "kitchen_echo volume" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("slider", { name: "office_sat volume" }),
    ).toBeInTheDocument();
  });
});
