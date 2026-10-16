import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { IMAGES_WITHIN_MS, READY_WITHIN_MS, SandboxedPage } from "@/components/SandboxedPage";
import type { SandboxedPageProps } from "@/components/SandboxedPage";
import { PAGE_CSP, SAVE_AFTER_MS } from "@/lib/sandboxedPage";

const PAGE = "<h1 id=t></h1><script>niles.onData(function (d) { t.textContent = d.markdown })</script>";

function show(props: Partial<SandboxedPageProps> = {}) {
  const onFallback = vi.fn();
  const onSave = vi.fn();
  const all: SandboxedPageProps = {
    html: PAGE,
    content: { markdown: "# Packing" },
    today: "2026-10-16",
    title: "Packing list",
    onSave,
    onFallback,
    ...props,
  };
  const view = render(<SandboxedPage {...all} />);
  const frame = screen.getByTitle("Packing list") as HTMLIFrameElement;
  const say = (data: Record<string, unknown>, source: unknown = frame.contentWindow) =>
    act(() => {
      window.dispatchEvent(
        new MessageEvent("message", {
          data: { niles: 1, ...data },
          origin: "null",
          source: source as Window,
        }),
      );
    });
  return { frame, say, onFallback, onSave, rerender: (p: Partial<SandboxedPageProps>) =>
    view.rerender(<SandboxedPage {...all} {...p} />) };
}

describe("SandboxedPage", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("is a frame that may run scripts and nothing else", () => {
    const { frame } = show();
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(frame.getAttribute("allow")).toContain("camera 'none'");
    expect(frame.hasAttribute("src")).toBe(false);
    const doc = frame.getAttribute("srcdoc") ?? "";
    expect(doc.startsWith("<!doctype html><html><head><meta charset=\"utf-8\">")).toBe(true);
    expect(doc).toContain(`content="${PAGE_CSP}"`);
    expect(doc).toContain("niles.onData(function (d)");
  });

  it("takes its height from the page, and ignores a stranger's word", () => {
    const { frame, say } = show();
    say({ type: "height", height: 512 });
    expect(frame.style.height).toBe("512px");
    say({ type: "height", height: 64 }, null);
    say({ type: "height", height: 64 }, window);
    expect(frame.style.height).toBe("512px");
  });

  it("opens a link only once the person has seen where it goes and said yes", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const { say } = show();
    say({ type: "open", url: "http://www.jayfai.com/" });
    say({ type: "open", url: "javascript:alert(1)" });
    expect(screen.queryByText("Open a link from this page?")).toBeNull();

    say({ type: "open", url: "https://www.jayfai.com/menu" });
    expect(screen.getByText("Open a link from this page?")).toBeInTheDocument();
    expect(screen.getByText("www.jayfai.com")).toBeInTheDocument();
    expect(open).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(open).toHaveBeenCalledWith("https://www.jayfai.com/menu", "_blank", "noopener,noreferrer");
  });

  it("opens nothing when the person cancels", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const { say } = show();
    say({ type: "open", url: "https://leak.example/?plan=secret" });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(open).not.toHaveBeenCalled();
  });

  it("asks for the built-in view when the page breaks", () => {
    const { say, onFallback } = show();
    say({ type: "ready" });
    say({ type: "error", message: "d.weeks is undefined" });
    expect(onFallback).toHaveBeenCalledWith("it stopped with an error (d.weeks is undefined)");
  });

  it("asks for the built-in view when the page never says it is ready", () => {
    vi.useFakeTimers();
    const { onFallback } = show();
    act(() => vi.advanceTimersByTime(READY_WITHIN_MS));
    expect(onFallback).toHaveBeenCalledWith("it did not start");
  });

  it("does not when it does", () => {
    vi.useFakeTimers();
    const { say, onFallback } = show();
    say({ type: "ready" });
    act(() => vi.advanceTimersByTime(READY_WITHIN_MS * 2));
    expect(onFallback).not.toHaveBeenCalled();
  });

  it("sends new data in, and only data and the theme", () => {
    const { frame, say, rerender } = show();
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    say({ type: "ready" });
    rerender({ content: { markdown: "# Marked" } });
    const sent = post.mock.calls.map(([m, target]) => [(m as { type: string }).type, target]);
    expect(sent).toContainEqual(["data", "*"]);
    expect(post.mock.calls.find(([m]) => (m as { type: string }).type === "data")?.[0]).toEqual({
      niles: 1,
      type: "data",
      data: { markdown: "# Marked" },
      today: "2026-10-16",
    });
    expect(sent.every(([type]) => type === "data" || type === "theme")).toBe(true);
  });

  it("saves what the page saves, batched, as the person", () => {
    vi.useFakeTimers();
    const { say, onSave } = show();
    say({ type: "save", content: { markdown: "- [x] passport" }, reason: "Packed passport" });
    say({ type: "save", content: { markdown: "- [x] passport\n- [x] charger" }, reason: "Packed charger" });
    expect(onSave).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(SAVE_AFTER_MS));
    expect(onSave).toHaveBeenCalledExactlyOnceWith(
      { markdown: "- [x] passport\n- [x] charger" },
      "On its page: Packed charger (and 1 more)",
    );
  });

  it("ignores a save that is not content", () => {
    vi.useFakeTimers();
    const { say, onSave } = show();
    say({ type: "save", content: "rm -rf", reason: "" });
    say({ type: "save", content: { a: 1 } }, null);
    act(() => vi.advanceTimersByTime(SAVE_AFTER_MS * 2));
    expect(onSave).not.toHaveBeenCalled();
  });
  const PALACE = { id: 12, alt: "The Grand Palace", credit: "J. Doe", source: null, width: 16, height: 9 };

  it("hands the page its pictures, fetched from Niles, once it is listening", async () => {
    const blob = new Blob(["jpeg"], { type: "image/jpeg" });
    const loadImage = vi.fn().mockResolvedValue(blob);
    const { frame, say } = show({ images: [PALACE], loadImage });
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    await act(async () => {});
    expect(loadImage).toHaveBeenCalledExactlyOnceWith(12);
    expect(frame.getAttribute("srcdoc")).toContain('"alt":"The Grand Palace"');
    expect(post).not.toHaveBeenCalled();
    say({ type: "hello" });
    expect(post).toHaveBeenCalledExactlyOnceWith(
      { niles: 1, type: "images", images: { "12": blob }, info: [PALACE] },
      "*",
    );
  });

  it("hands the pictures that came, and waits for them before counting to the fallback", async () => {
    vi.useFakeTimers();
    const blob = new Blob(["png"]);
    const loadImage = vi.fn((id: number) =>
      id === 12 ? Promise.resolve(blob) : new Promise<Blob>(() => {}),
    );
    const { frame, say, onFallback } = show({
      images: [PALACE, { ...PALACE, id: 13 }],
      loadImage,
    });
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    say({ type: "hello" });
    await act(() => vi.advanceTimersByTimeAsync(READY_WITHIN_MS + 10));
    expect(onFallback).not.toHaveBeenCalled();
    await act(() => vi.advanceTimersByTimeAsync(IMAGES_WITHIN_MS));
    expect(post.mock.calls[0][0]).toMatchObject({ type: "images", images: { "12": blob } });
    await act(() => vi.advanceTimersByTimeAsync(READY_WITHIN_MS));
    expect(onFallback).toHaveBeenCalledWith("it did not start");
  });
});
