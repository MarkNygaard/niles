import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { WebSearchPanel, searxngAddress, statusLine, testLines } from "./WebSearchPanel";
import { api } from "@/lib/api";
import type { WebSearchTest } from "@/lib/api";

const WORKING: WebSearchTest = {
  configured: true,
  web: { ok: true, results: 8, error: null },
  images: { ok: true, results: 5, error: null },
};

const NO_PICTURES: WebSearchTest = {
  ...WORKING,
  images: { ok: true, results: 0, error: null },
};

const UNREACHABLE: WebSearchTest = {
  configured: true,
  web: { ok: false, results: 0, error: "Nothing answered at that address." },
  images: { ok: false, results: 0, error: "Nothing answered at that address." },
};

afterEach(() => {
  vi.restoreAllMocks();
});

describe("searxngAddress", () => {
  it("adds the search path to a bare host, and a scheme when none was typed", () => {
    expect(searxngAddress("search.example.com")).toBe("https://search.example.com/search");
    expect(searxngAddress(" https://search.example.com/ ")).toBe(
      "https://search.example.com/search",
    );
    expect(searxngAddress("http://searxng.searxng.svc:8080")).toBe(
      "http://searxng.searxng.svc:8080/search",
    );
  });

  it("keeps an address that already names its path", () => {
    expect(searxngAddress("https://example.com/searx/search")).toBe(
      "https://example.com/searx/search",
    );
    expect(searxngAddress("")).toBe("");
  });
});

describe("statusLine", () => {
  it("is not added without an address", () => {
    expect(statusLine("", { state: "idle" })).toEqual({ tone: "off", text: "Not added." });
  });

  it("is checking until the test answers", () => {
    expect(statusLine("https://s.example/search", { state: "running" }).text).toBe("Checking…");
  });

  it("is in use once a web search answered", () => {
    const status = statusLine("https://s.example/search", { state: "done", result: WORKING });
    expect(status.tone).toBe("ok");
    expect(status.text).toMatch(/^In use/);
  });

  it("is not reachable, and says why, when it did not answer", () => {
    expect(
      statusLine("https://s.example/search", { state: "done", result: UNREACHABLE }),
    ).toEqual({ tone: "bad", text: "Not reachable: Nothing answered at that address." });
    expect(
      statusLine("https://s.example/search", { state: "failed", message: "request failed (502)" })
        .tone,
    ).toBe("bad");
  });
});

describe("testLines", () => {
  it("says how many of each it found", () => {
    expect(testLines(WORKING)).toEqual(["Web: 8 results.", "Pictures: 5 results."]);
  });

  it("says plainly that pictures need image engines when there were none", () => {
    const [, pictures] = testLines(NO_PICTURES);
    expect(pictures).toMatch(/^Pictures: none\./);
    expect(pictures).toMatch(/no image engines/);
    expect(pictures).toMatch(/pictures for artifacts need them/);
  });

  it("gives the reason for one that failed", () => {
    expect(testLines(UNREACHABLE)[0]).toBe("Web: Nothing answered at that address.");
  });
});

describe("WebSearchPanel", () => {
  it("says what happens without it, and tests nothing", () => {
    const tested = vi.spyOn(api, "testWebSearch");
    render(<WebSearchPanel address="" onAddress={vi.fn()} />);
    expect(screen.getByText(/Claude searches the web with its own built-in search/)).toBeTruthy();
    expect(screen.getByText(/ChatGPT can't search the web without SearXNG/)).toBeTruthy();
    expect(screen.getByText(/Pictures for artifacts need SearXNG/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Test" })).toBeNull();
    expect(tested).not.toHaveBeenCalled();
  });

  it("saves the address as SearXNG answers at it", () => {
    const onAddress = vi.fn();
    render(<WebSearchPanel address="" onAddress={onAddress} />);
    const field = screen.getByLabelText("SearXNG address");
    fireEvent.change(field, { target: { value: "search.example.com" } });
    fireEvent.blur(field);
    expect(onAddress).toHaveBeenCalledWith("https://search.example.com/search");
  });

  it("tests an added one as it opens, and shows what each search found", async () => {
    vi.spyOn(api, "testWebSearch").mockResolvedValue(NO_PICTURES);
    render(<WebSearchPanel address="https://s.example/search" onAddress={vi.fn()} />);
    expect(await screen.findByText(/^In use/)).toBeTruthy();
    expect(screen.getByText("Web: 8 results.")).toBeTruthy();
    expect(screen.getByText(/^Pictures: none\./)).toBeTruthy();
  });

  it("says when it is not reachable, and tests again on the button", async () => {
    const tested = vi
      .spyOn(api, "testWebSearch")
      .mockResolvedValueOnce(UNREACHABLE)
      .mockResolvedValueOnce(WORKING);
    render(<WebSearchPanel address="https://s.example/search" onAddress={vi.fn()} />);
    expect(await screen.findByText(/^Not reachable: Nothing answered/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Test" }));
    expect(await screen.findByText(/^In use/)).toBeTruthy();
    expect(screen.getByText("Pictures: 5 results.")).toBeTruthy();
    expect(tested).toHaveBeenCalledTimes(2);
  });

  it("says the request failed when the server could not be asked", async () => {
    vi.spyOn(api, "testWebSearch").mockRejectedValue(new Error("request failed (502)"));
    render(<WebSearchPanel address="https://s.example/search" onAddress={vi.fn()} />);
    await waitFor(() =>
      expect(screen.getByRole("status").textContent).toBe("Not reachable: request failed (502)"),
    );
  });
});
