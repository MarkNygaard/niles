import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, renderHook } from "@testing-library/react";
import { useDeviceStream } from "@/hooks/useDeviceStream";
import { RESTART, stopWaiting, useRestarting } from "@/lib/restart";

class FakeSocket {
  static instances: FakeSocket[] = [];
  readonly url: string;
  onopen: (() => void) | null = null;
  onmessage: ((message: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  closed = false;

  constructor(url: string) {
    this.url = url;
    FakeSocket.instances.push(this);
  }

  send() {}
  close() {
    this.closed = true;
    this.onclose?.();
  }

  /** Test hooks. */
  open() {
    this.onopen?.();
  }
  message(frame: unknown) {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
}

function Stream({ enabled = true }: { enabled?: boolean }) {
  useDeviceStream(enabled);
  return null;
}

function renderStream(queryClient: QueryClient, enabled = true) {
  return render(
    <QueryClientProvider client={queryClient}>
      <Stream enabled={enabled} />
    </QueryClientProvider>,
  );
}

describe("useDeviceStream restart frames", () => {
  beforeEach(() => {
    FakeSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeSocket);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    stopWaiting();
  });

  it("invalidates the restart query on restart_pending_changed", async () => {
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderStream(queryClient);
    const socket = FakeSocket.instances[0];
    socket.open();
    invalidate.mockClear();
    socket.message({ type: "restart_pending_changed" });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: RESTART });
  });

  it("starts waiting on restart_started, with the boot id", () => {
    const queryClient = new QueryClient();
    renderStream(queryClient);
    const socket = FakeSocket.instances[0];
    socket.open();
    socket.message({ type: "restart_started", boot_id: "boot-going" });
    const { result } = renderHook(() => useRestarting());
    expect(result.current).toEqual({ before: "boot-going", supervisor: null });
  });

  it("asks again on a reconnect", () => {
    vi.useFakeTimers();
    try {
      const queryClient = new QueryClient();
      const invalidate = vi.spyOn(queryClient, "invalidateQueries");
      renderStream(queryClient);
      const first = FakeSocket.instances[0];
      first.open();
      invalidate.mockClear();
      first.close();
      vi.advanceTimersByTime(1_000);
      expect(FakeSocket.instances.length).toBe(2);
      FakeSocket.instances[1].open();
      expect(invalidate).toHaveBeenCalledWith({ queryKey: RESTART });
    } finally {
      vi.useRealTimers();
    }
  });

  it("opens no socket when disabled", () => {
    const queryClient = new QueryClient();
    renderStream(queryClient, false);
    expect(FakeSocket.instances).toHaveLength(0);
  });
});
