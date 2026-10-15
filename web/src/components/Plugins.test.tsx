import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { IntegrationsPage } from "@/components/IntegrationsPage";
import { api } from "@/lib/api";
import { pluginStatusOf } from "@/lib/integrations";
import type { PluginPreview, PluginSummary } from "@/lib/api";

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

function setup(plugins: PluginSummary[] = []) {
  const onPluginsChanged = vi.fn();
  const onSecretsChanged = vi.fn();
  render(
    <IntegrationsPage
      integrations={[]}
      plugins={plugins}
      onPluginsChanged={onPluginsChanged}
      onChange={vi.fn()}
      onReset={vi.fn()}
      onSecretsChanged={onSecretsChanged}
      onTadoChanged={vi.fn()}
    />,
  );
  return { onPluginsChanged, onSecretsChanged };
}

afterEach(() => {
  vi.restoreAllMocks();
});

function openAdd() {
  fireEvent.click(screen.getByRole("tab", { name: /^Available/ }));
  fireEvent.click(screen.getByRole("button", { name: "Add from GitHub" }));
}

describe("Add from GitHub", () => {
  it("sits at the end of Available under its own heading", () => {
    setup();
    expect(screen.getByRole("tab", { name: /^Available/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const section = screen.getByRole("region", { name: "From GitHub" });
    expect(within(section).getByRole("button", { name: "Add from GitHub" })).toBeInTheDocument();
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

    fireEvent.click(within(dialog).getByRole("button", { name: "Remove Spot prices" }));
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
