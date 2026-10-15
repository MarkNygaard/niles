import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AddOnsPage } from "@/components/AddOnsPage";
import { api } from "@/lib/api";
import { pluginStatusOf } from "@/lib/integrations";
import type { PluginPreview, PluginSetting, PluginSummary, SecretsReport } from "@/lib/api";

const LOGO =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="currentColor"/></svg>';

const PLUGIN: PluginSummary = {
  id: "spotprice",
  name: "Spot prices",
  version: "0.1.0",
  description: "Hear what power costs hour by hour.",
  authors: ["Jane Doe"],
  homepage: null,
  group: { id: "custom", label: "Custom" },
  status: "running",
  status_line: "Working",
  error: null,
  dropped_events: 0,
  enabled: true,
  repo: "https://github.com/jane/niles-spotprice",
  release_tag: "v0.1.0",
  release_url: "https://github.com/jane/niles-spotprice/releases/tag/v0.1.0",
  installed_at: "2026-10-14T09:00:00Z",
  updated_at: "2026-10-14T09:00:00Z",
  logo: "/plugins/spotprice/logo",
  previous_version: null,
  update: null,
  settings: [],
  menu: [],
  widgets: [],
  interface: "0.1",
  older_interface: false,
};

const PREVIEW: PluginPreview = {
  token: "t0k3n",
  expires_in: 900,
  id: "spotprice",
  name: "Spot prices",
  description: "Hear what power costs hour by hour.",
  version: "0.1.0",
  authors: ["Jane Doe"],
  homepage: null,
  repo: "https://github.com/jane/niles-spotprice",
  tag: "v0.1.0",
  release_url: "https://github.com/jane/niles-spotprice/releases/tag/v0.1.0",
  notes: "",
  group: { id: "custom", label: "Custom" },
  logo: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M1 1h2"/></svg>',
  permissions: [
    "Connect to api.energidataservice.dk",
    "Add devices to your home. They appear in rooms, in rules and by voice.",
    "Adds a page to the menu: Electricity",
  ],
  cannot:
    "It cannot see your conversations, the people in your house, other integrations' passwords or Niles's database.",
  tools: [{ name: "prices", description: "Prices by the hour." }],
  settings: [
    { key: "area", label: "Price area", type: "choice", required: true, help: null },
  ],
  warnings: [],
  sha256: "ab".repeat(32),
  manifest_sha256: "cd".repeat(32),
  current_version: null,
  added: [],
  removed: [],
};

function setup(
  plugins: PluginSummary[] = [],
  more: { rooms?: string[]; secrets?: SecretsReport; effective?: unknown } = {},
) {
  const onPluginsChanged = vi.fn();
  const onSecretsChanged = vi.fn();
  const onChange = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <AddOnsPage
        integrations={[]}
        plugins={plugins}
        onPluginsChanged={onPluginsChanged}
        onChange={onChange}
        onReset={vi.fn()}
        onSecretsChanged={onSecretsChanged}
        onTadoChanged={vi.fn()}
        {...more}
      />
    </QueryClientProvider>,
  );
  return { onPluginsChanged, onSecretsChanged, onChange };
}

beforeEach(() => {
  vi.spyOn(api, "pluginLog").mockResolvedValue([]);
  vi.spyOn(api, "pluginLogo").mockResolvedValue(LOGO);
});

afterEach(() => {
  vi.restoreAllMocks();
});

function openAdd() {
  fireEvent.click(screen.getByRole("tab", { name: /^Available/ }));
  fireEvent.click(screen.getByRole("button", { name: "Add-on from GitHub" }));
}

describe("Add-on from GitHub", () => {
  it("sits at the end of Available under its own heading", () => {
    setup();
    expect(screen.getByRole("tab", { name: /^Available/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const section = screen.getByRole("region", { name: "From GitHub" });
    expect(within(section).getByRole("button", { name: "Add-on from GitHub" })).toBeInTheDocument();
  });

  it("previews the release with its permissions, then installs what it showed", async () => {
    const preview = vi.spyOn(api, "previewPlugin").mockResolvedValue(PREVIEW);
    const install = vi.spyOn(api, "installPlugin").mockResolvedValue(PLUGIN);
    const { onPluginsChanged } = setup();
    openAdd();

    fireEvent.change(screen.getByRole("textbox", { name: "GitHub repository" }), {
      target: { value: " jane/niles-spotprice " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    await screen.findByText("Connect to api.energidataservice.dk");
    expect(preview).toHaveBeenCalledWith("jane/niles-spotprice");

    const permissions = screen.getByRole("region", { name: "Permissions" });
    for (const line of PREVIEW.permissions) {
      expect(within(permissions).getByText(line)).toBeInTheDocument();
    }
    expect(within(permissions).getByText(PREVIEW.cannot)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "github.com/jane/niles-spotprice" })).toHaveAttribute(
      "href",
      "https://github.com/jane/niles-spotprice",
    );
    expect(screen.getByText("By Jane Doe", { exact: false })).toBeInTheDocument();
    expect(screen.getByText("It will ask for: Price area (required).")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() => expect(install).toHaveBeenCalledWith(PREVIEW));
    await waitFor(() => expect(onPluginsChanged).toHaveBeenCalled());
  });

  it("says plainly why a repo cannot be installed", async () => {
    vi.spyOn(api, "previewPlugin").mockRejectedValue(
      new Error("There is no public repository at github.com/jane/nope."),
    );
    const install = vi.spyOn(api, "installPlugin");
    setup();
    openAdd();
    fireEvent.change(screen.getByRole("textbox", { name: "GitHub repository" }), {
      target: { value: "jane/nope" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "There is no public repository at github.com/jane/nope.",
    );
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
    expect(install).not.toHaveBeenCalled();
  });
});

describe("installed plugins on Connected", () => {
  const rows: [PluginSummary, string][] = [
    [PLUGIN, "Working"],
    [
      {
        ...PLUGIN,
        id: "a",
        name: "Updatable",
        update: {
          version: "1.2.0",
          tag: "v1.2.0",
          release_url: "",
          notes: "",
          added: [],
          removed: [],
        },
      },
      "Update available: v1.2.0",
    ],
    [
      {
        ...PLUGIN,
        id: "b",
        name: "Broken",
        status: "failing",
        status_line: "Not working: timed out in on-timer after 10 s",
      },
      "Not working: timed out in on-timer after 10 s",
    ],
    [
      { ...PLUGIN, id: "c", name: "Switched off", status: "disabled", status_line: "Off", enabled: false },
      "Off",
    ],
  ];

  it("show their logo, name and status line", () => {
    setup(rows.map(([plugin]) => plugin));
    expect(screen.getByRole("tab", { name: /^Connected/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    for (const [plugin, line] of rows) {
      const row = screen.getByRole("button", { name: `Configure ${plugin.name}` });
      expect(within(row).getByText(line)).toBeInTheDocument();
    }
    const custom = screen.getByRole("region", { name: "Custom" });
    expect(within(custom).getAllByRole("button")).toHaveLength(rows.length);
  });

  it("open on their source, version and install date, and switch off", async () => {
    const off = vi
      .spyOn(api, "setPluginEnabled")
      .mockResolvedValue({ ...PLUGIN, enabled: false, status: "disabled" });
    const { onPluginsChanged } = setup([PLUGIN]);
    fireEvent.click(screen.getByRole("button", { name: "Configure Spot prices" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("link", { name: "github.com/jane/niles-spotprice" })).toBeInTheDocument();
    expect(within(dialog).getByText("0.1.0")).toBeInTheDocument();
    expect(within(dialog).getByText("Installed")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: /Roll back/ })).toBeNull();

    fireEvent.click(within(dialog).getByRole("button", { name: "Turn off" }));
    await waitFor(() => expect(off).toHaveBeenCalledWith("spotprice", false));
    await waitFor(() => expect(onPluginsChanged).toHaveBeenCalled());
  });

  it("update after showing what changes", async () => {
    const plugin: PluginSummary = {
      ...PLUGIN,
      update: {
        version: "0.2.0",
        tag: "v0.2.0",
        release_url: "",
        notes: "Cheaper hours.",
        added: ["See button presses"],
        removed: [],
      },
    };
    const updatePreview: PluginPreview = {
      ...PREVIEW,
      token: "up",
      version: "0.2.0",
      current_version: "0.1.0",
      added: ["See button presses"],
      notes: "Cheaper hours.",
    };
    vi.spyOn(api, "previewPluginUpdate").mockResolvedValue(updatePreview);
    const update = vi.spyOn(api, "updatePlugin").mockResolvedValue({ ...PLUGIN, version: "0.2.0" });
    setup([plugin]);
    fireEvent.click(screen.getByRole("button", { name: "Configure Spot prices" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Update available: v0.2.0")).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("button", { name: "Update…" }));
    const changes = await within(dialog).findByRole("region", { name: "What changes" });
    expect(within(changes).getByText("See button presses")).toBeInTheDocument();
    expect(within(changes).getByText("From v0.1.0 to v0.2.0.")).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("button", { name: "Update to v0.2.0" }));
    await waitFor(() => expect(update).toHaveBeenCalledWith("spotprice", updatePreview));
  });

  it("roll back, and are removed only once confirmed", async () => {
    const back = vi.spyOn(api, "rollBackPlugin").mockResolvedValue(PLUGIN);
    const remove = vi.spyOn(api, "removePlugin").mockResolvedValue(undefined);
    const { onSecretsChanged } = setup([{ ...PLUGIN, previous_version: "0.0.9" }]);
    fireEvent.click(screen.getByRole("button", { name: "Configure Spot prices" }));
    const dialog = await screen.findByRole("dialog");

    fireEvent.click(within(dialog).getByRole("button", { name: "Roll back to v0.0.9" }));
    await waitFor(() => expect(back).toHaveBeenCalledWith("spotprice"));

    fireEvent.click(within(dialog).getByRole("button", { name: "Remove add-on" }));
    expect(remove).not.toHaveBeenCalled();
    expect(within(dialog).getByText(/Its devices leave your rooms/)).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("spotprice"));
    await waitFor(() => expect(onSecretsChanged).toHaveBeenCalled());
  });
});

describe("pluginStatusOf", () => {
  it("puts not working ahead of an update", () => {
    expect(
      pluginStatusOf({
        ...PLUGIN,
        status: "stopped",
        status_line: "Stopped after repeated failures",
        update: { version: "9.0.0", tag: "v9.0.0", release_url: "", notes: "", added: [], removed: [] },
      }),
    ).toEqual({ ok: false, text: "Stopped after repeated failures" });
    expect(pluginStatusOf({ ...PLUGIN, status: "disabled" })).toEqual({ ok: false, text: "Off" });
    expect(pluginStatusOf(PLUGIN)).toEqual({ ok: true, text: "Working" });
  });
});

const PRESENCE: PluginSummary = {
  ...PLUGIN,
  id: "pres",
  name: "Presence thing",
  group: { id: "presence", label: "Presence & climate" },
};

describe("catalogue placement", () => {
  it("marks every plugin's row Custom, whatever its group", () => {
    setup([PLUGIN, PRESENCE]);
    for (const name of ["Spot prices", "Presence thing"]) {
      const row = screen.getByRole("button", { name: `Configure ${name}` });
      expect(within(row).getByText("Custom")).toBeInTheDocument();
    }
  });

  it("puts a plugin in its manifest's group, and the Custom group last", () => {
    setup([PLUGIN, PRESENCE]);
    const presence = screen.getByRole("region", { name: "Presence & climate" });
    const custom = screen.getByRole("region", { name: "Custom" });
    expect(
      within(presence).getByRole("button", { name: "Configure Presence thing" }),
    ).toBeInTheDocument();
    expect(within(custom).getByRole("button", { name: "Configure Spot prices" })).toBeInTheDocument();
    expect(presence.compareDocumentPosition(custom) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("the logo", () => {
  it("is drawn inline, so currentColor is the text colour", async () => {
    setup([PLUGIN]);
    const row = screen.getByRole("button", { name: "Configure Spot prices" });
    await waitFor(() =>
      expect(row.querySelector('[data-logo="plugin"] svg circle')).not.toBeNull(),
    );
    expect(row.querySelector("circle")).toHaveAttribute("fill", "currentColor");
    expect(row.querySelector("img")).toBeNull();
    expect(api.pluginLogo).toHaveBeenCalledWith("spotprice");
  });

  it("falls back to the generic mark for anything outside the allowlist", async () => {
    vi.mocked(api.pluginLogo).mockResolvedValue(
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><script>alert(1)</script></svg>',
    );
    setup([PLUGIN]);
    const row = screen.getByRole("button", { name: "Configure Spot prices" });
    await waitFor(() => expect(api.pluginLogo).toHaveBeenCalled());
    await waitFor(() => expect(row.querySelector('[data-logo="generic"]')).not.toBeNull());
    expect(row.querySelector("script")).toBeNull();
  });
});

const SETTINGS: PluginSetting[] = [
  {
    key: "label",
    label: "Label",
    type: "string",
    required: false,
    help: "What the page is called.",
    default: "Prices",
    options: [],
    min: null,
    max: null,
    value: null,
    set: false,
  },
  {
    key: "cheap_hours",
    label: "Cheap hours",
    type: "number",
    required: false,
    help: null,
    default: 6,
    options: [],
    min: 1,
    max: 24,
    value: 4,
    set: true,
  },
  {
    key: "speak",
    label: "Say when it is cheap",
    type: "boolean",
    required: false,
    help: null,
    default: false,
    options: [],
    min: null,
    max: null,
    value: null,
    set: false,
  },
  {
    key: "area",
    label: "Price area",
    type: "choice",
    required: true,
    help: "West of the Great Belt is DK1.",
    default: "DK1",
    options: [
      { value: "DK1", label: "West Denmark" },
      { value: "DK2", label: "East Denmark" },
    ],
    min: null,
    max: null,
    value: null,
    set: false,
  },
  {
    key: "room",
    label: "Room",
    type: "room",
    required: false,
    help: null,
    default: null,
    options: [],
    min: null,
    max: null,
    value: "kitchen",
    set: true,
  },
  {
    key: "api_key",
    label: "API key",
    type: "secret",
    required: true,
    help: "From your account page.",
    default: null,
    options: [],
    min: null,
    max: null,
    value: null,
    set: false,
  },
];

async function openPanel(plugin: PluginSummary, more: Parameters<typeof setup>[1] = {}) {
  const handles = setup([plugin], more);
  fireEvent.click(screen.getByRole("button", { name: `Configure ${plugin.name}` }));
  const dialog = await screen.findByRole("dialog");
  return { ...handles, dialog };
}

const UNSET: PluginSummary = {
  ...PLUGIN,
  status: "not_set_up",
  status_line: "Not set up: API key is required",
  settings: SETTINGS,
};

describe("the generated setup panel", () => {
  it("draws a field for each type, with required marks, help and defaults", async () => {
    const { dialog } = await openPanel(UNSET, { rooms: ["kitchen", "office"] });
    const settings = within(dialog).getByRole("region", { name: "Settings" });
    expect(within(settings).getByRole("textbox", { name: "Label" })).toHaveAttribute(
      "placeholder",
      "Prices",
    );
    expect(
      within(settings).getByText("What the page is called. Default: Prices."),
    ).toBeInTheDocument();
    expect(within(settings).getByRole("textbox", { name: "Cheap hours" })).toHaveValue("4");
    expect(within(settings).getByRole("switch", { name: "Say when it is cheap" })).not.toBeChecked();
    expect(within(settings).getByRole("combobox", { name: "Price area" })).toBeInTheDocument();
    expect(
      within(settings).getByText("West of the Great Belt is DK1. Default: West Denmark."),
    ).toBeInTheDocument();
    expect(within(settings).getByRole("combobox", { name: "Room" })).toHaveTextContent("Kitchen");
    // A secret is its own write-only field.
    expect(within(settings).getByLabelText("API key (required)")).toHaveAttribute(
      "type",
      "password",
    );
    expect(within(settings).getAllByText("required")).toHaveLength(1);
  });

  it("says what is missing before it starts", async () => {
    const { dialog } = await openPanel(UNSET);
    expect(within(dialog).getByText("Not set up: needs API key")).toBeInTheDocument();
  });

  it("says how it is doing once nothing is missing", async () => {
    const { dialog } = await openPanel({
      ...UNSET,
      status: "running",
      status_line: "Working",
      settings: SETTINGS.map((s) => (s.key === "api_key" ? { ...s, set: true } : s)),
    });
    expect(within(dialog).getByText("Working")).toBeInTheDocument();
  });

  it("saves what changed through PATCH /config, and an emptied field as its default", async () => {
    const patch = vi.spyOn(api, "patchConfig").mockResolvedValue({} as never);
    const { dialog, onPluginsChanged } = await openPanel(UNSET);
    const settings = within(dialog).getByRole("region", { name: "Settings" });
    fireEvent.change(within(settings).getByRole("textbox", { name: "Label" }), {
      target: { value: "Strøm" },
    });
    fireEvent.change(within(settings).getByRole("textbox", { name: "Cheap hours" }), {
      target: { value: "" },
    });
    fireEvent.click(within(settings).getByRole("switch", { name: "Say when it is cheap" }));
    fireEvent.click(within(settings).getAllByRole("button", { name: "Save" })[0]);
    await waitFor(() =>
      expect(patch).toHaveBeenCalledWith({
        plugins: { spotprice: { label: "Strøm", cheap_hours: null, speak: true } },
      }),
    );
    await waitFor(() => expect(onPluginsChanged).toHaveBeenCalled());
  });

  it("refuses a number outside its range without saving", async () => {
    const patch = vi.spyOn(api, "patchConfig");
    const { dialog } = await openPanel(UNSET);
    const settings = within(dialog).getByRole("region", { name: "Settings" });
    fireEvent.change(within(settings).getByRole("textbox", { name: "Cheap hours" }), {
      target: { value: "30" },
    });
    fireEvent.click(within(settings).getAllByRole("button", { name: "Save" })[0]);
    expect(
      await within(settings).findByText("Cheap hours is between 1 and 24."),
    ).toBeInTheDocument();
    expect(patch).not.toHaveBeenCalled();
  });

  it("writes a secret under plugins.<id>.<key>", async () => {
    const put = vi.spyOn(api, "setSecret").mockResolvedValue(undefined);
    const { dialog, onSecretsChanged } = await openPanel(UNSET, {
      secrets: {
        writable: true,
        secrets: [
          { key: "plugins.spotprice.api_key", label: "Spot prices: API key", source: "unset" },
        ],
      },
    });
    const input = within(dialog).getByLabelText("API key (required)");
    fireEvent.change(input, { target: { value: "hunter2" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(put).toHaveBeenCalledWith("plugins.spotprice.api_key", "hunter2"));
    await waitFor(() => expect(onSecretsChanged).toHaveBeenCalled());
  });
});

describe("the panel", () => {
  it("shows the log, newest first, with levels and times", async () => {
    vi.mocked(api.pluginLog).mockResolvedValue([
      { at: "2026-10-15T08:00:00Z", level: "info", message: "Working", niles: true },
      { at: "2026-10-15T08:00:05Z", level: "warn", message: "prices are late", niles: false },
    ]);
    const { dialog } = await openPanel(PLUGIN);
    const log = within(dialog).getByRole("region", { name: "Log" });
    const lines = await within(log).findAllByRole("listitem");
    expect(lines[0]).toHaveTextContent("prices are late");
    expect(lines[0]).toHaveAttribute("data-level", "warn");
    expect(lines[0]).toHaveTextContent("warn");
    expect(lines[1]).toHaveTextContent("Niles: Working");
    expect(lines[1].querySelector("time")).toHaveAttribute("datetime", "2026-10-15T08:00:00Z");
    expect(lines[1].querySelector("time")?.textContent).toMatch(/^\d\d\D\d\d\D\d\d$/);
    expect(api.pluginLog).toHaveBeenCalledWith("spotprice");
  });

  it("notes a plugin built for an older Niles, and keeps a rollback notice", async () => {
    const { dialog } = await openPanel({
      ...PLUGIN,
      older_interface: true,
      error:
        "Version 0.2.0 failed soon after the update (trapped in on-timer), so 0.1.0 was put back.",
    });
    expect(within(dialog).getByText(/Built for an older Niles/)).toBeInTheDocument();
    expect(within(dialog).getByText(/0.1.0 was put back/)).toBeInTheDocument();
  });

  it("shows where each widget goes", async () => {
    const plugin: PluginSummary = {
      ...PLUGIN,
      widgets: [{ id: "price_now", title: "Electricity now", room: "kitchen", refresh_every: null }],
    };
    const { dialog } = await openPanel(plugin, { rooms: ["kitchen"] });
    const widgets = within(dialog).getByRole("region", { name: "Widgets" });
    expect(
      within(widgets).getByRole("combobox", { name: "Where Electricity now goes" }),
    ).toHaveTextContent("Kitchen");
  });
});
