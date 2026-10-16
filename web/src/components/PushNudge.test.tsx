import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ChatView } from "@/components/ChatView";
import { PushNudge } from "@/components/PushNudge";
import type { Push } from "@/hooks/usePush";
import type { ArtifactRef, Turn } from "@/lib/api";

const push: { current: Push } = { current: undefined as unknown as Push };
vi.mock("@/hooks/usePush", () => ({ usePush: () => push.current }));

function device(rest: Partial<Push> = {}): Push {
  return {
    env: {
      userAgent: "",
      maxTouchPoints: 0,
      standalone: false,
      hasPush: true,
      permission: "default",
    },
    support: "available",
    serverReady: true,
    on: false,
    here: null,
    devices: [],
    canAsk: true,
    busy: false,
    enable: vi.fn(),
    disable: vi.fn(),
    remove: vi.fn(),
    test: vi.fn(),
    ...rest,
  };
}

let store: Record<string, string> = {};
beforeEach(() => {
  store = {};
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store[k] ?? null,
    setItem: (k: string, v: string) => {
      store[k] = v;
    },
  });
});
afterEach(() => vi.unstubAllGlobals());

describe("PushNudge", () => {
  it("turns notifications on in one tap", () => {
    push.current = device();
    render(<PushNudge />);
    fireEvent.click(screen.getByRole("button", { name: "Notify me when it's done" }));
    expect(push.current.enable).toHaveBeenCalled();
  });

  it("is not offered again once dismissed", () => {
    push.current = device();
    const { unmount } = render(<PushNudge />);
    fireEvent.click(screen.getByRole("button", { name: "Don't offer notifications" }));
    expect(screen.queryByRole("button", { name: "Notify me when it's done" })).toBeNull();
    unmount();
    render(<PushNudge />);
    expect(screen.queryByRole("button", { name: "Notify me when it's done" })).toBeNull();
  });

  it("is not offered where notifications are on, or cannot be", () => {
    push.current = device({ on: true });
    const { unmount } = render(<PushNudge />);
    expect(screen.queryByRole("button")).toBeNull();
    unmount();
    push.current = device({ support: "needs-home-screen" });
    render(<PushNudge />);
    expect(screen.queryByRole("button")).toBeNull();
  });
});

describe("ChatView's nudge", () => {
  const card = (id: number, status: ArtifactRef["status"]): ArtifactRef => ({
    id,
    kind: "document",
    title: `Plan ${id}`,
    summary: "",
    status,
    preview: { lines: [] },
  });
  const turn = (artifacts: ArtifactRef[]): Turn => ({
    at: new Date().toISOString(),
    said: "plan it",
    reply: "I'm putting it together.",
    via: "app",
    room: null,
    by: null,
    proposal: null,
    artifacts,
  });

  function chat(turns: Turn[], working: number[]) {
    render(
      <ChatView
        turns={turns}
        onSend={() => {}}
        onNew={() => {}}
        onOpenPanel={() => {}}
        onAnswer={() => {}}
        working={working}
      />,
    );
  }

  it("is offered once, under the newest job still being written", () => {
    push.current = device();
    chat([turn([card(1, "working")]), turn([card(2, "working")])], [1, 2]);
    expect(screen.getAllByRole("button", { name: "Notify me when it's done" })).toHaveLength(1);
  });

  it("is not offered for a job that has finished since its card was kept", () => {
    push.current = device();
    chat([turn([card(1, "working")])], []);
    expect(screen.queryByRole("button", { name: "Notify me when it's done" })).toBeNull();
  });
});
