import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { AvatarMenu } from "./AvatarMenu";

describe("AvatarMenu", () => {
  it("opens to the profile, what was put there, the look and signing out", () => {
    render(
      <AvatarMenu
        email="mark@example.com"
        items={[
          { id: "settings", hidden: false },
          { id: "chat", hidden: true },
        ]}
        theme="system"
        onTheme={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Your menu" }));
    expect(screen.getByRole("link", { name: "My profile" })).toHaveAttribute("href", "#/me/profile");
    expect(screen.getByRole("link", { name: "Settings" })).toHaveAttribute("href", "#/me/settings");
    // Hidden is hidden here as in the bar.
    expect(screen.queryByRole("link", { name: "Chat" })).toBeNull();
    expect(screen.getByRole("link", { name: "Sign out" })).toBeInTheDocument();
  });

  it("changes the look", () => {
    const onTheme = vi.fn();
    render(<AvatarMenu email="mark@example.com" items={[]} theme="light" onTheme={onTheme} />);
    fireEvent.click(screen.getByRole("button", { name: "Your menu" }));
    fireEvent.click(screen.getByRole("button", { name: "Dark" }));
    expect(onTheme).toHaveBeenCalledWith("dark");
  });

  it("has no profile and no signing out with sign-in off", () => {
    render(<AvatarMenu items={[{ id: "settings", hidden: false }]} theme="light" onTheme={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Your menu" }));
    expect(screen.queryByRole("link", { name: "My profile" })).toBeNull();
    expect(screen.queryByRole("link", { name: "Sign out" })).toBeNull();
    expect(screen.getByRole("link", { name: "Settings" })).toBeInTheDocument();
  });
});
