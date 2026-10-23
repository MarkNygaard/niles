import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { NotificationBell } from "@/components/NotificationBell";
import { api } from "@/lib/api";
import type { NotificationList } from "@/lib/api";

/** A WebSocket that only the test speaks through. */
class FakeSocket {
  static last: FakeSocket | null = null;
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor() {
    FakeSocket.last = this;
  }
  close() {}
}

function list(unread: number): NotificationList {
  return { notifications: [], unread, next: null, kept: true };
}

function show() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <NotificationBell />
    </QueryClientProvider>,
  );
}

describe("NotificationBell", () => {
  beforeEach(() => vi.stubGlobal("WebSocket", FakeSocket));
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("counts what is unread and opens the page", async () => {
    vi.spyOn(api, "notifications").mockResolvedValue(list(3));
    show();
    const bell = await screen.findByRole("link", { name: "Notifications, 3 unread" });
    expect(bell).toHaveAttribute("href", "#/notifications");
    expect(bell).toHaveTextContent("3");
  });

  it("says 9+ over nine", async () => {
    vi.spyOn(api, "notifications").mockResolvedValue(list(12));
    show();
    expect(await screen.findByRole("link", { name: "Notifications, 12 unread" })).toHaveTextContent(
      "9+",
    );
  });

  it("shows no badge with nothing unread", async () => {
    vi.spyOn(api, "notifications").mockResolvedValue(list(0));
    show();
    const bell = await screen.findByRole("link", { name: "Notifications" });
    expect(bell.querySelector("[data-slot=bell-badge]")).toBeNull();
  });

  it("follows the stream without a reload", async () => {
    const asked = vi
      .spyOn(api, "notifications")
      .mockResolvedValueOnce(list(0))
      .mockResolvedValueOnce(list(1))
      .mockResolvedValueOnce(list(0));
    show();
    await screen.findByRole("link", { name: "Notifications" });

    act(() =>
      FakeSocket.last!.onmessage!({ data: JSON.stringify({ type: "notification_added", id: 4 }) }),
    );
    expect(await screen.findByRole("link", { name: "Notifications, 1 unread" })).toBeInTheDocument();

    act(() => FakeSocket.last!.onmessage!({ data: JSON.stringify({ type: "notifications_read" }) }));
    expect(await screen.findByRole("link", { name: "Notifications" })).toBeInTheDocument();
    expect(asked).toHaveBeenCalledTimes(3);
  });
});
