import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { LiveBrowser } from "@/components/LiveBrowser";
import { HandoffCard } from "@/components/HandoffCard";
import { api } from "@/lib/api";
import type { LiveView } from "@/lib/api";

/** A socket that opens at once and keeps what is sent on it. */
class FakeSocket {
  static OPEN = 1;
  static made: FakeSocket[] = [];
  readyState = 1;
  binaryType = "blob";
  sent: string[] = [];
  onmessage: ((m: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(public url: string) {
    FakeSocket.made.push(this);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {}
}

const NILES: LiveView = {
  address: "https://shop.example/basket",
  control: "niles",
  handoff: null,
  width: 1280,
  height: 900,
};

const YOURS: LiveView = {
  ...NILES,
  control: "person",
  handoff: {
    id: "h1",
    site: "https://shop.example",
    why: "The site wants a person to show they aren't a robot.",
  },
};

function renderBrowser(onGiveBack = vi.fn()) {
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <LiveBrowser onGiveBack={onGiveBack} givingBack={false} />
    </QueryClientProvider>,
  );
  return onGiveBack;
}

beforeEach(() => {
  FakeSocket.made = [];
  vi.stubGlobal("WebSocket", FakeSocket);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("LiveBrowser", () => {
  it("says when nothing is open, and opens no socket", async () => {
    vi.spyOn(api, "browser").mockResolvedValue(null);
    renderBrowser();
    expect(await screen.findByText(/Nothing is open in Niles's browser/)).toBeInTheDocument();
    expect(FakeSocket.made).toHaveLength(0);
  });

  it("shows Niles at work and lets the person take over", async () => {
    vi.spyOn(api, "browser").mockResolvedValue(NILES);
    const take = vi.spyOn(api, "takeBrowser").mockResolvedValue(undefined);
    renderBrowser();
    expect(await screen.findByText("Niles is using the browser.")).toBeInTheDocument();
    expect(screen.getByText(NILES.address)).toBeInTheDocument();
    await waitFor(() => expect(FakeSocket.made).toHaveLength(1));
    expect(FakeSocket.made[0].url).toMatch(/\/me\/browser\/live$/);
    expect(screen.queryByRole("button", { name: "Give back to Niles" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Take over/ }));
    await waitFor(() => expect(take).toHaveBeenCalled());
  });

  it("sends nothing from the page while Niles has it", async () => {
    vi.spyOn(api, "browser").mockResolvedValue(NILES);
    renderBrowser();
    const page = await screen.findByLabelText("The page Niles is using");
    fireEvent.keyDown(page, { key: "a" });
    expect(FakeSocket.made[0]?.sent ?? []).toEqual([]);
  });

  it("gives the browser back, to carry on or to stop", async () => {
    vi.spyOn(api, "browser").mockResolvedValue(YOURS);
    const onGiveBack = renderBrowser();
    expect(await screen.findByText(YOURS.handoff!.why)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Give back to Niles" }));
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onGiveBack.mock.calls).toEqual([["carry_on"], ["stop"]]);
  });

  it("sends what the person types, keys and text both", async () => {
    vi.spyOn(api, "browser").mockResolvedValue(YOURS);
    renderBrowser();
    const page = await screen.findByLabelText("The page, yours to use");
    await waitFor(() => expect(FakeSocket.made).toHaveLength(1));
    fireEvent.keyDown(page, { key: "a", shiftKey: true });
    fireEvent.keyDown(page, { key: "Shift" });
    fireEvent.change(screen.getByLabelText("Type into the page"), {
      target: { value: "hello" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    fireEvent.click(screen.getByRole("button", { name: "Enter" }));
    const sent = FakeSocket.made[0].sent.map((s) => JSON.parse(s));
    expect(sent).toEqual([
      { type: "key", press: "down", key: "a", modifiers: 8 },
      { type: "text", text: "hello" },
      { type: "key", press: "down", key: "Enter", modifiers: 0 },
      { type: "key", press: "up", key: "Enter", modifiers: 0 },
    ]);
  });
});

describe("HandoffCard", () => {
  const handoff = YOURS.handoff!;

  it("opens the browser while it is still the person's", () => {
    const onOpen = vi.fn();
    render(<HandoffCard handoff={handoff} waiting onOpen={onOpen} />);
    expect(screen.getByText(handoff.why)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open the browser" }));
    expect(onOpen).toHaveBeenCalled();
  });

  it("says when the browser went back to Niles", () => {
    render(<HandoffCard handoff={handoff} waiting={false} />);
    expect(screen.getByText("The browser went back to Niles.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open the browser" })).toBeNull();
  });
});
