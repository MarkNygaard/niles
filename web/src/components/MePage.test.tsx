import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MePage } from "@/components/MePage";

describe("MePage", () => {
  it("leads to your profile, Settings, and out", () => {
    render(<MePage email="majse@example.com" theme="system" onTheme={vi.fn()} />);
    expect(screen.getByRole("link", { name: /My profile/ })).toHaveAttribute(
      "href",
      "#/me/profile",
    );
    expect(screen.getByRole("link", { name: /Settings/ })).toHaveAttribute(
      "href",
      "#/me/settings",
    );
    expect(screen.getByRole("link", { name: "Sign out" })).toHaveAttribute(
      "href",
      "/auth/signout",
    );
  });

  it("has no profile and nobody to sign out with sign-in off", () => {
    render(<MePage theme="system" onTheme={vi.fn()} />);
    expect(screen.queryByRole("link", { name: /My profile/ })).toBeNull();
    expect(screen.queryByRole("link", { name: "Sign out" })).toBeNull();
    // The house's settings and the theme are still somebody's business.
    expect(screen.getByRole("link", { name: /Settings/ })).toBeInTheDocument();
  });

  it("switches the theme", () => {
    const onTheme = vi.fn();
    render(<MePage theme="system" onTheme={onTheme} />);
    expect(screen.getByRole("button", { name: /Auto/ })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: /Dark/ }));
    expect(onTheme).toHaveBeenCalledWith("dark");
  });
});
