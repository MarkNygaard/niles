import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PluginPageView } from "@/components/PluginPageView";
import { api } from "@/lib/api";
import type { PluginSummary } from "@/lib/api";

vi.mock("@/hooks/useDeviceStream", () => ({ useDeviceStream: () => {} }));

const PLUGIN = {
  id: "spotprice",
  enabled: true,
  menu: [{ id: "electricity", label: "Electricity", icon: "zap", refresh_every: null }],
} as unknown as PluginSummary;

function draw(plugin: string, page: string) {
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <PluginPageView plugin={plugin} page={page} />
    </QueryClientProvider>,
  );
}

afterEach(() => vi.restoreAllMocks());

describe("PluginPageView", () => {
  it("draws the page the route names", async () => {
    vi.spyOn(api, "plugins").mockResolvedValue([PLUGIN]);
    vi.spyOn(api, "surface").mockResolvedValue({
      nodes: [{ type: "heading", text: "Prices today", children: [] }],
    });
    draw("spotprice", "electricity");
    expect(await screen.findByRole("heading", { name: "Prices today" })).toBeInTheDocument();
    expect(api.surface).toHaveBeenCalledWith("spotprice", "electricity", undefined);
  });

  it("says calmly that a page whose plugin is off or gone is not available", async () => {
    vi.spyOn(api, "plugins").mockResolvedValue([{ ...PLUGIN, enabled: false }]);
    const surface = vi.spyOn(api, "surface");
    draw("spotprice", "electricity");
    expect(await screen.findByText("Not available right now")).toBeInTheDocument();
    expect(surface).not.toHaveBeenCalled();
  });
});
