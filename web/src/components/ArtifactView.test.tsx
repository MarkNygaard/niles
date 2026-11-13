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

async function openMenu() {
  fireEvent.click(await screen.findByRole("button", { name: "Share and download" }));
  await screen.findByRole("menu");
}

function menuItems(): string[] {
  return within(screen.getByRole("menu"))
    .getAllByRole("menuitem")
    .map((item) => item.textContent ?? "");
}

/** Files handed to the browser to save, caught at the link's click. */
function catchDownloads(): { name: string; blob: Blob }[] {
  const blobs = new Map<string, Blob>();
  const saved: { name: string; blob: Blob }[] = [];
  let n = 0;
  Object.defineProperty(URL, "createObjectURL", {
    configurable: true,
    value: (blob: Blob) => {
      const url = `blob:test/${n++}`;
      blobs.set(url, blob);
      return url;
    },
  });
  Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: () => {} });
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
    this: HTMLAnchorElement,
  ) {
    const blob = blobs.get(this.href);
    if (blob) saved.push({ name: this.download, blob });
  });
  return saved;
}

function textOf(blob: Blob): Promise<string> {
  return new Promise((resolve) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.readAsText(blob);
  });
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

  it("copies the document as markdown, from the share menu", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    show();
    await openMenu();
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("# Packing\n\n- [ ] Passport\n- [x] Charger"),
    );
    // Said in the menu, which stays open to say it.
    expect(await screen.findByRole("menuitem", { name: "Copied" })).toBeInTheDocument();
  });

  it("keeps only Versions under the item: Share and Copy are in the menu", async () => {
    show(detail({ page: 3, page_html: "<p>poster</p>" }));
    expect(await screen.findByRole("button", { name: "Versions (2)" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove page" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Share" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
  });

  it("offers a document as Word and as Markdown, and Share to its owner", async () => {
    show();
    await openMenu();
    expect(menuItems()).toEqual(["Share…", "Copy", "Download as Word", "Download as Markdown"]);
  });

  it("downloads the markdown as a file named after it", async () => {
    const saved = catchDownloads();
    show();
    await openMenu();
    fireEvent.click(screen.getByRole("menuitem", { name: "Download as Markdown" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].name).toBe("Packing list.md");
    expect(await textOf(saved[0].blob)).toBe("# Packing\n\n- [ ] Passport\n- [x] Charger");
  });

  it("downloads a Word file named after it", async () => {
    const saved = catchDownloads();
    show();
    await openMenu();
    fireEvent.click(screen.getByRole("menuitem", { name: "Download as Word" }));
    await waitFor(() => expect(saved).toHaveLength(1), { timeout: 5000 });
    expect(saved[0].name).toBe("Packing list.docx");
    expect(saved[0].blob.size).toBeGreaterThan(0);
  });

  it("downloads the version being read, and says so in the name", async () => {
    const saved = catchDownloads();
    show();
    vi.spyOn(api, "artifactRevision").mockResolvedValue({
      n: 1,
      reason: "First version",
      author: "claude",
      at: NOW,
      content: { markdown: "- [ ] Passport only" },
    });
    fireEvent.click(await screen.findByRole("button", { name: "Versions (2)" }));
    fireEvent.click(screen.getByText(/Version 1 · First version/));
    await screen.findByRole("checkbox", { name: "Passport only" });
    await openMenu();
    fireEvent.click(screen.getByRole("menuitem", { name: "Download as Markdown" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].name).toBe("Packing list (version 1).md");
    expect(await textOf(saved[0].blob)).toBe("- [ ] Passport only");
  });

  it("offers Word but no Markdown for a plan and a trip", async () => {
    show(
      detail({
        kind: "trip",
        content: { destination: "Thailand", start: "2027-02-01", end: "2027-02-03", days: [] },
      }),
    );
    await openMenu();
    expect(menuItems()).toEqual(["Share…", "Copy", "Download as Word"]);
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
    await openMenu();
    expect(menuItems()).toEqual(["Share…"]);
  });

  it("shows no built-in trip while its page is still being designed", async () => {
    const trip = {
      destination: "Thailand",
      start: "2027-01-03",
      end: "2027-01-04",
      days: [{ date: "2027-01-03", place: "Bangkok", items: [] }],
    };
    show(
      detail({
        kind: "trip",
        status: "working",
        status_text: "Writing your Thailand travel plan…",
        content: trip,
        current_revision: 1,
      }),
    );
    expect(await screen.findByText("Writing your Thailand travel plan…")).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Days" })).not.toBeInTheDocument();
  });

  it("shows a document as it is written", async () => {
    show(detail({ status: "working", status_text: "Writing your packing list…" }));
    expect(await screen.findByText("Writing your packing list…")).toBeInTheDocument();
    expect(screen.getByText("Passport")).toBeInTheDocument();
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

    describe("on a trip", () => {
      const TRIP_PAGED = () =>
        detail({
          kind: "trip",
          content: {
            destination: "Thailand",
            start: "2027-01-03",
            end: "2027-01-04",
            days: [{ date: "2027-01-03", place: "Bangkok", items: [] }],
          },
          page: 3,
          page_html: "<h1>Thailand</h1>",
          today: "2026-10-16",
        });

      it("offers no built-in view and no Remove page beside a page that works", async () => {
        show(TRIP_PAGED());
        expect(await screen.findByTitle("Packing list")).toBeInTheDocument();
        expect(screen.queryByRole("group", { name: "Show as" })).toBeNull();
        expect(screen.queryByRole("button", { name: "Remove page" })).toBeNull();
      });

      it("falls back to the built-in view when the page breaks, and back to the page on a tap", async () => {
        show(TRIP_PAGED());
        await screen.findByTitle("Packing list");
        say({ type: "error", message: "boom" });
        expect(await screen.findByText(/The page could not be shown/)).toBeInTheDocument();
        expect(screen.getByRole("list", { name: "Days" })).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Page" }));
        expect(await screen.findByTitle("Packing list")).toBeInTheDocument();
        expect(screen.queryByRole("group", { name: "Show as" })).toBeNull();
      });
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

describe("ArtifactView, shared", () => {
  afterEach(() => vi.restoreAllMocks());

  const PAGED = { page: 3, page_html: "<h1>A poster of the packing</h1>", today: "2026-10-16" };

  it("shared to read, shows it with nothing to change", async () => {
    const { patch } = show(
      detail({
        access: "view",
        owner: "Mark",
        status: "failed",
        error: "it timed out",
        revisions: [{ n: 2, reason: "Added the charger", author: "claude", by: "Mark", at: NOW }],
      }),
    );
    expect(await screen.findByText("Shared by Mark · read only")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Passport" })).toBeDisabled();
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Versions (1)" }));
    expect(screen.getByText(/^Claude for Mark ·/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox", { name: "Passport" }));
    expect(patch).not.toHaveBeenCalled();
    // Theirs to copy and download, not to share.
    await openMenu();
    expect(menuItems()).toEqual(["Copy", "Download as Word", "Download as Markdown"]);
  });

  it("shared to read, its page cannot save and cannot be taken away", async () => {
    const { patch } = show(detail({ access: "view", owner: "Mark", ...PAGED }));
    const frame = (await screen.findByTitle("Packing list")) as HTMLIFrameElement;
    expect(screen.queryByRole("button", { name: "Remove page" })).toBeNull();
    vi.useFakeTimers();
    try {
      act(() => {
        window.dispatchEvent(
          new MessageEvent("message", {
            data: { niles: 1, type: "save", content: { markdown: "- [x] Passport" }, reason: "x" },
            origin: "null",
            source: frame.contentWindow,
          }),
        );
      });
      act(() => vi.advanceTimersByTime(12_000));
    } finally {
      vi.useRealTimers();
    }
    expect(patch).not.toHaveBeenCalled();
  });

  it("shared to edit, ticks and takes the page away, but is not theirs to share", async () => {
    const { patch } = show(detail({ access: "edit", owner: "Mark", ...PAGED }));
    expect(await screen.findByText("Shared by Mark")).toBeInTheDocument();
    expect(screen.queryByRole("switch")).toBeNull();
    await openMenu();
    expect(menuItems()).not.toContain("Share…");
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "Remove page" }));
    await waitFor(() => expect(patch).toHaveBeenCalledWith(5, { remove_page: true }));
    fireEvent.click(await screen.findByRole("checkbox", { name: "Passport" }));
    await waitFor(() => expect(patch).toHaveBeenCalledTimes(2));
  });

  it("names who else made a version", async () => {
    show(
      detail({
        revisions: [
          { n: 2, reason: "Ticked passport", author: "person", by: "Majse", at: NOW },
          { n: 1, reason: "First version", author: "claude", at: NOW },
        ],
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Versions (2)" }));
    const versions = within(screen.getByRole("list", { name: "Versions" }));
    expect(versions.getByText(/^Majse ·/)).toBeInTheDocument();
    expect(versions.getByText(/^Claude ·/)).toBeInTheDocument();
  });

  it("lets its owner share it with someone, to view or to edit", async () => {
    vi.spyOn(api, "artifactShares").mockResolvedValue({
      people: [
        { person: "person:majse", name: "Majse", level: null },
        { person: "person:gone", name: null, level: "view" },
      ],
    });
    const share = vi.spyOn(api, "shareArtifact").mockResolvedValue({
      people: [
        { person: "person:majse", name: "Majse", level: "edit" },
        { person: "person:gone", name: null, level: "view" },
      ],
    });
    show(detail({ access: "owner", shared_with: 1 }));
    expect(await screen.findByText("Shared with 1")).toBeInTheDocument();
    await openMenu();
    fireEvent.click(screen.getByRole("menuitem", { name: "Share…" }));
    const majse = await screen.findByRole("radiogroup", { name: "Majse" });
    expect(within(majse).getByRole("radio", { name: "None" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    const gone = screen.getByRole("radiogroup", { name: "Someone no longer in the household" });
    expect(within(gone).getAllByRole("radio").map((r) => r.textContent)).toEqual(["None", "View"]);

    fireEvent.click(within(majse).getByRole("radio", { name: "Edit" }));
    await waitFor(() => expect(share).toHaveBeenCalledWith(5, "person:majse", "edit"));
    await waitFor(() =>
      expect(
        within(screen.getByRole("radiogroup", { name: "Majse" })).getByRole("radio", {
          name: "Edit",
        }),
      ).toHaveAttribute("aria-checked", "true"),
    );
    fireEvent.click(within(gone).getByRole("radio", { name: "None" }));
    await waitFor(() => expect(share).toHaveBeenCalledWith(5, "person:gone", null));
  });
});

describe("ArtifactView's pictures", () => {
  afterEach(() => vi.restoreAllMocks());

  const PALACE = {
    id: 12,
    mime: "image/jpeg",
    width: 1600,
    height: 900,
    size: 200_000,
    alt: "The Grand Palace",
    credit: "J. Doe",
    source: null,
    created_at: NOW,
  };

  it("draws a document's kept pictures from Niles", async () => {
    show(detail({ content: { markdown: "![The palace](niles-image:12)" }, images: [PALACE] }));
    const img = await screen.findByRole("img", { name: "The palace" });
    expect(img.getAttribute("src")).toBe("/me/artifacts/5/images/12");
  });

  it("fetches the pictures for its page from Niles, as bytes", async () => {
    const artifactImage = vi.spyOn(api, "artifactImage").mockResolvedValue(new Blob(["jpeg"]));
    show(
      detail({
        page: 3,
        page_html: "<p>poster</p>",
        images: [PALACE],
      }),
    );
    await screen.findByTitle("Packing list");
    await waitFor(() => expect(artifactImage).toHaveBeenCalledWith(5, 12));
  });
});

describe("ArtifactView, podcast", () => {
  afterEach(() => vi.restoreAllMocks());

  const EPISODE = {
    kind: "podcast",
    title: "Why the sky is blue",
    content: {
      topic: "Why the sky is blue",
      minutes: 5,
      hosts: ["Heart", "Michael"],
      lines: [
        { host: "Heart", text: "Why is the sky blue?" },
        { host: "Michael", text: "Sunlight scattering off the air." },
      ],
      sources: [{ title: "Rayleigh scattering", url: "https://example.com/rayleigh" }],
    },
  };

  const AUDIO = {
    url: "https://abc123.r2.cloudflarestorage.com/niles-podcasts/podcasts/5.mp3?X-Amz-Expires=14400",
    download_url:
      "https://abc123.r2.cloudflarestorage.com/niles-podcasts/podcasts/5.mp3?X-Amz-Expires=14400&response-content-disposition=attachment",
    expires_at: NOW,
    kept_until: "2026-10-31T00:00:00Z",
  };

  it("plays a ready episode from the signed address, and shows its script and sources", async () => {
    show(detail(EPISODE));
    vi.spyOn(api, "episodeAudio").mockResolvedValue(AUDIO);
    const player = (await screen.findByLabelText("Play Why the sky is blue")) as HTMLAudioElement;
    expect(player.getAttribute("src")).toBe(AUDIO.url);
    expect(screen.getByText(/Kept until/)).toBeInTheDocument();
    expect(screen.getByText(/play it in the kitchen/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Rayleigh scattering/ })).toHaveAttribute(
      "href",
      "https://example.com/rayleigh",
    );
    expect(screen.queryByText("Heart: Why is the sky blue?")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /^Script/ }));
    expect(screen.getByText(/Why is the sky blue\?/)).toBeInTheDocument();
    expect(screen.getByText(/Sunlight scattering off the air\./)).toBeInTheDocument();
  });

  it("offers the episode from the share menu, by the address the bucket saves", async () => {
    const asked = vi.spyOn(api, "episodeAudio").mockResolvedValue(AUDIO);
    show(detail(EPISODE));
    await screen.findByLabelText("Play Why the sky is blue");
    await openMenu();
    expect(menuItems()).toEqual(["Share…", "Copy", "Download the episode"]);
    const link = screen.getByRole("menuitem", { name: "Download the episode" });
    expect(link).toHaveAttribute("href", AUDIO.download_url);
    expect(link).toHaveAttribute("download", "Why the sky is blue.mp3");
    expect(link).not.toHaveAttribute("target");
    // One address for the player and the menu.
    expect(asked).toHaveBeenCalledTimes(1);
  });

  it("says why a ready episode cannot be played when the add-on is gone", async () => {
    show(detail(EPISODE));
    vi.spyOn(api, "episodeAudio").mockRejectedValue(new Error("Podcasts are not set up."));
    expect(await screen.findByText("Podcasts are not set up.")).toBeInTheDocument();
    expect(screen.queryByLabelText("Play Why the sky is blue")).toBeNull();
  });

  it("has no player while the episode is being recorded", async () => {
    const asked = vi.spyOn(api, "episodeAudio");
    show(
      detail({
        ...EPISODE,
        status: "working" as const,
        status_text: "Recording “Why the sky is blue”…",
        content: null,
      }),
    );
    expect(await screen.findByText("Recording “Why the sky is blue”…")).toBeInTheDocument();
    expect(screen.queryByLabelText("Play Why the sky is blue")).toBeNull();
    expect(asked).not.toHaveBeenCalled();
  });

  it("reads an old version without offering to restore it", async () => {
    show(detail(EPISODE));
    vi.spyOn(api, "episodeAudio").mockResolvedValue(AUDIO);
    vi.spyOn(api, "artifactRevision").mockResolvedValue({
      n: 1,
      reason: "First version",
      author: "niles",
      at: NOW,
      content: EPISODE.content,
    });
    await screen.findByLabelText("Play Why the sky is blue");
    fireEvent.click(screen.getByRole("button", { name: "Versions (2)" }));
    fireEvent.click(screen.getByText(/Version 1 · First version/));
    expect(await screen.findByText("Version 1, as it was")).toBeInTheDocument();
    // The version is read, not restored: a podcast is kept as recorded.
    expect(screen.queryByRole("button", { name: "Restore this version" })).toBeNull();
    // A version is not the recording either.
    expect(screen.queryByLabelText("Play Why the sky is blue")).toBeNull();
    await openMenu();
    expect(menuItems()).not.toContain("Download the episode");
  });
});
