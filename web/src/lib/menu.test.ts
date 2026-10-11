import { describe, expect, it } from "vitest";
import { menuLayout } from "./menu";

const ids = (items: { id: string }[]) => items.map((i) => i.id);

describe("menuLayout", () => {
  it("puts Settings and Automations in the avatar menu and the rest at the bottom at first", () => {
    const layout = menuLayout({});
    expect(ids(layout.avatar)).toEqual(["settings", "automations"]);
    expect(ids(layout.main)).toEqual(["groceries", "chat", "media"]);
  });

  it("follows both arrangements", () => {
    const layout = menuLayout({
      menu: { avatar: ["automations", "chat", "settings"], order: ["media", "groceries"] },
    });
    expect(ids(layout.avatar)).toEqual(["automations", "chat", "settings"]);
    expect(ids(layout.main)).toEqual(["media", "groceries"]);
  });

  it("puts an entry neither names at the end of the main navigation", () => {
    // A page added later has to turn up somewhere.
    const layout = menuLayout({ menu: { avatar: [], order: ["chat"] } });
    expect(ids(layout.main)).toEqual(["chat", "groceries", "media", "settings"]);
  });

  it("puts Automations at the end of the avatar menu while neither names it", () => {
    // A menu arranged before the page existed gains no tab for it.
    const layout = menuLayout({
      menu: { avatar: ["chat", "settings"], order: ["media", "groceries"] },
    });
    expect(ids(layout.avatar)).toEqual(["chat", "settings", "automations"]);
    expect(ids(layout.main)).not.toContain("automations");
  });

  it("keeps Automations where it was put", () => {
    const main = menuLayout({ menu: { avatar: ["settings"], order: ["automations", "chat"] } });
    expect(ids(main.avatar)).toEqual(["settings"]);
    expect(ids(main.main).slice(0, 2)).toEqual(["automations", "chat"]);
    const avatar = menuLayout({ menu: { avatar: ["automations", "settings"] } });
    expect(ids(avatar.avatar)).toEqual(["automations", "settings"]);
  });

  it("hides Automations like any other entry", () => {
    const layout = menuLayout({ menu: { hidden: ["automations"] } });
    expect(layout.avatar.find((m) => m.id === "automations")!.hidden).toBe(true);
  });

  it("ignores names it does not know and names given twice", () => {
    const layout = menuLayout({ menu: { avatar: ["me", "settings", "settings", 3] } });
    expect(ids(layout.avatar)).toEqual(["settings", "automations"]);
  });

  it("hides what is hidden, but never Settings", () => {
    const layout = menuLayout({ menu: { hidden: ["groceries", "settings"] } });
    expect(layout.main.find((m) => m.id === "groceries")!.hidden).toBe(true);
    expect(layout.avatar.find((m) => m.id === "settings")!.hidden).toBe(false);
  });

  it("shows Media while something plays, unless told otherwise", () => {
    const media = (effective: unknown, playing?: boolean) =>
      menuLayout(effective, playing).main.find((m) => m.id === "media")!.hidden;
    expect(media({}, false)).toBe(true);
    expect(media({}, true)).toBe(false);
    expect(media({ menu: { media: "always" } }, false)).toBe(false);
    expect(media({ menu: { media: "never" } }, true)).toBe(true);
    // Settings lists it whatever plays.
    expect(media({}, undefined)).toBe(false);
  });
});
