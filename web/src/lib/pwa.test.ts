import { afterEach, describe, expect, it, vi } from "vitest";
import { approvalTag } from "@/lib/approvals";
import { REPLACED_WITHIN_MS, closeWaiting, closeWaitingLater } from "@/lib/pwa";

interface FakeNotification {
  data: { approval?: string; url?: string } | null;
  close: () => void;
}

function stubNotifications(listed: FakeNotification[]) {
  const getNotifications = vi.fn(async () => listed);
  Object.defineProperty(navigator, "serviceWorker", {
    configurable: true,
    value: { getRegistration: async () => ({ getNotifications }) },
  });
  return getNotifications;
}

describe("closeWaiting", () => {
  afterEach(() => {
    Reflect.deleteProperty(navigator, "serviceWorker");
  });

  it("closes only the ones that still ask", async () => {
    const waiting: FakeNotification = { data: { approval: "a1" }, close: vi.fn() };
    const answered: FakeNotification = { data: { url: "/#/chat?approval=a1" }, close: vi.fn() };
    const getNotifications = stubNotifications([waiting, answered]);
    await closeWaiting("approval-a1");
    expect(getNotifications).toHaveBeenCalledWith({ tag: "approval-a1" });
    expect(waiting.close).toHaveBeenCalled();
    expect(answered.close).not.toHaveBeenCalled();
  });

  it("does nothing without a service worker", async () => {
    await expect(closeWaiting("approval-a1")).resolves.toBeUndefined();
  });
});

describe("closeWaitingLater", () => {
  afterEach(() => {
    Reflect.deleteProperty(navigator, "serviceWorker");
    vi.useRealTimers();
  });

  it("waits for Niles's push before closing", async () => {
    vi.useFakeTimers();
    const waiting: FakeNotification = { data: { approval: "a1" }, close: vi.fn() };
    stubNotifications([waiting]);
    closeWaitingLater("approval-a1");
    await vi.advanceTimersByTimeAsync(REPLACED_WITHIN_MS - 1);
    expect(waiting.close).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(waiting.close).toHaveBeenCalledTimes(1);
  });
});

describe("approvalTag", () => {
  it("is the tag Niles pushes them under", () => {
    expect(approvalTag("a1")).toBe("approval-a1");
  });
});
