import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { SECTIONS, SettingsNav, labelOf, sectionId } from "@/components/SettingsNav";

describe("SettingsNav", () => {
  it("says what each section is, not just its name", () => {
    // The reason this is a list and not a tab bar: a tab has room for
    // a word, and "Devices" on its own says nothing.
    render(<SettingsNav current={null} onPick={vi.fn()} />);
    expect(screen.getByRole("button", { name: /Devices/ })).toHaveTextContent(
      "The broker your Zigbee lights and WLED strips are reached through",
    );
  });

  it("splits the base system from the add-ons", () => {
    expect(SECTIONS.map((g) => g.group)).toEqual(["The house", "System", "Add-ons", "Advanced"]);
    expect(SECTIONS[1].items.map((i) => i.label)).toEqual(["Devices", "Voice & AI"]);
    expect(SECTIONS[2].items.map((i) => i.label)).toEqual(["Add-ons"]);
  });

  it("has no page called Integrations any more", () => {
    render(<SettingsNav current={null} onPick={vi.fn()} />);
    expect(screen.queryByRole("button", { name: /Integrations/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Credentials/ })).toBeNull();
  });

  it("opens the section that was pressed", () => {
    const onPick = vi.fn();
    render(<SettingsNav current={null} onPick={onPick} />);
    fireEvent.click(screen.getByRole("button", { name: /Voice & AI/ }));
    expect(onPick).toHaveBeenCalledWith("voice");
    fireEvent.click(screen.getByRole("button", { name: /^Add-ons/ }));
    expect(onPick).toHaveBeenCalledWith("addons");
  });

  it("marks the open one for a screen reader, not just with a tint", () => {
    render(<SettingsNav current="people" onPick={vi.fn()} />);
    expect(screen.getByRole("button", { name: /People/ })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("button", { name: /Lighting/ })).not.toHaveAttribute(
      "aria-current",
    );
  });

  it("counts what is left to set up beside each section", () => {
    // A new install sees where to start without opening every page.
    render(<SettingsNav current={null} onPick={vi.fn()} todo={{ devices: 2, voice: 1 }} />);
    expect(screen.getByRole("button", { name: /Devices/ })).toHaveTextContent("2 to set up");
    expect(screen.getByRole("button", { name: /Voice & AI/ })).toHaveTextContent("1 to set up");
    expect(screen.getByRole("button", { name: /Lighting/ })).not.toHaveTextContent("to set up");
  });
});

describe("sectionId", () => {
  it("sends the old pages to where their contents went", () => {
    // `#/me/settings/integrations` and `#/me/settings/language` from a
    // bookmark or the installed app land somewhere, not on a blank pane.
    expect(sectionId("integrations")).toBe("addons");
    expect(sectionId("language")).toBe("voice");
    expect(labelOf("language")).toBe("Voice & AI");
  });

  it("keeps the ones that did not move", () => {
    expect(sectionId("devices")).toBe("devices");
    expect(sectionId("history")).toBe("history");
  });

  it("is nothing for a section that never existed", () => {
    expect(sectionId("nonsense")).toBeNull();
    expect(sectionId(null)).toBeNull();
  });
});
