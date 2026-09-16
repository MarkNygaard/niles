import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { AccountMenu, initials } from "@/components/AccountMenu";

describe("initials", () => {
  it("takes both parts of a dotted address", () => {
    expect(initials("mark.nygaard@hotmail.com")).toBe("MN");
  });

  it("falls back to the first two letters", () => {
    expect(initials("majse@example.com")).toBe("MA");
  });

  it("handles the other separators people use", () => {
    expect(initials("mark_nygaard@example.com")).toBe("MN");
    expect(initials("mark-nygaard@example.com")).toBe("MN");
  });

  it("has something to draw when nobody is signed in", () => {
    // Sign-in can be off entirely, and the menu still holds the
    // appearance setting.
    expect(initials(undefined)).toBe("·");
  });
});

describe("AccountMenu", () => {
  it("shuts itself on the way to settings", () => {
    // It used to stay open over the page it had just navigated to.
    const onOpenSettings = vi.fn();
    render(<AccountMenu email="mark@example.com" onOpenSettings={onOpenSettings} />);
    fireEvent.click(screen.getByRole("button", { name: /Account/ }));
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: "Settings" })).toBeNull();
  });
});
