import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { SettingsNav } from "@/components/SettingsNav";

describe("SettingsNav", () => {
  it("says what each section is, not just its name", () => {
    // The reason this is a list and not a tab bar: a tab has room for
    // a word, and "Services" on its own says nothing.
    render(<SettingsNav current={null} onPick={vi.fn()} />);
    expect(
      screen.getByRole("button", { name: /Integrations/ }),
    ).toHaveTextContent("The services Niles reads from");
  });

  it("has no page of every key: each key is set where it is used", () => {
    // A list of every key showed most of them twice, beside the
    // integration each belongs to.
    render(<SettingsNav current={null} onPick={vi.fn()} />);
    expect(screen.queryByRole("button", { name: /Credentials/ })).toBeNull();
    expect(screen.getByRole("button", { name: /Integrations/ })).toBeInTheDocument();
  });

  it("opens the section that was pressed", () => {
    const onPick = vi.fn();
    render(<SettingsNav current={null} onPick={onPick} />);
    fireEvent.click(screen.getByRole("button", { name: /Lighting/ }));
    expect(onPick).toHaveBeenCalledWith("lighting");
  });

  it("leads to what Niles remembers", () => {
    const onPick = vi.fn();
    render(<SettingsNav current={null} onPick={onPick} />);
    fireEvent.click(screen.getByRole("button", { name: /Memory/ }));
    expect(onPick).toHaveBeenCalledWith("memory");
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
