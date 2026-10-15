import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { CONNECT_URL, StravaPanel, outcomeOf, syncText } from "@/components/StravaPanel";
import { api } from "@/lib/api";
import type { StravaStatus } from "@/lib/api";

const NONE: StravaStatus = {
  client_id: null,
  secret_set: false,
  connected: false,
  athlete: null,
  last_sync: null,
  last_error: null,
  syncing: false,
  paused_until: null,
  activities: 0,
  runs: 0,
};

const SAVED: StravaStatus = { ...NONE, client_id: "1001", secret_set: true };

const CONNECTED: StravaStatus = {
  ...SAVED,
  connected: true,
  athlete: "Majse H",
  last_sync: new Date(Date.now() - 10 * 60_000).toISOString(),
  activities: 42,
  runs: 30,
};

function renderCard() {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <StravaPanel />
    </QueryClientProvider>,
  );
}

describe("StravaPanel", () => {
  const writeText = vi.fn();

  beforeEach(() => {
    vi.restoreAllMocks();
    writeText.mockReset().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
  });

  afterEach(() => {
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
  });

  it("walks through making your own Strava app, with this host as the callback domain", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(NONE);
    renderCard();
    expect(await screen.findByText(/strava.com\/settings\/api/)).toBeInTheDocument();
    expect(screen.getByLabelText("Callback domain")).toHaveTextContent(
      window.location.hostname,
    );
    expect(screen.getByText(/everyone makes their own/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Copy callback domain" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(window.location.hostname));
  });

  it("saves the Client ID and Secret, and needs both the first time", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(NONE);
    const save = vi.spyOn(api, "saveStravaApp").mockResolvedValue(SAVED);
    renderCard();

    fireEvent.change(await screen.findByLabelText("Client ID"), {
      target: { value: " 1001 " },
    });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Client Secret"), {
      target: { value: "the-secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0][0]).toEqual({ client_id: "1001", client_secret: "the-secret" });
  });

  it("never shows a saved secret, and keeps it when changing nothing else", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(SAVED);
    const save = vi.spyOn(api, "saveStravaApp").mockResolvedValue(SAVED);
    renderCard();
    expect(await screen.findByText(/secret saved/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Change app" }));
    expect(screen.getByLabelText("Client ID")).toHaveValue("1001");
    expect(screen.getByLabelText("Client Secret")).toHaveValue("");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0][0]).toEqual({ client_id: "1001", client_secret: undefined });
  });

  it("connects by going to Strava", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(SAVED);
    const assign = vi.fn();
    const location = window.location;
    Object.defineProperty(window, "location", {
      value: { ...location, assign },
      configurable: true,
    });
    try {
      renderCard();
      fireEvent.click(await screen.findByRole("button", { name: "Connect Strava" }));
      expect(assign).toHaveBeenCalledWith(CONNECT_URL);
    } finally {
      Object.defineProperty(window, "location", { value: location, configurable: true });
    }
  });

  it("shows who is connected, when it last synced and how much", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(CONNECTED);
    renderCard();
    expect(await screen.findByText("Majse H")).toBeInTheDocument();
    expect(
      screen.getByText("Last synced 10 minutes ago · 42 activities, 30 runs"),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Client Secret")).toBeNull();
  });

  it("syncs now", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(CONNECTED);
    const sync = vi.spyOn(api, "syncStrava").mockResolvedValue({ started: true });
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Sync now" }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
  });

  it("does not offer Sync now while a sync runs", async () => {
    vi.spyOn(api, "strava").mockResolvedValue({ ...CONNECTED, syncing: true });
    renderCard();
    expect(await screen.findByRole("button", { name: "Sync now" })).toBeDisabled();
  });

  it("says when Strava's limits paused the sync", async () => {
    vi.spyOn(api, "strava").mockResolvedValue({
      ...CONNECTED,
      paused_until: new Date(Date.now() + 5 * 60_000).toISOString(),
    });
    renderCard();
    expect(await screen.findByText(/request limit is reached/)).toBeInTheDocument();
  });

  it("asks before disconnecting, then disconnects", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(CONNECTED);
    const disconnect = vi
      .spyOn(api, "disconnectStrava")
      .mockResolvedValue({ revoked_at_strava: true });
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(await screen.findByText(/Strava takes the access back/)).toBeInTheDocument();
    expect(disconnect).not.toHaveBeenCalled();

    fireEvent.click(
      screen
        .getAllByRole("button", { name: "Disconnect" })
        .find((b) => b.closest("[role=dialog]"))!,
    );
    await waitFor(() => expect(disconnect).toHaveBeenCalledTimes(1));
  });

  it("says where to finish when Strava could not be told", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(CONNECTED);
    vi.spyOn(api, "disconnectStrava").mockResolvedValue({ revoked_at_strava: false });
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
    fireEvent.click(
      (await screen.findAllByRole("button", { name: "Disconnect" })).find((b) =>
        b.closest("[role=dialog]"),
      )!,
    );
    expect(await screen.findByText(/strava.com\/settings\/apps/)).toBeInTheDocument();
  });

  it("removes the app after asking", async () => {
    vi.spyOn(api, "strava").mockResolvedValue(SAVED);
    const remove = vi.spyOn(api, "removeStravaApp").mockResolvedValue(undefined);
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Remove app" }));
    fireEvent.click(await screen.findByRole("button", { name: "Remove" }));
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(1));
  });
});

describe("outcomeOf", () => {
  it("reads a connection and a refusal", () => {
    expect(outcomeOf("?strava=connected&name=Majse%20H")).toEqual({
      ok: true,
      text: "Connected as Majse H. Fetching your last half year…",
    });
    expect(outcomeOf("?strava_error=Strava%20was%20not%20given%20access")).toEqual({
      ok: false,
      text: "Strava was not given access",
    });
    expect(outcomeOf("?calendar=connected")).toBeNull();
  });
});

describe("syncText", () => {
  const now = new Date("2026-10-15T12:00:00Z");

  it("counts one of each in the singular", () => {
    expect(
      syncText({ ...CONNECTED, last_sync: "2026-10-14T12:00:00Z", activities: 1, runs: 1 }, now),
    ).toBe("Last synced yesterday · 1 activity, 1 run");
  });

  it("says the backfill is under way before the first sync ends", () => {
    expect(syncText({ ...CONNECTED, last_sync: null, syncing: true }, now)).toBe(
      "Fetching the last half year…",
    );
  });
});
