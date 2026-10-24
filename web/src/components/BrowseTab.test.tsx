/**
 * Browse: Listed first, Unreviewed folded, removed hidden; Install is
 * the same preview as a pasted address; the panel's fallback when the
 * library can't be loaded.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AddOnsPage } from "@/components/AddOnsPage";
import { api } from "@/lib/api";
import type { LibraryEntry, PluginLibrary, PluginPreview, PluginSummary } from "@/lib/api";

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
  addon: null,
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
  inbound: null,
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
  logo: LOGO,
  permissions: ["Connect to api.energidataservice.dk"],
  cannot:
    "It cannot see your conversations, the people in your house, other add-ons' passwords or Niles's database.",
  tools: [],
  settings: [],
  warnings: [],
  sha256: "ab".repeat(32),
  manifest_sha256: "cd".repeat(32),
  current_version: null,
  added: [],
  removed: [],
};

let seq = 0;

/** One library entry, with sane defaults and `over` on top. */
function entry(over: Partial<LibraryEntry> = {}): LibraryEntry {
  seq += 1;
  const repo = over.repo ?? `https://github.com/jane/add-on-${seq}`;
  const addon = over.addon ?? null;
  const path = repo.replace("https://github.com/", "");
  return {
    repo,
    addon,
    address: addon ? `${path}/${addon}` : path,
    releases: `${repo}/releases`,
    shares_repo: false,
    owner: "jane",
    standing: "listed",
    removed_reason: null,
    name: `Add-on ${seq}`,
    description: "Does a thing.",
    version: "0.1.0",
    published_at: null,
    installs: 1,
    downloads: 10,
    stars: 1,
    logo: null,
    readme: null,
    permissions: [],
    min_niles: "0.2.0",
    needs_newer_niles: false,
    blocked: null,
    ...over,
  };
}

const SPOTPRICE = entry({
  repo: "https://github.com/jane/niles-spotprice",
  name: "Spot prices",
  description: "Hear what power costs hour by hour.",
  installs: 34,
  stars: 12,
  permissions: ["Connect to api.energidataservice.dk"],
});

const POOL = entry({
  repo: "https://github.com/bob/niles-pool",
  owner: "bob",
  name: "Pool pump",
  standing: "unreviewed",
  installs: null,
  downloads: 120,
});

const FUTURE = entry({
  repo: "https://github.com/eve/niles-future",
  owner: "eve",
  name: "Future thing",
  needs_newer_niles: true,
  blocked: "It needs Niles 9.0.0 or newer, and this is Niles 0.4.0.",
});

const GONE = entry({
  repo: "https://github.com/mallory/niles-gone",
  owner: "mallory",
  name: "Gone",
  standing: "removed",
  removed_reason: "Its author archived it.",
});

function library(...addons: LibraryEntry[]): PluginLibrary {
  return { generated_at: null, cannot: PREVIEW.cannot, addons };
}

function setup(plugins: PluginSummary[] = []) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <AddOnsPage
        integrations={[]}
        plugins={plugins}
        onChange={vi.fn()}
        onReset={vi.fn()}
        onSecretsChanged={vi.fn()}
        onTadoChanged={vi.fn()}
      />
    </QueryClientProvider>,
  );
}

function browse() {
  fireEvent.click(screen.getByRole("tab", { name: "Browse" }));
}

beforeEach(() => {
  vi.spyOn(api, "pluginLog").mockResolvedValue([]);
  vi.spyOn(api, "pluginLogo").mockResolvedValue(LOGO);
  vi.spyOn(api, "pluginLibrary").mockResolvedValue(
    library(SPOTPRICE, POOL, FUTURE, GONE),
  );
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("Browse", () => {
  it("lists Listed add-ons first, keeps Unreviewed folded until asked, and leaves out removed ones", async () => {
    setup();
    browse();
    const listed = await screen.findByRole("region", { name: "Listed" });
    expect(within(listed).getByText("Spot prices")).toBeInTheDocument();
    expect(within(listed).getByText("34 installs")).toBeInTheDocument();
    expect(screen.queryByText("Pool pump")).toBeNull();
    expect(screen.queryByText("Gone")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Unreviewed \(1\)/ }));
    const unreviewed = await screen.findByRole("region", { name: "Unreviewed" });
    expect(within(unreviewed).getByText("Pool pump")).toBeInTheDocument();
    expect(within(unreviewed).getByText("120 downloads")).toBeInTheDocument();
    expect(
      screen.getByText("Tagged niles-addon by their authors on GitHub. Nobody has checked them."),
    ).toBeInTheDocument();
  });

  it("marks an add-on already installed, whatever the case of its address", async () => {
    setup([{ ...PLUGIN, repo: "https://github.com/Jane/Niles-Spotprice" }]);
    browse();
    const listed = await screen.findByRole("region", { name: "Listed" });
    expect(within(listed).getByText("Installed")).toBeInTheDocument();
    expect(
      within(listed).queryByRole("button", { name: "Install Spot prices" }),
    ).toBeNull();
  });

  it("won't install an add-on for a newer Niles, and says why", async () => {
    setup();
    browse();
    const listed = await screen.findByRole("region", { name: "Listed" });
    expect(within(listed).getByText("Needs a newer Niles")).toBeInTheDocument();
    expect(
      within(listed).queryByRole("button", { name: "Install Future thing" }),
    ).toBeNull();

    fireEvent.click(within(listed).getByRole("button", { name: "Future thing" }));
    expect(await screen.findByText(/It needs Niles 9\.0\.0/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install" })).toBeDisabled();
  });

  it("installs through the same preview as a pasted address", async () => {
    const preview = vi.spyOn(api, "previewPlugin").mockResolvedValue(PREVIEW);
    const install = vi.spyOn(api, "installPlugin").mockResolvedValue(PLUGIN);
    setup();
    browse();
    fireEvent.click(
      await screen.findByRole("button", { name: "Install Spot prices" }),
    );
    expect(
      await screen.findByText("Connect to api.energidataservice.dk"),
    ).toBeInTheDocument();
    expect(preview).toHaveBeenCalledTimes(1);
    expect(preview).toHaveBeenCalledWith("jane/niles-spotprice");

    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    expect(install).toHaveBeenCalledWith(PREVIEW);
  });

  it("shows an add-on's permissions in the approval screen's words", async () => {
    setup();
    browse();
    const listed = await screen.findByRole("region", { name: "Listed" });
    fireEvent.click(within(listed).getByRole("button", { name: "Spot prices" }));
    const dialog = await screen.findByRole("dialog");
    const permissions = within(dialog).getByRole("region", { name: "Permissions" });
    expect(
      within(permissions).getByText("Connect to api.energidataservice.dk"),
    ).toBeInTheDocument();
    expect(within(permissions).getByText(PREVIEW.cannot)).toBeInTheDocument();
  });

  it("says when the library can't be loaded, and adding by address still works", async () => {
    vi.mocked(api.pluginLibrary).mockRejectedValue(
      new Error("The add-on library could not be reached: offline"),
    );
    setup();
    browse();
    expect(await screen.findByText("Couldn't load the add-on library.")).toBeInTheDocument();
    expect(screen.getByText("The add-on library could not be reached: offline")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Add-on from GitHub" }));
    expect(await screen.findByRole("textbox", { name: "GitHub repository" })).toBeInTheDocument();
  });

  it("lists each add-on of a repo on its own, installs it by owner/repo/name, and says its stars are the repo's", async () => {
    const shared = { repo: "https://github.com/jane/niles-addons", stars: 9, shares_repo: true };
    const alpha = entry({ ...shared, addon: "alpha", name: "Alpha", installs: 3 });
    const bravo = entry({ ...shared, addon: "bravo", name: "Bravo", installs: 5 });
    vi.mocked(api.pluginLibrary).mockResolvedValue(library(alpha, bravo));
    const preview = vi.spyOn(api, "previewPlugin").mockResolvedValue(PREVIEW);
    setup([
      { ...PLUGIN, id: "bravo", repo: "https://github.com/jane/niles-addons", addon: "bravo" },
    ]);
    browse();
    const listed = await screen.findByRole("region", { name: "Listed" });
    expect(within(listed).getByText("3 installs")).toBeInTheDocument();
    expect(within(listed).getByText("5 installs")).toBeInTheDocument();
    expect(within(listed).getAllByText("9 stars on its repo")).toHaveLength(2);
    expect(within(listed).getAllByText("Installed")).toHaveLength(1);
    expect(within(listed).queryByRole("button", { name: "Install Bravo" })).toBeNull();

    fireEvent.click(within(listed).getByRole("button", { name: "Install Alpha" }));
    expect(await screen.findByText("Connect to api.energidataservice.dk")).toBeInTheDocument();
    expect(preview).toHaveBeenCalledWith("jane/niles-addons/alpha");
  });

  it("narrows the library with the page's search", async () => {
    setup();
    browse();
    const listed = await screen.findByRole("region", { name: "Listed" });
    fireEvent.change(screen.getByRole("searchbox", { name: "Search add-ons" }), {
      target: { value: "pool" },
    });
    expect(within(listed).queryByText("Spot prices")).toBeNull();
    expect(screen.getByRole("button", { name: /Unreviewed \(1\)/ })).toBeInTheDocument();
  });
});
