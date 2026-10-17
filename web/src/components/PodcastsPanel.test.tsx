import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PodcastsPanel } from "./PodcastsPanel";
import { api } from "@/lib/api";
import type { PodcastsTest } from "@/lib/api";

const WORKING: PodcastsTest = {
  configured: true,
  missing: [],
  kokoro: { ok: true, error: null },
  storage: { ok: true, error: null },
};

const SET = {
  kokoro_url: "http://kokoro.ns.svc:8880",
  endpoint: "https://abc123.r2.cloudflarestorage.com",
  bucket: "niles-podcasts",
};

function show(settings: object | undefined = SET, extra: Record<string, unknown> = {}) {
  const onChange = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <PodcastsPanel
        settings={settings}
        writable
        onChange={onChange}
        onSecretsChanged={vi.fn()}
        {...extra}
      />
    </QueryClientProvider>,
  );
  return { onChange };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("PodcastsPanel", () => {
  it("tests as it opens, and says Working when both parts pass", async () => {
    const tested = vi.spyOn(api, "testPodcasts").mockResolvedValue(WORKING);
    vi.spyOn(api, "podcastVoices").mockResolvedValue({ voices: ["af_heart"], error: null });
    show();
    expect(await screen.findByText("Working")).toBeTruthy();
    expect(screen.getByText("Kokoro: OK.")).toBeTruthy();
    expect(screen.getByText("Storage: OK.")).toBeTruthy();
    expect(tested).toHaveBeenCalledTimes(1);
  });

  it("says Not working, and names the storage part, when it failed", async () => {
    vi.spyOn(api, "testPodcasts").mockResolvedValue({
      configured: true,
      missing: [],
      kokoro: { ok: true, error: null },
      storage: { ok: false, error: "Uploading a test file: the bucket refused the key" },
    });
    show();
    expect(await screen.findByText("Not working")).toBeTruthy();
    expect(
      screen.getByText("Storage: Uploading a test file: the bucket refused the key"),
    ).toBeTruthy();
  });

  it("says what is missing when it is not set up enough to test", async () => {
    vi.spyOn(api, "testPodcasts").mockResolvedValue({
      configured: false,
      missing: ["R2 endpoint", "bucket"],
      kokoro: null,
      storage: null,
    });
    show();
    expect(await screen.findByText("Needs its R2 endpoint, bucket")).toBeTruthy();
    expect(screen.queryByText("Kokoro: OK.")).toBeNull();
  });

  it("tests nothing on open while fields are missing, and on the button again", async () => {
    const tested = vi.spyOn(api, "testPodcasts").mockResolvedValue(WORKING);
    show({ kokoro_url: "http://kokoro.ns.svc:8880" });
    expect(tested).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Test" }));
    expect(await screen.findByText("Working")).toBeTruthy();
    expect(tested).toHaveBeenCalledTimes(1);
  });

  it("saves the Kokoro address as Kokoro answers at it", () => {
    const { onChange } = show();
    const field = screen.getByLabelText("Kokoro address");
    fireEvent.change(field, { target: { value: "kokoro.ns.svc:8880/v1/" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([
      { path: "integrations.podcasts.kokoro_url", value: "http://kokoro.ns.svc:8880" },
    ]);
  });

  it("saves the R2 endpoint without a trailing slash", () => {
    const { onChange } = show();
    const field = screen.getByLabelText("R2 endpoint");
    fireEvent.change(field, { target: { value: "abc123.r2.cloudflarestorage.com/" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([
      { path: "integrations.podcasts.endpoint", value: "https://abc123.r2.cloudflarestorage.com" },
    ]);
  });

  it("saves the bucket lowercased, and the region as typed", () => {
    const { onChange } = show();
    const bucket = screen.getByLabelText("Bucket");
    fireEvent.change(bucket, { target: { value: " Niles-Podcasts " } });
    fireEvent.blur(bucket);
    expect(onChange).toHaveBeenCalledWith([
      { path: "integrations.podcasts.bucket", value: "niles-podcasts" },
    ]);

    const region = screen.getByLabelText("Region");
    fireEvent.change(region, { target: { value: "us-east-1" } });
    fireEvent.blur(region);
    expect(onChange).toHaveBeenCalledWith([
      { path: "integrations.podcasts.region", value: "us-east-1" },
    ]);
  });

  it("shows the region as auto by default", () => {
    show();
    expect(screen.getByLabelText("Region")).toHaveValue("auto");
  });

  it("commits the days as a number", () => {
    const { onChange } = show();
    const field = screen.getByLabelText("Keep episodes for (days)");
    fireEvent.change(field, { target: { value: "30" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([
      { path: "integrations.podcasts.keep_days", value: 30 },
    ]);
  });

  it("refuses 0 days, says why, and saves nothing", () => {
    const { onChange } = show();
    const field = screen.getByLabelText("Keep episodes for (days)");
    fireEvent.change(field, { target: { value: "0" } });
    fireEvent.blur(field);
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByText("A number of days from 1 to 365.")).toBeTruthy();
  });

  it("shows the voice pickers once Kokoro has an address, from its voice list", async () => {
    vi.spyOn(api, "testPodcasts").mockResolvedValue(WORKING);
    const listed = vi
      .spyOn(api, "podcastVoices")
      .mockResolvedValue({ voices: ["af_heart", "am_michael", "bf_emma"], error: null });
    show();
    const first = await screen.findByRole("combobox", { name: "First host" });
    const second = screen.getByRole("combobox", { name: "Second host" });
    expect(first).toHaveTextContent("Heart · American woman");
    expect(second).toHaveTextContent("Michael · American man");
    expect(listed).toHaveBeenCalledWith();
  });

  it("has no voice pickers without a Kokoro address", () => {
    show({ bucket: "niles-podcasts" });
    expect(screen.queryByRole("combobox", { name: "First host" })).toBeNull();
    expect(screen.queryByRole("combobox", { name: "Second host" })).toBeNull();
  });

  it("shows the voices' error under the pickers", async () => {
    vi.spyOn(api, "testPodcasts").mockResolvedValue(WORKING);
    vi.spyOn(api, "podcastVoices").mockResolvedValue({
      voices: [],
      error: "Kokoro did not answer within 10s.",
    });
    show();
    expect(await screen.findByText("Kokoro did not answer within 10s.")).toBeTruthy();
  });

  it("says when the test itself could not be asked", async () => {
    vi.spyOn(api, "testPodcasts").mockRejectedValue(new Error("request failed (502)"));
    show();
    await waitFor(() =>
      expect(screen.getByRole("status").textContent).toBe("Not reachable: request failed (502)"),
    );
  });
});
