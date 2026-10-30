import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ApiError, api, type RestartStatus } from "@/lib/api";
import { stopWaiting, useRestarting } from "@/lib/restart";
import { RestartBar } from "@/components/RestartBar";

vi.mock("@/hooks/useDeviceStream", () => ({ useDeviceStream: () => {} }));

function status(overrides: Partial<RestartStatus> = {}): RestartStatus {
  return {
    boot: { id: "boot-a", started_at: "2026-10-30T00:00:00Z" },
    pending: [],
    supervisor: null,
    persistent: true,
    restarting: false,
    cut_short: { jobs: 0, wake_ups: 0, approvals: 0 },
    ...overrides,
  };
}

function renderBar() {
  const queryClient = new QueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <RestartBar />
    </QueryClientProvider>,
  );
}

describe("RestartBar", () => {
  beforeEach(() => {
    vi.spyOn(api, "restartStatus").mockResolvedValue(status());
  });
  afterEach(() => {
    stopWaiting();
    vi.restoreAllMocks();
  });

  it("shows nothing when nothing waits", () => {
    const { container } = renderBar();
    expect(container).toBeEmptyDOMElement();
  });

  it("names what waits", async () => {
    vi.spyOn(api, "restartStatus").mockResolvedValue(
      status({
        pending: [
          { what: "mqtt", label: "Zigbee & MQTT", since: "2026-10-30T00:00:00Z" },
          { what: "llm", label: "Thinking", since: "2026-10-30T00:00:00Z" },
        ],
      }),
    );
    renderBar();
    expect(
      await screen.findByText(/Some changes take effect after a restart: Zigbee & MQTT, Thinking\./),
    ).toBeInTheDocument();
  });

  it("says to restart where Niles runs when nothing would start it again", async () => {
    vi.spyOn(api, "restartStatus").mockResolvedValue(
      status({
        pending: [{ what: "mqtt", label: "Zigbee & MQTT", since: "x" }],
      }),
    );
    renderBar();
    expect(
      await screen.findByText(/Restart Niles where it runs to apply them\./),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Restart now" })).not.toBeInTheDocument();
  });

  it("points at niles.toml when there is no database", async () => {
    vi.spyOn(api, "restartStatus").mockResolvedValue(
      status({
        pending: [{ what: "mqtt", label: "Zigbee & MQTT", since: "x" }],
        supervisor: "kubernetes",
        persistent: false,
      }),
    );
    renderBar();
    expect(await screen.findByText(/set them in niles\.toml instead\./)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Restart now" })).not.toBeInTheDocument();
  });

  it("confirms and restarts under kubernetes, then waits for the new Niles", async () => {
    vi.spyOn(api, "restartStatus").mockResolvedValue(
      status({
        supervisor: "kubernetes",
        pending: [{ what: "mqtt", label: "Zigbee & MQTT", since: "x" }],
        cut_short: { jobs: 1, wake_ups: 0, approvals: 2 },
      }),
    );
    const restart = vi.spyOn(api, "restart").mockResolvedValue({
      // The cached status can predate a deploy. Wait for the process
      // that accepted this restart, not the one the page first saw.
      boot: { id: "boot-b", started_at: "x" },
    });
    renderBar();
    fireEvent.click(await screen.findByRole("button", { name: "Restart now" }));
    expect(await screen.findByText(/about half a minute/)).toBeInTheDocument();
    expect(
      screen.getByText("A background job the assistant is doing stops before it finishes."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Kubernetes may start a newer image of Niles, if one was published since this one started.",
      ),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Restart" }));
    await waitFor(() => expect(restart).toHaveBeenCalledTimes(1));
    await waitFor(() => {
      const {
        result: { current },
      } = renderHook(() => useRestarting());
      expect(current).toEqual({ before: "boot-b", supervisor: "kubernetes" });
    });
  });

  it("shows a refused restart in the dialog", async () => {
    vi.spyOn(api, "restartStatus").mockResolvedValue(
      status({
        supervisor: "docker",
        pending: [{ what: "mqtt", label: "Zigbee & MQTT", since: "x" }],
      }),
    );
    vi.spyOn(api, "restart").mockRejectedValue(new ApiError("…would only stop it…", 409));
    renderBar();
    fireEvent.click(await screen.findByRole("button", { name: "Restart now" }));
    fireEvent.click(screen.getByRole("button", { name: "Restart" }));
    expect(await screen.findByText("…would only stop it…")).toBeInTheDocument();
  });
});
