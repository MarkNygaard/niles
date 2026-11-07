import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RestartStatus, UpdateStatus } from "@/lib/api";
import {
  RESTART_WAITING,
  UPDATE_AVAILABLE,
  forgetBuild,
  howToUpdate,
  noticeBuild,
  settingsDotFor,
} from "@/lib/update";

const SHA = "0123456789abcdef0123456789abcdef01234567";

function restart(pending: number): RestartStatus {
  return {
    boot: { id: "boot-a", started_at: "2026-11-01T00:00:00Z" },
    pending: Array.from({ length: pending }, (_, i) => ({
      what: `what-${i}`,
      label: `Label ${i}`,
      since: "2026-11-01T00:00:00Z",
    })),
    supervisor: null,
    persistent: true,
    restarting: false,
    cut_short: { jobs: 0, wake_ups: 0, approvals: 0 },
    build: { version: "0.6.0", commit: null, built_at: null },
  };
}

function update(state: UpdateStatus["state"]): UpdateStatus {
  return {
    running: { version: "0.6.0", commit: SHA, built_at: "2026-11-01T00:00:00Z" },
    state,
    checked_at: "2026-11-06T10:00:00Z",
    failed: null,
    newest:
      state === "update_available"
        ? {
            digest: `sha256:${"a".repeat(64)}`,
            image: `ghcr.io/marknygaard/niles@sha256:${"a".repeat(64)}`,
            commit: "89abcdef0123456789abcdef0123456789abcdef",
            created: "2026-11-05T09:30:00Z",
            version: "0.7.0",
          }
        : null,
    newer_than_published: false,
    changes: null,
    changes_failed: null,
    compare_url:
      state === "update_available"
        ? `https://github.com/MarkNygaard/niles/compare/${SHA}...89abcdef0123456789abcdef0123456789abcdef`
        : null,
    namespace: null,
  };
}

describe("settingsDotFor", () => {
  it("says both when a restart waits and an update is available", () => {
    expect(settingsDotFor(restart(1), update("update_available"))).toBe(
      "a restart is waiting and an update is available",
    );
  });

  it("says the restart alone", () => {
    expect(settingsDotFor(restart(1), update("up_to_date"))).toBe(RESTART_WAITING);
    expect(settingsDotFor(restart(1), undefined)).toBe(RESTART_WAITING);
  });

  it("says the update alone", () => {
    expect(settingsDotFor(restart(0), update("update_available"))).toBe(UPDATE_AVAILABLE);
    expect(settingsDotFor(undefined, update("update_available"))).toBe(UPDATE_AVAILABLE);
  });

  it("says nothing when neither waits", () => {
    expect(settingsDotFor(restart(0), update("up_to_date"))).toBeUndefined();
    expect(settingsDotFor(undefined, undefined)).toBeUndefined();
  });
});

describe("howToUpdate", () => {
  it("offers the rollout restart and the pin for kubernetes", () => {
    const steps = howToUpdate(
      { ...update("update_available"), namespace: "niles" },
      "kubernetes",
    );
    expect(steps).toHaveLength(2);
    expect(steps[0].restart).toBe(true);
    expect(steps[0].command).toBe("kubectl -n niles rollout restart deployment/niles");
    expect(steps[1].command).toBe(update("update_available").newest?.image);
  });

  it("leaves the namespace out when it is not known", () => {
    const steps = howToUpdate(update("update_available"), "kubernetes");
    expect(steps[0].command).toBe("kubectl rollout restart deployment/niles");
  });

  it("names the version in the pin step when there is one", () => {
    const steps = howToUpdate(update("update_available"), "kubernetes");
    expect(steps[1].text).toContain("change it to 0.7.0");
  });

  it("names the short sha when the newest has no version", () => {
    const newest = update("update_available");
    const steps = howToUpdate(
      {
        ...newest,
        newest: newest.newest && { ...newest.newest, version: null },
      },
      "kubernetes",
    );
    expect(steps[1].text).toContain("change it to sha-89abcde");
  });

  it("offers the compose commands for docker", () => {
    const steps = howToUpdate(update("update_available"), "docker");
    expect(steps[0].command).toBe("docker compose pull niles && docker compose up -d niles");
    expect(steps[1].command).toBe(update("update_available").newest?.image);
  });

  it("offers the digest alone for anything else", () => {
    const steps = howToUpdate(update("update_available"), null);
    expect(steps).toHaveLength(1);
    expect(steps[0].command).toBe(update("update_available").newest?.image);
  });

  it("says nothing when no update is available", () => {
    expect(howToUpdate(update("up_to_date"), "kubernetes")).toEqual([]);
  });
});

describe("noticeBuild", () => {
  beforeEach(() => forgetBuild());

  it("adopts the first answer without reloading", () => {
    const reload = vi.fn();
    noticeBuild("a", reload);
    expect(reload).not.toHaveBeenCalled();
  });

  it("ignores the same commit", () => {
    const reload = vi.fn();
    noticeBuild("a", reload);
    noticeBuild("a", reload);
    expect(reload).not.toHaveBeenCalled();
  });

  it("reloads once for a different commit, and not again", () => {
    const reload = vi.fn();
    noticeBuild("a", reload);
    noticeBuild("b", reload);
    noticeBuild("c", reload);
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("treats null as a real answer", () => {
    const reload = vi.fn();
    noticeBuild(null, reload);
    noticeBuild(null, reload);
    expect(reload).not.toHaveBeenCalled();
    noticeBuild("a", reload);
    expect(reload).toHaveBeenCalledTimes(1);
  });
});
