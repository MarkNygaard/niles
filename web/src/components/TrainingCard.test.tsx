import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TrainingCard, ingestUrl, receivedText } from "@/components/TrainingCard";
import { api } from "@/lib/api";
import type { TrainingOverview } from "@/lib/api";

const KEY = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const NONE: TrainingOverview = {
  key_made_at: null,
  last_received_at: null,
  runs: 0,
  activities: 0,
  wellness_days: 0,
};

const IN_USE: TrainingOverview = {
  ...NONE,
  key_made_at: "2026-10-01T08:00:00Z",
  last_received_at: new Date(Date.now() - 10 * 60_000).toISOString(),
  runs: 42,
  activities: 50,
  wellness_days: 30,
};

function renderCard() {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <TrainingCard />
    </QueryClientProvider>,
  );
}

describe("TrainingCard", () => {
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

  it("gives the address to put in Health Auto Export", async () => {
    vi.spyOn(api, "training").mockResolvedValue(NONE);
    renderCard();
    expect(await screen.findByLabelText("URL")).toHaveTextContent(
      `${window.location.origin}/training/ingest`,
    );
    expect(screen.getByText(/Connected Apps → Apple Health/)).toBeInTheDocument();
  });

  it("makes a key, shows it once and copies it", async () => {
    vi.spyOn(api, "training").mockResolvedValue(NONE);
    const make = vi.spyOn(api, "makeTrainingKey").mockResolvedValue({ key: KEY });
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "Create key" }));
    await waitFor(() => expect(make).toHaveBeenCalledTimes(1));
    expect(await screen.findByLabelText("Your key")).toHaveTextContent(KEY);
    expect(screen.getByText(/Shown this once/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Copy your key" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(KEY));
    expect(await screen.findByText("Copied")).toBeInTheDocument();
  });

  it("never shows a key that is already in use", async () => {
    vi.spyOn(api, "training").mockResolvedValue(IN_USE);
    renderCard();
    expect(await screen.findByText(/A key is in use/)).toBeInTheDocument();
    expect(screen.queryByLabelText("Your key")).toBeNull();
  });

  it("says when data last arrived and how much", async () => {
    vi.spyOn(api, "training").mockResolvedValue(IN_USE);
    renderCard();
    expect(
      await screen.findByText(
        "Last data 10 minutes ago · 42 runs · 30 days of sleep, heart and weight",
      ),
    ).toBeInTheDocument();
  });

  it("asks before revoking, then revokes", async () => {
    vi.spyOn(api, "training").mockResolvedValue(IN_USE);
    const revoke = vi.spyOn(api, "revokeTrainingKey").mockResolvedValue(undefined);
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "Revoke" }));
    expect(await screen.findByText(/stops sending to Niles/)).toBeInTheDocument();
    expect(revoke).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getAllByRole("button", { name: "Revoke" }).find((b) => b.closest("[role=dialog]"))!,
    );
    await waitFor(() => expect(revoke).toHaveBeenCalledTimes(1));
  });

  it("replaces the key and shows the new one", async () => {
    vi.spyOn(api, "training").mockResolvedValue(IN_USE);
    const make = vi.spyOn(api, "makeTrainingKey").mockResolvedValue({ key: KEY });
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "Replace" }));
    fireEvent.click(await screen.findByRole("button", { name: "Make a new key" }));
    await waitFor(() => expect(make).toHaveBeenCalledTimes(1));
    expect(await screen.findByLabelText("Your key")).toHaveTextContent(KEY);
  });
});

describe("receivedText", () => {
  const now = new Date("2026-10-15T12:00:00Z");

  it("is empty with no key and nothing sent", () => {
    expect(receivedText(NONE, now)).toBe("");
  });

  it("waits for the first post once a key exists", () => {
    expect(receivedText({ ...NONE, key_made_at: "2026-10-15T11:00:00Z" }, now)).toBe(
      "Nothing has arrived yet.",
    );
  });

  it("counts one of each in the singular", () => {
    const one = { ...NONE, last_received_at: "2026-10-14T12:00:00Z", runs: 1, wellness_days: 1 };
    expect(receivedText(one, now)).toBe(
      "Last data yesterday · 1 run · 1 day of sleep, heart and weight",
    );
  });
});

describe("ingestUrl", () => {
  it("is this Niles's address", () => {
    expect(ingestUrl("https://niles.example")).toBe("https://niles.example/training/ingest");
  });
});
