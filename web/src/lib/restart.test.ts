import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, type RestartStatus } from "@/lib/api";
import {
  GIVE_UP_MS,
  POLL_MS,
  canRestart,
  logHint,
  restartNotes,
  startWaiting,
  stopWaiting,
  useRestarting,
  waitForNewBoot,
} from "@/lib/restart";
import { act, renderHook } from "@testing-library/react";

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

describe("restartNotes", () => {
  it("says nothing when nothing is under way", () => {
    expect(restartNotes(status())).toEqual([]);
  });

  it("names the singular of each kind", () => {
    const notes = restartNotes(
      status({ cut_short: { jobs: 1, wake_ups: 1, approvals: 1 } }),
    );
    expect(notes).toEqual([
      "A background job the assistant is doing stops before it finishes.",
      "An approval waiting for a yes lapses.",
      "A wake-up ramp under way pauses, and carries on when Niles is back.",
    ]);
  });

  it("names the plural of each kind", () => {
    const notes = restartNotes(
      status({ cut_short: { jobs: 2, wake_ups: 3, approvals: 4 } }),
    );
    expect(notes).toEqual([
      "2 background jobs the assistant is doing stop before they finish.",
      "4 approvals waiting for a yes lapse.",
      "3 wake-up ramps under way pause, and carry on when Niles is back.",
    ]);
  });

  it("only mentions a newer image under kubernetes", () => {
    const note = "Kubernetes may start a newer image of Niles, if one was published since this one started.";
    expect(restartNotes(status({ supervisor: "kubernetes" }))).toEqual([note]);
    expect(restartNotes(status({ supervisor: "docker" }))).toEqual([]);
  });
});

describe("canRestart", () => {
  it("needs a supervisor, persistence, and a quiet process", () => {
    expect(canRestart(status({ supervisor: "docker" }))).toBe(true);
    expect(canRestart(status())).toBe(false);
    expect(canRestart(status({ supervisor: "docker", persistent: false }))).toBe(false);
    expect(
      canRestart(status({ supervisor: "docker", restarting: true })),
    ).toBe(false);
  });
});

describe("logHint", () => {
  it("names the log command per supervisor", () => {
    expect(logHint("kubernetes")).toEqual({
      text: "See what it says with",
      command: "kubectl logs deploy/niles",
    });
    expect(logHint("docker")).toEqual({
      text: "See what it says with",
      command: "docker compose logs niles",
    });
    expect(logHint("systemd")).toEqual({
      text: "See what it says with",
      command: "journalctl -u niles",
    });
    expect(logHint(null)).toEqual({ text: "Look at its log where it runs." });
  });
});

describe("waitForNewBoot", () => {
  /** A clock that advances by step on every read. */
  function steppingClock(step = POLL_MS) {
    let t = 0;
    return () => {
      t += step;
      return t;
    };
  }

  it("comes back when the id changes", async () => {
    let answers = 0;
    const outcome = await waitForNewBoot("boot-a", {
      status: () => {
        answers += 1;
        return Promise.resolve(status(answers < 3 ? {} : { boot: { id: "boot-b", started_at: "x" } }));
      },
      sleep: () => Promise.resolve(),
      now: steppingClock(),
    });
    expect(outcome).toBe("back");
    expect(answers).toBe(3);
  });

  it("waits through a network error and a 502 from the ingress", async () => {
    let answers = 0;
    const outcome = await waitForNewBoot("boot-a", {
      status: () => {
        answers += 1;
        if (answers === 1) return Promise.reject(new TypeError("network down"));
        if (answers === 2) return Promise.reject(new ApiError("bad gateway", 502));
        return Promise.resolve(status({ boot: { id: "boot-b", started_at: "x" } }));
      },
      sleep: () => Promise.resolve(),
      now: steppingClock(),
    });
    expect(outcome).toBe("back");
  });

  it("treats a 401 as back: a new Niles answered, and sign-in shows", async () => {
    const outcome = await waitForNewBoot("boot-a", {
      status: () => Promise.reject(new ApiError("unauthorised", 401)),
      sleep: () => Promise.resolve(),
      now: steppingClock(),
    });
    expect(outcome).toBe("back");
  });

  it("gives up after two minutes", async () => {
    const outcome = await waitForNewBoot("boot-a", {
      status: () => Promise.resolve(status()),
      sleep: () => Promise.resolve(),
      now: steppingClock(POLL_MS * 2),
      giveUpMs: GIVE_UP_MS,
    });
    expect(outcome).toBe("gave_up");
  });

  it("aborts when the signal fires", async () => {
    const controller = new AbortController();
    controller.abort();
    const outcome = await waitForNewBoot("boot-a", {
      status: () => Promise.resolve(status()),
      sleep: () => Promise.resolve(),
      now: steppingClock(),
      signal: controller.signal,
    });
    expect(outcome).toBe("aborted");
  });
});

describe("the waiting store", () => {
  afterEach(() => stopWaiting());

  it("keeps the first wait and clears on stop", () => {
    const { result } = renderHook(() => useRestarting());
    expect(result.current).toBeNull();
    act(() => startWaiting({ before: "boot-a", supervisor: "docker" }));
    expect(result.current).toEqual({ before: "boot-a", supervisor: "docker" });
    act(() => startWaiting({ before: "boot-b", supervisor: "docker" }));
    expect(result.current?.before).toBe("boot-a", "a second start is ignored");
    act(() => stopWaiting());
    expect(result.current).toBeNull();
  });
});
