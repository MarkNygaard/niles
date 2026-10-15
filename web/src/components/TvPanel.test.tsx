import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { TvPanel, macLine, pairingLine } from "./TvPanel";
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

  it("says when it cannot be woken", () => {
    expect(pairingLine(tv({ paired: true, mac: "a8:23:fe:01:02:03" }))).toBe("Paired.");
    expect(pairingLine(tv({ paired: true }))).toMatch(/cannot turn it on/);
  });
});

describe("macLine", () => {
  it("says a MAC was detected", () => {
    expect(macLine(tv({ mac: "84:90:0a:63:a9:94", mac_source: "detected" }))).toBe(
      "Detected from the TV when it was paired.",
    );
  });

  it("says a MAC was set by hand, and what the TV reported", () => {
    const line = macLine(
      tv({ mac: "64:e4:a5:6c:ab:b4", mac_source: "hand", detected_mac: "84:90:0a:63:a9:94" }),
    );
    expect(line).toMatch(/^Set by hand, and kept when the TV is paired again\./);
    expect(line).toMatch(/The TV reported 84:90:0a:63:a9:94\./);
  });

  it("says where one comes from before there is one", () => {
    expect(macLine(tv({}))).toMatch(/paired/);
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

  it("shows the wake MAC and saves one typed", () => {
    const onChange = vi.fn();
    render(
      <TvPanel
        host="192.168.69.10"
        room="living_room"
        showAnnouncements
        tv={tv({ paired: true, mac: "64:e4:a5:6c:ab:b4", mac_source: "detected" })}
        rooms={["living_room"]}
        onChange={onChange}
        onPaired={vi.fn()}
      />,
    );
    const field = screen.getByRole("textbox", { name: "Wake MAC" });
    expect(field).toHaveValue("64:e4:a5:6c:ab:b4");
    expect(screen.getByText("Detected from the TV when it was paired.")).toBeInTheDocument();
    fireEvent.change(field, { target: { value: "84:90:0a:63:a9:94" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([{ path: "tv.mac", value: "84:90:0a:63:a9:94" }]);
  });

  it("clears a MAC set by hand", () => {
    const onChange = vi.fn();
    render(
      <TvPanel
        host="192.168.69.10"
        room="living_room"
        showAnnouncements
        tv={tv({
          paired: true,
          mac: "64:e4:a5:6c:ab:b4",
          mac_source: "hand",
          detected_mac: "84:90:0a:63:a9:94",
        })}
        rooms={["living_room"]}
        onChange={onChange}
        onPaired={vi.fn()}
      />,
    );
    expect(screen.getByText(/Set by hand/)).toBeInTheDocument();
    const field = screen.getByRole("textbox", { name: "Wake MAC" });
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([{ path: "tv.mac", value: "" }]);
  });
});
