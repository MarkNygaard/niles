import { describe, expect, it } from "vitest";
import type { PluginSetting, PluginSummary } from "@/lib/api";
import {
  choiceOptions,
  draftOf,
  imageUrl,
  missingRequired,
  pageHref,
  pageRoute,
  placeWidgets,
  pluginGroupsOf,
  pluginPages,
  roomOptions,
  settingsPatch,
  setupLine,
  widgetPlace,
  widgetPlaceOptions,
} from "@/lib/plugins";

function setting(over: Partial<PluginSetting>): PluginSetting {
  return {
    key: "x",
    label: "X",
    type: "string",
    required: false,
    help: null,
    default: null,
    options: [],
    min: null,
    max: null,
    value: null,
    set: false,
    ...over,
  };
}

const PLUGIN: PluginSummary = {
  id: "spotprice",
  name: "Spot prices",
  version: "0.1.0",
  description: "",
  authors: [],
  homepage: null,
  group: { id: "custom", label: "Custom" },
  status: "running",
  status_line: "Working",
  error: null,
  dropped_events: 0,
  enabled: true,
  repo: null,
  release_tag: null,
  release_url: null,
  installed_at: null,
  updated_at: null,
  logo: null,
  previous_version: null,
  update: null,
  settings: [],
  menu: [{ id: "electricity", label: "Electricity", icon: "zap", refresh_every: 300 }],
  widgets: [
    { id: "now", title: "Now", room: "kitchen", refresh_every: null },
    { id: "chart", title: "Chart", room: null, refresh_every: null },
    { id: "away", title: "Away", room: "garage", refresh_every: null },
  ],
  interface: "0.1",
  older_interface: false,
};

describe("pluginGroupsOf", () => {
  it("keeps the order groups appear in, with Custom last", () => {
    const groups = pluginGroupsOf([
      { ...PLUGIN, id: "a" },
      { ...PLUGIN, id: "b", group: { id: "music", label: "Music" } },
      { ...PLUGIN, id: "c" },
    ]);
    expect(groups.map((g) => g.group.id)).toEqual(["music", "custom"]);
    expect(groups[1].items.map((p) => p.id)).toEqual(["a", "c"]);
  });
});

describe("the setup panel's helpers", () => {
  const settings = [
    setting({ key: "name", value: "Old" }),
    setting({ key: "hours", type: "number", min: 1, max: 24, value: 4 }),
    setting({ key: "loud", type: "boolean", default: true }),
    setting({ key: "token", type: "secret", required: true }),
  ];

  it("opens on what was entered, booleans on their default, and no secrets", () => {
    expect(draftOf(settings)).toEqual({ name: "Old", hours: "4", loud: true });
  });

  it("writes only what changed, an emptied field as null, under plugins.<id>", () => {
    expect(
      settingsPatch("spotprice", settings, { name: "", hours: "6,5", loud: true }),
    ).toEqual({ patch: { plugins: { spotprice: { name: null, hours: 6.5 } } }, errors: {} });
    expect(
      settingsPatch("spotprice", settings, { name: "Old", hours: "4", loud: true }).patch,
    ).toBeNull();
    expect(settingsPatch("spotprice", settings, { loud: false }).patch).toEqual({
      plugins: { spotprice: { loud: false } },
    });
  });

  it("refuses what is not a number, or outside its range", () => {
    expect(settingsPatch("p", settings, { hours: "lots" }).errors).toEqual({
      hours: "X is a number.",
    });
    expect(settingsPatch("p", settings, { hours: "0" }).errors.hours).toContain("between 1 and 24");
  });

  it("names the required settings without a value or default", () => {
    const required = [
      setting({ key: "a", label: "Area", required: true, default: "DK1" }),
      setting({ key: "k", label: "API key", type: "secret", required: true }),
      setting({ key: "u", label: "User", required: true }),
    ];
    expect(missingRequired(required)).toEqual(["API key", "User"]);
    expect(setupLine({ ...PLUGIN, settings: required })).toBe(
      "Not set up: needs API key and User",
    );
    expect(setupLine({ ...PLUGIN, settings: [] })).toBe("Working");
  });

  it("offers a choice's options, and the house's rooms with the chosen one kept", () => {
    expect(
      choiceOptions(setting({ type: "choice", options: [{ value: "DK1", label: "West" }] })),
    ).toEqual([{ value: "DK1", label: "West" }]);
    expect(roomOptions(["living_room"], "attic")).toEqual([
      { value: "living_room", label: "Living room" },
      { value: "attic", label: "Attic" },
    ]);
  });
});

describe("pages in the menu", () => {
  it("are the pages of the plugins that are on", () => {
    expect(pluginPages([PLUGIN, { ...PLUGIN, id: "off", enabled: false }])).toEqual([
      {
        id: "spotprice.electricity",
        plugin: "spotprice",
        page: "electricity",
        label: "Electricity",
        icon: "zap",
        refresh_every: 300,
      },
    ]);
  });

  it("each have a route of their own", () => {
    const [page] = pluginPages([PLUGIN]);
    expect(pageHref(page)).toBe("#/plugins/spotprice/electricity");
    expect(pageRoute("/plugins/spotprice/electricity")).toEqual({
      plugin: "spotprice",
      page: "electricity",
    });
    expect(pageRoute("/plugins/spotprice")).toBeNull();
  });
});

describe("widgets", () => {
  it("start where the manifest says, and a room the house lacks is the front page", () => {
    const placed = placeWidgets([PLUGIN], {}, ["kitchen"]);
    expect(placed.rooms.kitchen.map((w) => w.widget.id)).toEqual(["now"]);
    expect(placed.rooms.kitchen[0].room).toBe("kitchen");
    expect(placed.front.map((w) => w.widget.id)).toEqual(["chart", "away"]);
    // Drawn for its home room even on the front page.
    expect(placed.front[1].room).toBe("garage");
  });

  it("go where the household moved them, or nowhere", () => {
    const effective = {
      menu: { widgets: { spotprice: { now: "front", chart: "room:kitchen", away: "hidden" } } },
    };
    expect(widgetPlace("spotprice", PLUGIN.widgets[0], effective)).toBe("front");
    const placed = placeWidgets([PLUGIN], effective, ["kitchen"]);
    expect(placed.front.map((w) => w.widget.id)).toEqual(["now"]);
    expect(placed.rooms.kitchen.map((w) => w.widget.id)).toEqual(["chart"]);
  });

  it("leave with their plugin when it is off", () => {
    const placed = placeWidgets([{ ...PLUGIN, enabled: false }], {}, ["kitchen"]);
    expect(placed.front).toEqual([]);
    expect(placed.rooms).toEqual({});
  });

  it("can be put on the front page, any room, or nowhere", () => {
    expect(widgetPlaceOptions(["kitchen"])).toEqual([
      { value: "front", label: "Front page" },
      { value: "room:kitchen", label: "Kitchen" },
      { value: "hidden", label: "Hidden" },
    ]);
  });
});

describe("imageUrl", () => {
  it("sends a URL through Niles and draws a data: image as it is", () => {
    expect(imageUrl("cams", "https://img.example.com/a b.png")).toBe(
      "/plugins/cams/image?src=https%3A%2F%2Fimg.example.com%2Fa%20b.png",
    );
    expect(imageUrl("cams", "data:image/png;base64,AAAA")).toBe("data:image/png;base64,AAAA");
  });
});
