import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { McpServersPanel, signInReturn } from "@/components/McpServersPanel";
import { api } from "@/lib/api";
import type { PersonalServer } from "@/lib/api";

const KEY = "notion-secret-key";

const NOTION: PersonalServer = {
  id: 1,
  slug: "notion",
  name: "Notion",
  url: "https://mcp.notion.com/mcp",
  sign_in: "key",
  header: null,
  modes: {},
  signed_in: true,
  status: "working",
  message: null,
  tools: [
    {
      name: "search",
      title: "Search",
      description: "",
      read_only: true,
      destructive: false,
      mode: "on",
    },
    {
      name: "create_page",
      title: null,
      description: "",
      read_only: false,
      destructive: false,
      mode: "ask",
    },
  ],
};

const LINEAR: PersonalServer = {
  ...NOTION,
  id: 2,
  slug: "linear",
  name: "Linear",
  url: "https://mcp.linear.app/mcp",
  sign_in: "browser",
  signed_in: false,
  status: "needs_sign_in",
  tools: [],
};

function renderPanel(returned: "done" | "failed" | null = null) {
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <McpServersPanel returned={returned} />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("McpServersPanel", () => {
  it("lists each server with its state and each tool with its setting", async () => {
    vi.spyOn(api, "myServers").mockResolvedValue([NOTION, LINEAR]);
    renderPanel();

    const list = await screen.findByRole("list", { name: "Your MCP servers" });
    expect(list).toHaveTextContent("Notion");
    expect(list).toHaveTextContent("Connected");
    expect(list).toHaveTextContent("Sign in needed");
    const search = screen.getByRole("group", { name: "search setting" });
    expect(search.querySelector('[aria-pressed="true"]')).toHaveTextContent("On");
    const create = screen.getByRole("group", { name: "create_page setting" });
    expect(create.querySelector('[aria-pressed="true"]')).toHaveTextContent("Ask");
    expect(screen.getByRole("button", { name: "Sign in to Linear" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sign in to Notion" })).toBeNull();
  });

  it("sets a tool's setting", async () => {
    vi.spyOn(api, "myServers").mockResolvedValue([NOTION]);
    const set = vi.spyOn(api, "setMyServerMode").mockResolvedValue(undefined);
    renderPanel();

    const create = await screen.findByRole("group", { name: "create_page setting" });
    fireEvent.click(create.querySelector("button:first-child")!);
    await waitFor(() => expect(set).toHaveBeenCalledWith(1, "create_page", "on"));
  });

  it("adds a server with a key, and never shows the key again", async () => {
    vi.spyOn(api, "myServers").mockResolvedValue([]);
    const add = vi.spyOn(api, "addMyServer").mockResolvedValue(NOTION);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Add a server" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Notion" } });
    fireEvent.change(screen.getByLabelText("Address"), {
      target: { value: "https://mcp.notion.com/mcp" },
    });
    fireEvent.change(screen.getByLabelText("Key"), { target: { value: KEY } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add.mock.calls[0][0]).toEqual({
      name: "Notion",
      url: "https://mcp.notion.com/mcp",
      sign_in: "key",
      header: "",
      key: KEY,
    });
    await waitFor(() => expect(screen.queryByDisplayValue(KEY)).toBeNull());
  });

  it("sends the browser to the server's own page to sign in", async () => {
    vi.spyOn(api, "myServers").mockResolvedValue([LINEAR]);
    vi.spyOn(api, "signInMyServer").mockResolvedValue({
      url: "https://linear.app/oauth/authorize?state=x",
    });
    const assign = vi.fn();
    vi.spyOn(window, "location", "get").mockReturnValue({
      ...window.location,
      assign,
    } as Location);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Sign in to Linear" }));
    await waitFor(() =>
      expect(assign).toHaveBeenCalledWith("https://linear.app/oauth/authorize?state=x"),
    );
  });

  it("says how a sign-in came back", async () => {
    vi.spyOn(api, "myServers").mockResolvedValue([]);
    renderPanel("failed");
    expect(await screen.findByText("The sign-in didn't finish. Try again.")).toBeInTheDocument();
  });
});

describe("signInReturn", () => {
  it("reads the return a sign-in left in the address", () => {
    expect(signInReturn(new URLSearchParams("signed_in=3"))).toBe("done");
    expect(signInReturn(new URLSearchParams("sign_in=failed"))).toBe("failed");
    expect(signInReturn(new URLSearchParams(""))).toBeNull();
  });
});
