import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { SpeakersCard, describe as describeSpeaker } from "./SpeakersCard";
import type { SonosSpeaker, SpeakersReport } from "@/lib/api";

const BAR: SonosSpeaker = {
  id: "RINCON_BAR",
  name: "Living Room",
  ip: "192.168.10.122",
  home_theater: true,
  room: "living_room",
  answering: true,
};
const MOVE: SonosSpeaker = {
  id: "RINCON_MOVE",
  name: "Sonos Move",
  ip: "192.168.10.174",
  home_theater: false,
  room: null,
  answering: true,
};

function report(sonos: SonosSpeaker[], error: string | null = null): SpeakersReport {
  return { configured: true, error, sonos };
}

const props = {
  rooms: ["kitchen", "living_room"],
  onPlace: vi.fn(),
  onUnplace: vi.fn(),
};

describe("SpeakersCard", () => {
  it("lists each Sonos room with where it plays", () => {
    render(
      <SpeakersCard report={report([BAR, MOVE])} placed={{ RINCON_BAR: "living_room" }} {...props} />,
    );
    expect(screen.getByText("Living Room")).toBeInTheDocument();
    expect(screen.getByText("Sonos Move")).toBeInTheDocument();
    // Not opened: Base UI's Select popup hangs jsdom.
    expect(screen.getByRole("combobox", { name: "Living Room room" })).toHaveTextContent(
      "Living room",
    );
    expect(screen.getByRole("combobox", { name: "Sonos Move room" })).toHaveTextContent(
      "Not in a room",
    );
  });

  it("sends somebody to Integrations before there is an address", () => {
    render(
      <SpeakersCard report={{ configured: false, error: null, sonos: [] }} placed={{}} {...props} />,
    );
    expect(screen.getByText(/Add Sonos under Integrations first/)).toBeInTheDocument();
  });

  it("says why when no speaker answered", () => {
    render(
      <SpeakersCard
        report={report([], "No Sonos answered at 192.168.10.174")}
        placed={{}}
        {...props}
      />,
    );
    expect(screen.getByText("No Sonos answered at 192.168.10.174")).toBeInTheDocument();
  });
});

describe("describe", () => {
  it("names the soundbar as the one that plays the TV", () => {
    expect(describeSpeaker(BAR)).toBe("Soundbar, plays the TV · 192.168.10.122");
    expect(describeSpeaker(MOVE)).toBe("Speaker · 192.168.10.174");
  });

  it("says when a placed speaker did not answer", () => {
    expect(describeSpeaker({ ...MOVE, answering: false, ip: null })).toBe("Not answering");
  });
});
