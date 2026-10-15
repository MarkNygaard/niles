import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PluginSurface, choiceItems, failureOf } from "@/components/PluginSurface";
import { shownLabels, yTicks } from "@/components/SurfaceChart";
import { useDeviceStream } from "@/hooks/useDeviceStream";
import { ApiError, api } from "@/lib/api";
import type { Surface, UiNode } from "@/lib/api";

const EVERY_KIND: Surface = {
  nodes: [
    { type: "stack", children: [1, 2, 7, 8, 9] },
    { type: "heading", text: "Electricity", children: [] },
    { type: "row", children: [3, 4] },
    { type: "reading", label: "Now", value: "1.23", unit: "kr/kWh", children: [] },
    { type: "card", title: "Today", children: [5, 6] },
    { type: "badge", label: "Cheap now", tone: "good", children: [] },
    { type: "text", value: "Fetched 2 min ago", muted: true, children: [] },
    {
      type: "items",
      items: [
        { title: "Washing machine", detail: "Starts at 02:00", trailing: "0.91 kr" },
        { title: "Dishwasher", detail: null, trailing: null },
      ],
      children: [],
    },
    {
      type: "chart",
      kind: "bar",
      labels: ["00", "01", "02"],
      series: [{ name: "Price", values: [1.1, 0.9, 1.4] }],
      unit: "kr/kWh",
      children: [],
    },
    { type: "card", title: null, children: [10, 11, 12, 13, 14, 15, 16] },
    { type: "button", id: "refresh", label: "Refresh now", style: "secondary", children: [] },
    { type: "switch", id: "auto", label: "Run cheapest", value: false, children: [] },
    {
      type: "slider",
      id: "hours",
      label: "Cheap hours",
      min: 1,
      max: 12,
      step: 1,
      value: 4,
      unit: "h",
      children: [],
    },
    {
      type: "choice",
      id: "area",
      label: "Area",
      options: [
        { value: "DK1", label: "West" },
        { value: "DK2", label: "East" },
      ],
      value: "DK1",
      children: [],
    },
    {
      type: "text_field",
      id: "note",
      label: "Note",
      placeholder: "Say something",
      value: null,
      children: [],
    },
    { type: "image", src: "https://img.example.com/map.png", alt: "Price map", children: [] },
    { type: "image", src: "data:image/png;base64,iVBORw0KGgo=", alt: "Dot", children: [] },
  ],
};

function client() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

function draw(props: Partial<React.ComponentProps<typeof PluginSurface>> = {}, queries = client()) {
  render(
    <QueryClientProvider client={queries}>
      <PluginSurface plugin="spotprice" surface="electricity" {...props} />
    </QueryClientProvider>,
  );
  return queries;
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("PluginSurface", () => {
  it("draws every kind of node with the app's own components", async () => {
    vi.spyOn(api, "surface").mockResolvedValue(EVERY_KIND);
    draw({ room: "kitchen" });
    expect(await screen.findByRole("heading", { name: "Electricity" })).toBeInTheDocument();
    expect(api.surface).toHaveBeenCalledWith("spotprice", "electricity", "kitchen");
    expect(screen.getByText("1.23")).toBeInTheDocument();
    expect(screen.getByText("kr/kWh")).toBeInTheDocument();
    expect(screen.getByText("Today")).toBeInTheDocument();
    expect(screen.getByText("Cheap now")).toHaveAttribute("data-tone", "good");
    expect(screen.getByText("Fetched 2 min ago")).toHaveClass("text-muted-foreground");
    expect(screen.getByText("Washing machine")).toBeInTheDocument();
    expect(screen.getByText("Starts at 02:00")).toBeInTheDocument();
    expect(screen.getByText("0.91 kr")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: /Bar chart of Price, in kr\/kWh/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Refresh now" })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Run cheapest" })).not.toBeChecked();
    expect(screen.getByRole("slider", { name: "Cheap hours" })).toHaveValue("4");
    expect(screen.getByText("4 h")).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Area" })).toHaveTextContent("West");
    expect(screen.getByRole("textbox", { name: "Note" })).toHaveAttribute(
      "placeholder",
      "Say something",
    );
    // Through Niles, never straight from the plugin's host.
    expect(screen.getByRole("img", { name: "Price map" })).toHaveAttribute(
      "src",
      "/plugins/spotprice/image?src=https%3A%2F%2Fimg.example.com%2Fmap.png",
    );
    expect(screen.getByRole("img", { name: "Dot" })).toHaveAttribute(
      "src",
      "data:image/png;base64,iVBORw0KGgo=",
    );
  });

  it("offers a choice's options in their order", () => {
    const choice = EVERY_KIND.nodes[13] as Extract<UiNode, { type: "choice" }>;
    expect(choiceItems(choice)).toEqual([
      { value: "DK1", label: "West" },
      { value: "DK2", label: "East" },
    ]);
  });

  it("presses a button, shows the toast, and draws again when told to", async () => {
    vi.spyOn(api, "surface").mockResolvedValue(EVERY_KIND);
    const act_ = vi.spyOn(api, "act").mockResolvedValue({ toast: "Prices updated", rerender: true });
    draw({ room: "kitchen" });
    fireEvent.click(await screen.findByRole("button", { name: "Refresh now" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Prices updated");
    expect(act_).toHaveBeenCalledWith(
      "spotprice",
      "electricity",
      "refresh",
      { type: "pressed" },
      "kitchen",
    );
    await waitFor(() => expect(api.surface).toHaveBeenCalledTimes(2));
  });

  it("does not draw again when not told to", async () => {
    vi.spyOn(api, "surface").mockResolvedValue(EVERY_KIND);
    vi.spyOn(api, "act").mockResolvedValue({ toast: null, rerender: false });
    draw();
    fireEvent.click(await screen.findByRole("button", { name: "Refresh now" }));
    await waitFor(() => expect(api.act).toHaveBeenCalled());
    expect(api.surface).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("sends a switch's new value, and puts it back with the error when refused", async () => {
    vi.spyOn(api, "surface").mockResolvedValue(EVERY_KIND);
    const act_ = vi
      .spyOn(api, "act")
      .mockRejectedValue(new ApiError("The pump is offline.", 422));
    draw();
    const toggle = await screen.findByRole("switch", { name: "Run cheapest" });
    fireEvent.click(toggle);
    expect(await screen.findByRole("alert")).toHaveTextContent("The pump is offline.");
    expect(act_).toHaveBeenCalledWith(
      "spotprice",
      "electricity",
      "auto",
      { type: "toggled", value: true },
      undefined,
    );
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "Run cheapest" })).not.toBeChecked(),
    );
  });

  it("submits a text field", async () => {
    vi.spyOn(api, "surface").mockResolvedValue(EVERY_KIND);
    const act_ = vi.spyOn(api, "act").mockResolvedValue({ toast: "Noted", rerender: false });
    draw();
    fireEvent.change(await screen.findByRole("textbox", { name: "Note" }), {
      target: { value: "hello" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send Note" }));
    await waitFor(() =>
      expect(act_).toHaveBeenCalledWith(
        "spotprice",
        "electricity",
        "note",
        { type: "text", value: "hello" },
        undefined,
      ),
    );
  });

  it("shows the plugin's own sentence in place of a page it could not draw", async () => {
    vi.spyOn(api, "surface").mockRejectedValue(new ApiError("No prices yet.", 422));
    draw();
    expect(await screen.findByText("No prices yet.")).toBeInTheDocument();
    expect(screen.queryByText("Not available right now")).toBeNull();
  });

  it("says calmly that a plugin that is down is not available", async () => {
    vi.spyOn(api, "surface").mockRejectedValue(
      new ApiError("Not working: timed out in on-timer after 10 s", 503),
    );
    draw();
    expect(await screen.findByText("Not available right now")).toBeInTheDocument();
    expect(screen.getByText("Not working: timed out in on-timer after 10 s")).toBeInTheDocument();
    expect(failureOf(new ApiError("gone", 404))).toEqual({
      text: "Not available right now",
      detail: null,
    });
  });

  it("asks again every refresh_every seconds while on screen", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.spyOn(api, "surface").mockResolvedValue(EVERY_KIND);
    draw({ refreshEvery: 10 });
    await screen.findByRole("heading", { name: "Electricity" });
    expect(api.surface).toHaveBeenCalledTimes(1);
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    await waitFor(() => expect(api.surface).toHaveBeenCalledTimes(2));
  });
});

/** A WebSocket that only the test speaks through. */
class FakeSocket {
  static last: FakeSocket | null = null;
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor() {
    FakeSocket.last = this;
  }
  close() {}
}

function Page() {
  useDeviceStream();
  return <PluginSurface plugin="spotprice" surface="electricity" />;
}

describe("a ui_invalidated frame", () => {
  it("draws the surface again if it is on screen, and only that surface", async () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.spyOn(api, "surface").mockResolvedValue(EVERY_KIND);
    render(
      <QueryClientProvider client={client()}>
        <Page />
      </QueryClientProvider>,
    );
    await screen.findByRole("heading", { name: "Electricity" });
    expect(api.surface).toHaveBeenCalledTimes(1);

    act(() =>
      FakeSocket.last!.onmessage!({
        data: JSON.stringify({ type: "ui_invalidated", plugin: "spotprice", surface: "other" }),
      }),
    );
    act(() =>
      FakeSocket.last!.onmessage!({
        data: JSON.stringify({ type: "ui_invalidated", plugin: "spotprice", surface: "electricity" }),
      }),
    );
    await waitFor(() => expect(api.surface).toHaveBeenCalledTimes(2));
    vi.unstubAllGlobals();
  });
});

describe("the chart", () => {
  it("has round ticks from zero past the largest value", () => {
    expect(yTicks([1.1, 0.9, 1.4])).toEqual([0, 0.5, 1, 1.5]);
    expect(yTicks([-2, 3])[0]).toBeLessThanOrEqual(-2);
  });

  it("writes only as many labels along the bottom as fit", () => {
    expect(shownLabels(4)).toEqual([0, 1, 2, 3]);
    const shown = shownLabels(24);
    expect(shown[0]).toBe(0);
    expect(shown[shown.length - 1]).toBe(23);
    expect(shown.length).toBeLessThanOrEqual(7);
  });

  it("draws a legend once there are two series", async () => {
    vi.spyOn(api, "surface").mockResolvedValue({
      nodes: [
        {
          type: "chart",
          kind: "line",
          labels: ["a", "b"],
          series: [
            { name: "Today", values: [1, 2] },
            { name: "Tomorrow", values: [2, 1] },
          ],
          unit: null,
          children: [],
        },
      ],
    });
    draw();
    const chart = await screen.findByRole("img", { name: /Line chart of Today, Tomorrow/ });
    const figure = chart.closest("figure")!;
    expect(within(figure).getAllByRole("listitem").map((l) => l.textContent)).toEqual([
      "Today",
      "Tomorrow",
    ]);
    expect(within(figure).getByRole("table")).toBeInTheDocument();
  });
});
