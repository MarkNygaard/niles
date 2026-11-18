import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ChatView } from "@/components/ChatView";
import type { ChatViewProps } from "@/components/ChatView";
import type { LiveView } from "@/lib/api";

/** A socket that opens at once, so a view connects without a server. */
class FakeSocket {
  static OPEN = 1;
  static made: FakeSocket[] = [];
  readyState = 1;
  binaryType = "blob";
  onmessage: ((m: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(public url: string) {
    FakeSocket.made.push(this);
  }
  send() {}
  close() {}
}

const ZALANDO: LiveView = {
  address: "https://www.zalando.dk/herretoej-jeans/",
  control: "niles",
  handoff: null,
  width: 1280,
  height: 900,
};

function renderChat(props: Partial<ChatViewProps> = {}) {
  const handlers = { onSend: vi.fn(), onNew: vi.fn(), onOpenPanel: vi.fn(), onAnswer: vi.fn() };
  render(<ChatView turns={[]} {...handlers} {...props} />);
}

beforeEach(() => {
  FakeSocket.made = [];
  vi.stubGlobal("WebSocket", FakeSocket);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("the browser in the chat", () => {
  it("shows Niles's page live in the bubble being waited for, and opens it", async () => {
    const onOpenBrowser = vi.fn();
    renderChat({ pending: "find me some black jeans", browser: ZALANDO, onOpenBrowser });
    expect(screen.getByLabelText("Niles is answering")).toBeInTheDocument();
    const glimpse = screen.getByRole("button", { name: "Niles is on zalando.dk. Open the browser" });
    await waitFor(() => expect(FakeSocket.made).toHaveLength(1));
    expect(FakeSocket.made[0].url).toMatch(/\/me\/browser\/live$/);
    fireEvent.click(glimpse);
    expect(onOpenBrowser).toHaveBeenCalled();
    expect(screen.queryByText(/browser is open/)).toBeNull();
  });

  it("waits without a picture when Niles has no browser open", () => {
    renderChat({ pending: "is anyone home?", browser: null });
    expect(screen.getByLabelText("Niles is answering")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Open the browser/ })).toBeNull();
    expect(FakeSocket.made).toHaveLength(0);
  });

  it("keeps a way back to the open browser above the message field", () => {
    const onOpenBrowser = vi.fn();
    renderChat({ browser: ZALANDO, onOpenBrowser });
    const strip = screen.getByRole("button", { name: /Niles's browser is open · zalando\.dk/ });
    fireEvent.click(strip);
    expect(onOpenBrowser).toHaveBeenCalled();
    expect(FakeSocket.made).toHaveLength(0);
  });

  it("steps aside while the Browser tab shows the page, and streams nothing", async () => {
    const onCloseBrowser = vi.fn();
    const onOpenBrowser = vi.fn();
    renderChat({
      pending: "find me some black jeans",
      browser: ZALANDO,
      browserBeside: true,
      onCloseBrowser,
      onOpenBrowser,
    });
    const aside = screen.getByRole("group", {
      name: "The browser on zalando.dk is open beside the chat",
    });
    expect(aside).toHaveTextContent("Open beside the chat");
    expect(screen.queryByRole("button", { name: /Open the browser/ })).toBeNull();
    expect(FakeSocket.made).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onCloseBrowser).toHaveBeenCalled();
    expect(onOpenBrowser).not.toHaveBeenCalled();
  });

  it("comes back live when the Browser tab closes", async () => {
    const handlers = { onSend: vi.fn(), onNew: vi.fn(), onOpenPanel: vi.fn(), onAnswer: vi.fn() };
    const { rerender } = render(
      <ChatView turns={[]} {...handlers} pending="jeans" browser={ZALANDO} browserBeside />,
    );
    expect(FakeSocket.made).toHaveLength(0);
    rerender(<ChatView turns={[]} {...handlers} pending="jeans" browser={ZALANDO} />);
    expect(
      screen.getByRole("button", { name: "Niles is on zalando.dk. Open the browser" }),
    ).toBeInTheDocument();
    await waitFor(() => expect(FakeSocket.made).toHaveLength(1));
  });

  it("turns the strip into a way to close the tab while it shows the page", () => {
    const onCloseBrowser = vi.fn();
    const onOpenBrowser = vi.fn();
    renderChat({ browser: ZALANDO, browserBeside: true, onCloseBrowser, onOpenBrowser });
    fireEvent.click(screen.getByRole("button", { name: /Showing beside the chat · zalando\.dk/ }));
    expect(onCloseBrowser).toHaveBeenCalled();
    expect(onOpenBrowser).not.toHaveBeenCalled();
  });

  it("says when the person has the browser", () => {
    renderChat({ browser: { ...ZALANDO, control: "person" } });
    expect(
      screen.getByRole("button", { name: /You have Niles's browser · zalando\.dk/ }),
    ).toBeInTheDocument();
  });
});
