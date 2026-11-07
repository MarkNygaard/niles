import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { api, type RestartStatus, type UpdateStatus } from "@/lib/api";
import { AboutCard } from "@/components/AboutCard";

vi.mock("@/hooks/useDeviceStream", () => ({ useDeviceStream: () => {} }));

const SHA = "0123456789abcdef0123456789abcdef01234567";
const NEWER = "89abcdef0123456789abcdef0123456789abcdef";
const DIGEST = `sha256:${"a".repeat(64)}`;

function update(overrides: Partial<UpdateStatus> = {}): UpdateStatus {
  return {
    running: { version: "0.6.0", commit: SHA, built_at: "2026-11-01T10:15:00Z" },
    state: "up_to_date",
    checked_at: "2026-11-06T10:00:00Z",
    failed: null,
    newest: null,
    newer_than_published: false,
    changes: null,
    changes_failed: null,
    compare_url: null,
    namespace: null,
    ...overrides,
  };
}

function restart(overrides: Partial<RestartStatus> = {}): RestartStatus {
  return {
    boot: { id: "boot-a", started_at: "2026-11-01T00:00:00Z" },
    pending: [],
    supervisor: null,
    persistent: true,
    restarting: false,
    cut_short: { jobs: 0, wake_ups: 0, approvals: 0 },
    build: { version: "0.6.0", commit: null, built_at: null },
    ...overrides,
  };
}

function renderCard() {
  return render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <AboutCard />
    </QueryClientProvider>,
  );
}

describe("AboutCard", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(api, "restartStatus").mockResolvedValue(restart());
  });

  /** Both spies answer the same: the mount check writes its answer
      into the query cache. */
  function mockUpdate(status: UpdateStatus) {
    vi.spyOn(api, "updateStatus").mockResolvedValue(status);
    vi.spyOn(api, "checkForUpdate").mockResolvedValue(status);
  }

  it("shows the version, the linked commit and the build date", async () => {
    mockUpdate(update());
    renderCard();
    expect(await screen.findByText("0.6.0")).toBeInTheDocument();
    const commit = screen.getByRole("link", { name: SHA.slice(0, 7) });
    expect(commit).toHaveAttribute("href", `https://github.com/MarkNygaard/niles/commit/${SHA}`);
    // The long local date of the build time: some day in 2026.
    expect(screen.getByText(/2026/)).toBeInTheDocument();
  });

  it("says development build with no Check again, and still checks on mount", async () => {
    mockUpdate(
      update({
        running: { version: "0.6.0", commit: null, built_at: null },
        state: "dev_build",
      }),
    );
    renderCard();
    expect(
      await screen.findByText(/This is a development build, so Niles doesn't check/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Check again/ })).toBeNull();
    expect(api.checkForUpdate).toHaveBeenCalledTimes(1);
  });

  it("checks once on mount", async () => {
    mockUpdate(update());
    renderCard();
    await screen.findByText(/Up to date/);
    expect(api.checkForUpdate).toHaveBeenCalledTimes(1);
  });

  it("shows what's new and how to update for an available update", async () => {
    vi.spyOn(api, "restartStatus").mockResolvedValue(restart({ supervisor: "kubernetes" }));
    const status = update({
        state: "update_available",
        newest: {
          digest: DIGEST,
          image: `ghcr.io/marknygaard/niles@${DIGEST}`,
          commit: NEWER,
          created: "2026-11-05T09:30:00Z",
          version: "0.7.0",
        },
        changes: {
          commits: [
            {
              sha: NEWER,
              subject: "fix: second (#512)",
              pr: 512,
              url: "https://github.com/MarkNygaard/niles/pull/512",
              at: "2026-11-04T10:00:00Z",
            },
          ],
          truncated: false,
        },
        compare_url: `https://github.com/MarkNygaard/niles/compare/${SHA}...${NEWER}`,
    });
    mockUpdate(status);
    renderCard();
    expect(await screen.findByText("Update available")).toBeInTheDocument();
    const change = screen.getByRole("link", { name: "fix: second (#512)" });
    expect(change).toHaveAttribute("href", "https://github.com/MarkNygaard/niles/pull/512");
    expect(screen.getByText(/kubectl rollout restart deployment\/niles/)).toBeInTheDocument();
    expect(screen.getByText(`ghcr.io/marknygaard/niles@${DIGEST}`)).toBeInTheDocument();
  });

  it("says up to date", async () => {
    mockUpdate(update());
    renderCard();
    expect(await screen.findByText(/Up to date/)).toBeInTheDocument();
  });

  it("says why it couldn't check, and Check again checks again", async () => {
    mockUpdate(
      update({ state: "not_checked", checked_at: null, failed: { at: "2026-11-06T10:00:00Z", reason: "GitHub's container registry answered HTTP 503" } }),
    );
    renderCard();
    expect(
      await screen.findByText(/Couldn't check for a newer Niles\. GitHub's container registry answered HTTP 503\./),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    await screen.findByRole("button", { name: /Check/ });
    expect(api.checkForUpdate).toHaveBeenCalledTimes(2);
  });

  it("links the compare page when what changed couldn't be listed", async () => {
    mockUpdate(
      update({
        state: "update_available",
        newest: {
          digest: DIGEST,
          image: `ghcr.io/marknygaard/niles@${DIGEST}`,
          commit: NEWER,
          created: "2026-11-05T09:30:00Z",
          version: "0.7.0",
        },
        changes_failed: "GitHub doesn't know one of the two commits, so what changed can't be listed",
        compare_url: `https://github.com/MarkNygaard/niles/compare/${SHA}...${NEWER}`,
      }),
    );
    renderCard();
    expect(
      await screen.findByRole("link", { name: "See the changes on GitHub" }),
    ).toHaveAttribute(
      "href",
      `https://github.com/MarkNygaard/niles/compare/${SHA}...${NEWER}`,
    );
  });
});
