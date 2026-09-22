import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ConfigPanel, satellitesAt } from "@/components/ConfigPanel";
import { api } from "@/lib/api";

/**
 * The panel owns which queries go stale after a write, and nothing
 * smaller can tell you it got that wrong: every card below it was
 * correct on its own while the page as a whole showed a switch that
 * would not move.
 */
function stubApi() {
  vi.spyOn(api, "getConfig").mockResolvedValue({
    effective: { presence: { enabled: false } },
    overrides: {},
    sections: [],
    persistent: true,
  });
  vi.spyOn(api, "history").mockResolvedValue([]);
  vi.spyOn(api, "devices").mockResolvedValue([]);
  vi.spyOn(api, "secrets").mockResolvedValue({ writable: true, secrets: [] });
  vi.spyOn(api, "setup").mockResolvedValue({ set_up: true, gaps: [] });
  vi.spyOn(api, "timezones").mockResolvedValue(["UTC"]);
  vi.spyOn(api, "integrations").mockResolvedValue([
    {
      id: "tado",
      label: "tado°",
      blurb: "Who is home.",
      kind: "service",
      base_url: null,
      serves: [],
      added: true,
      secret_key: null,
    },
  ]);
  vi.spyOn(api, "tadoStatus").mockResolvedValue({
    connectable: true,
    authorised: true,
    presence_enabled: false,
  });
  return vi.spyOn(api, "patchConfig").mockResolvedValue({
    revision: 1,
    noop: false,
    changes: [],
    summary: "presence.enabled → true",
    needs_restart: [],
  });
}

function renderPanel() {
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <ConfigPanel />
    </QueryClientProvider>,
  );
}

describe("ConfigPanel", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("re-asks what the server works out from the config, not only the config", async () => {
    // The bug: a write invalidated `config` and `history` and nothing
    // else, so the tado card — which reads /presence/tado — kept showing
    // the old answer while presence really did turn on behind it.
    const patch = stubApi();
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: /Integrations/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Configure/ }));
    const before = vi.mocked(api.tadoStatus).mock.calls.length;

    fireEvent.click(await screen.findByRole("switch"));

    await waitFor(() => expect(patch).toHaveBeenCalled());
    await waitFor(() =>
      expect(vi.mocked(api.tadoStatus).mock.calls.length).toBeGreaterThan(
        before,
      ),
    );
  });

  it("re-asks the credentials too, since a key's source depends on the config", async () => {
    stubApi();
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: /Integrations/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Configure/ }));
    const before = vi.mocked(api.secrets).mock.calls.length;

    fireEvent.click(await screen.findByRole("switch"));

    await waitFor(() =>
      expect(vi.mocked(api.secrets).mock.calls.length).toBeGreaterThan(before),
    );
  });
});

describe("satellitesAt", () => {
  it("reads the volume back", () => {
    // The bug this pins: the slider wrote `satellites.<name>.volume`
    // and this never read it, so every reload showed the shipped 100
    // and the setting looked as though it had not saved.
    const got = satellitesAt({
      satellites: {
        kitchen: { ip: "192.168.42.30", room: "kitchen", volume: 40 },
      },
    });
    expect(got).toEqual([
      { name: "kitchen", ip: "192.168.42.30", room: "kitchen", volume: 40 },
    ]);
  });

  it("leaves volume unset for an entry that has none", () => {
    // Undefined rather than 100: the card decides what an unset volume
    // looks like, and deciding it twice is how the two come to
    // disagree.
    const got = satellitesAt({
      satellites: { kitchen: { ip: "192.168.42.30", room: "kitchen" } },
    });
    expect(got[0].volume).toBeUndefined();
  });

  it("survives a config with no satellites at all", () => {
    expect(satellitesAt({})).toEqual([]);
    expect(satellitesAt(null)).toEqual([]);
  });
});
