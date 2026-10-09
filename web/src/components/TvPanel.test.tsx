import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { TvPanel, pairingLine } from "./TvPanel";
import type { TvInfo } from "@/lib/api";

function tv(over: Partial<TvInfo>): TvInfo {
  return {
    configured: true,
    paired: false,
    mac: null,
    room: null,
    status: null,
    error: null,
    ...over,
  };
}

describe("pairingLine", () => {
  it("asks for the address before anything else", () => {
    expect(pairingLine(undefined)).toMatch(/address/);
    expect(pairingLine(tv({ configured: false }))).toMatch(/address/);
  });

  it("says when it is not paired", () => {
    expect(pairingLine(tv({}))).toBe("Not paired yet.");
  });

  it("says what it will be woken by, or that it cannot be", () => {
    expect(pairingLine(tv({ paired: true, mac: "a8:23:fe:01:02:03" }))).toBe(
      "Paired. Woken by a8:23:fe:01:02:03.",
    );
    expect(pairingLine(tv({ paired: true }))).toMatch(/cannot turn it on/);
  });
});

describe("TvPanel", () => {
  it("draws before a room is chosen", () => {
    // A new TV has no room yet; this used to blank the whole page.
    render(
      <TvPanel
        host=""
        room=""
        showAnnouncements
        rooms={["kitchen", "living_room"]}
        onChange={vi.fn()}
        onPaired={vi.fn()}
      />,
    );
    expect(screen.getByRole("combobox", { name: "TV room" })).toHaveTextContent("Pick a room");
    expect(screen.getByRole("button", { name: "Pair" })).toBeDisabled();
  });

  it("draws a paired TV in its room", () => {
    render(
      <TvPanel
        host="192.168.69.10"
        room="living_room"
        showAnnouncements={false}
        tv={{
          configured: true,
          paired: true,
          mac: "a8:23:fe:01:02:03",
          room: "living_room",
          status: { on: true, app: "Netflix" },
          error: null,
        }}
        rooms={["kitchen", "living_room"]}
        onChange={vi.fn()}
        onPaired={vi.fn()}
      />,
    );
    expect(screen.getByRole("combobox", { name: "TV room" })).toHaveTextContent("Living room");
    expect(screen.getByRole("button", { name: "Pair again" })).toBeEnabled();
  });
});
