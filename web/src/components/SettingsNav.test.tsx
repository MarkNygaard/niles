import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { SettingsNav } from "@/components/SettingsNav";

describe("SettingsNav", () => {
  it("says what each section is, not just its name", () => {
    // The reason this is a list and not a tab bar: a tab has room for
    // a word, and "Services" on its own says nothing.
    render(<SettingsNav current={null} onPick={vi.fn()} />);
    expect(
      screen.getByRole("button", { name: /Services/ }),
    ).toHaveTextContent("tado, and the credentials Niles needs");
  });

  it("opens the section that was pressed", () => {
    const onPick = vi.fn();
    render(<SettingsNav current={null} onPick={onPick} />);
    fireEvent.click(screen.getByRole("button", { name: /Lighting/ }));
    expect(onPick).toHaveBeenCalledWith("lighting");
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
});
