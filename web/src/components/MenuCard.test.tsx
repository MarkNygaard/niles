import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MEDIA_SHOWN, MenuCard, place, spotAt, step } from "./MenuCard";
import type { MenuLayout } from "@/lib/menu";

const LAYOUT: MenuLayout = {
  avatar: [{ id: "settings", hidden: false }],
  main: [
    { id: "groceries", hidden: false },
    { id: "chat", hidden: false },
    { id: "media", hidden: false },
  ],
};

describe("place", () => {
  it("moves an entry within a menu and between them", () => {
    const lists = { avatar: ["settings" as const], main: ["groceries" as const, "chat" as const] };
    expect(place(lists, "chat", { list: "main", at: 0 })).toEqual({
      avatar: ["settings"],
      main: ["chat", "groceries"],
    });
    expect(place(lists, "groceries", { list: "avatar", at: 1 })).toEqual({
      avatar: ["settings", "groceries"],
      main: ["chat"],
    });
  });
});

describe("spotAt", () => {
  // Rows: 0 avatar heading, 1 My profile, 2 settings, 3 main heading,
  // 4 Home, 5 groceries, 6 chat.
  it("reads each row as a place", () => {
    expect(spotAt(1, 1)).toEqual({ list: "avatar", at: 0 });
    expect(spotAt(2, 1)).toEqual({ list: "avatar", at: 0 });
    // The main navigation's heading is the end of the avatar menu.
    expect(spotAt(3, 1)).toEqual({ list: "avatar", at: 1 });
    // Home is the start of the main navigation.
    expect(spotAt(4, 1)).toEqual({ list: "main", at: 0 });
    expect(spotAt(6, 1)).toEqual({ list: "main", at: 1 });
  });
});

describe("step", () => {
  const lists = { avatar: ["settings" as const], main: ["groceries" as const, "chat" as const] };

  it("moves past the top of the main navigation into the avatar menu", () => {
    expect(step(lists, "groceries", -1)).toEqual({
      avatar: ["settings", "groceries"],
      main: ["chat"],
    });
  });

  it("moves past the end of the avatar menu into the main navigation", () => {
    expect(step(lists, "settings", 1)).toEqual({
      avatar: [],
      main: ["settings", "groceries", "chat"],
    });
  });

  it("goes no further than the ends", () => {
    expect(step(lists, "settings", -1)).toEqual(lists);
    expect(step(lists, "chat", 1)).toEqual(lists);
  });
});

describe("MenuCard", () => {
  function setup(onChange = vi.fn()) {
    render(
      <MenuCard layout={LAYOUT} onChange={onChange} mediaShown="playing" onMediaShown={vi.fn()} />,
    );
    return onChange;
  }

  it("shows both menus, each with its locked first entry", () => {
    setup();
    const text = document.body.textContent ?? "";
    const order = ["Avatar menu", "My profile", "Settings", "Main navigation", "Home", "Groceries"];
    const positions = order.map((label) => text.indexOf(label));
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
    expect(screen.getAllByRole("button", { name: /^Move / })).toHaveLength(4);
  });

  it("moves an entry into the avatar menu with the arrow keys", () => {
    const onChange = setup();
    fireEvent.keyDown(screen.getByRole("button", { name: "Move Groceries" }), { key: "ArrowUp" });
    expect(onChange).toHaveBeenCalledWith({
      avatar: ["settings", "groceries"],
      main: ["chat", "media"],
      hidden: [],
    });
  });

  it("hides with a switch, but Settings has none", () => {
    const onChange = setup();
    fireEvent.click(screen.getByRole("switch", { name: "Show Chat" }));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ hidden: ["chat"] }),
    );
    expect(screen.queryByRole("switch", { name: "Show Settings" })).toBeNull();
  });

  it("offers Media three answers rather than a switch", () => {
    setup();
    // Not opened: Base UI's Select popup hangs jsdom.
    expect(screen.getByRole("combobox", { name: "Show Media" })).toHaveTextContent(
      "When something plays",
    );
    expect(MEDIA_SHOWN.map((o) => o.value)).toEqual(["playing", "always", "never"]);
  });
});
