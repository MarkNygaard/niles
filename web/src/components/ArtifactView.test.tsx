import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ArtifactView } from "@/components/ArtifactView";
import { api } from "@/lib/api";
import type { ArtifactDetail, ArtifactPatch } from "@/lib/api";

const NOW = new Date().toISOString();

function detail(rest: Partial<ArtifactDetail> = {}): ArtifactDetail {
  return {
    id: 5,
    kind: "document",
    title: "Packing list",
    summary: "For the cabin",
    pinned: false,
    archived: false,
    notify: true,
    status: "ready",
    status_text: null,
    error: null,
    created_at: NOW,
    updated_at: NOW,
    current_revision: 2,
    preview: { lines: [] },
    content: { markdown: "# Packing\n\n- [ ] Passport\n- [x] Charger" },
    revisions: [
      { n: 2, reason: "Added the charger", author: "claude", at: NOW },
      { n: 1, reason: "First version", author: "claude", at: NOW },
    ],
    ...rest,
  };
}

function show(start: ArtifactDetail = detail()) {
  let current = start;
  vi.spyOn(api, "artifact").mockImplementation(async () => current);
  const patch = vi
    .spyOn(api, "patchArtifact")
    .mockImplementation(async (_id: number, p: ArtifactPatch) => {
      current = {
        ...current,
        ...(p.notify !== undefined && { notify: p.notify }),
        ...(p.remove_page && { page: null, page_html: null }),
        ...(p.content !== undefined && {
          content: p.content,
          current_revision: current.current_revision + 1,
          revisions: [
            {
              n: current.current_revision + 1,
              reason: p.reason ?? "",
              author: "person" as const,
              at: NOW,
            },
            ...current.revisions,
          ],
        }),
      };
      return current;
    });
  const back = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ArtifactView id={5} onBack={back} />
    </QueryClientProvider>,
  );
  return { patch, back };
}

describe("ArtifactView", () => {
  afterEach(() => vi.restoreAllMocks());

  it("shows the document, and goes back to the list", async () => {
    const { back } = show();
    expect(await screen.findByRole("heading", { name: "Packing list" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Packing" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Saved" }));
    expect(back).toHaveBeenCalled();
  });

  it("saves a tick as a version by the person", async () => {
    const { patch } = show();
    fireEvent.click(await screen.findByRole("checkbox", { name: "Passport" }));
    await waitFor(() => expect(patch).toHaveBeenCalledTimes(1));
    expect(patch.mock.calls[0]).toEqual([
      5,
      {
        content: { markdown: "# Packing\n\n- [x] Passport\n- [x] Charger" },
        reason: "Ticked “Passport”",
      },
    ]);
    expect(await screen.findByRole("checkbox", { name: "Passport" })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Versions (3)" }));
    const versions = within(screen.getByRole("list", { name: "Versions" }));
    expect(versions.getByText(/Version 3 \(current\) · Ticked “Passport”/)).toBeInTheDocument();
    expect(versions.getAllByText(/^You ·/)).toHaveLength(1);
  });

  it("reads an older version and restores it", async () => {
    show();
    vi.spyOn(api, "artifactRevision").mockResolvedValue({
      n: 1,
      reason: "First version",
      author: "claude",
      at: NOW,
      content: { markdown: "- [ ] Passport only" },
    });
    const restore = vi.spyOn(api, "restoreArtifact").mockResolvedValue(
      detail({
        current_revision: 3,
        content: { markdown: "- [ ] Passport only" },
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Versions (2)" }));
    fireEvent.click(screen.getByText(/Version 1 · First version/));
    expect(await screen.findByText("Version 1, as it was")).toBeInTheDocument();
    // An old version is read, not ticked.
    expect(await screen.findByRole("checkbox", { name: "Passport only" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Restore this version" }));
    await waitFor(() => expect(restore).toHaveBeenCalledWith(5, 1));
    await waitFor(() => expect(screen.queryByText("Version 1, as it was")).toBeNull());
  });

  it("switches off being told", async () => {
    const { patch } = show();
    fireEvent.click(await screen.findByRole("switch"));
    await waitFor(() => expect(patch).toHaveBeenCalledWith(5, { notify: false }));
  });

  it("copies the document as markdown", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Copy" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("# Packing\n\n- [ ] Passport\n- [x] Charger"),
    );
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("says what a job is doing while it writes", async () => {
    show(
      detail({
        status: "working",
        status_text: "Writing your Thailand travel plan…",
        content: null,
        current_revision: 0,
        revisions: [],
      }),
    );
    expect(await screen.findByText("Writing your Thailand travel plan…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
  });

  it("says why a job did not finish, and tries it again", async () => {
    show(
      detail({ status: "failed", error: "Claude did not finish", content: null, revisions: [] }),
    );
    const retry = vi.spyOn(api, "retryArtifact").mockResolvedValue(detail({ status: "working" }));
    expect(await screen.findByText(/It didn't finish: Claude did not finish/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(retry).toHaveBeenCalledWith(5));
  });

  describe("with a page", () => {
    const PAGED = () =>
      detail({
        page: 3,
        page_html: "<h1>A poster of the packing</h1>",
        today: "2026-10-16",
      });

    /** The frame saying something to the view, as it would. */
    const say = (data: Record<string, unknown>) => {
      const frame = screen.getByTitle("Packing list") as HTMLIFrameElement;
      act(() => {
        window.dispatchEvent(
          new MessageEvent("message", {
            data: { niles: 1, ...data },
            origin: "null",
            source: frame.contentWindow,
          }),
        );
      });
    };

    it("shows the page in its frame, and the built-in view on the switch", async () => {
      show(PAGED());
      const frame = (await screen.findByTitle("Packing list")) as HTMLIFrameElement;
      expect(frame.tagName).toBe("IFRAME");
      expect(frame.getAttribute("srcdoc")).toContain("A poster of the packing");
      expect(frame.getAttribute("srcdoc")).toContain('"today":"2026-10-16"');
      expect(screen.queryByRole("checkbox", { name: "Passport" })).toBeNull();
      expect(screen.getByRole("button", { name: "Page" })).toHaveAttribute("aria-pressed", "true");

      fireEvent.click(screen.getByRole("button", { name: "Built-in" }));
      expect(screen.queryByTitle("Packing list")).toBeNull();
      expect(screen.getByRole("checkbox", { name: "Passport" })).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Page" }));
      expect(await screen.findByTitle("Packing list")).toBeInTheDocument();
    });

    it("falls back to the built-in view when the page breaks, and tries it again on a tap", async () => {
      show(PAGED());
      await screen.findByTitle("Packing list");
      say({ type: "error", message: "boom" });
      expect(await screen.findByText(/The page could not be shown: it stopped with an error \(boom\)/)).toBeInTheDocument();
      expect(screen.getByRole("checkbox", { name: "Passport" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Built-in" })).toHaveAttribute("aria-pressed", "true");
      fireEvent.click(screen.getByRole("button", { name: "Page" }));
      expect(await screen.findByTitle("Packing list")).toBeInTheDocument();
      expect(screen.queryByText(/The page could not be shown/)).toBeNull();
    });

    it("saves what the page saves as a version by the person", async () => {
      const { patch } = show(PAGED());
      await screen.findByTitle("Packing list");
      vi.useFakeTimers();
      try {
        say({ type: "save", content: { markdown: "- [x] Passport" }, reason: "Packed passport" });
        act(() => vi.advanceTimersByTime(2000));
      } finally {
        vi.useRealTimers();
      }
      await waitFor(() =>
        expect(patch).toHaveBeenCalledWith(5, {
          content: { markdown: "- [x] Passport" },
          reason: "On its page: Packed passport",
        }),
      );
    });

    it("takes the page away, as a version", async () => {
      const { patch } = show(PAGED());
      fireEvent.click(await screen.findByRole("button", { name: "Remove page" }));
      await waitFor(() => expect(patch).toHaveBeenCalledWith(5, { remove_page: true }));
      expect(await screen.findByRole("checkbox", { name: "Passport" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Built-in" })).toBeNull();
    });

    it("has no switch and no frame without a page", async () => {
      show();
      expect(await screen.findByRole("checkbox", { name: "Passport" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Built-in" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Remove page" })).toBeNull();
      expect(screen.queryByTitle("Packing list")).toBeNull();
    });
  });
});
