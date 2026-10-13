import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryCard } from "./MemoryCard";
import { ApiError, api } from "@/lib/api";
import type { Memory } from "@/lib/api";

const HOUSE: Memory = {
  user: ["Mark takes his coffee black", "Anna is vegetarian"],
  agent: ["The hallway lamp is slow to respond"],
};

function show(memory: Memory) {
  vi.spyOn(api, "memory").mockResolvedValue(memory);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <MemoryCard />
    </QueryClientProvider>,
  );
}

async function pressDelete(group: string, text: string) {
  const list = await screen.findByRole("list", { name: group });
  const row = within(list).getByText(text).closest("li")!;
  fireEvent.click(within(row).getByRole("button", { name: "Delete" }));
  return screen.findByRole("dialog");
}

describe("MemoryCard", () => {
  afterEach(() => vi.restoreAllMocks());

  it("shows what Niles knows about the household apart from what it noted itself", async () => {
    show(HOUSE);
    const household = await screen.findByRole("list", { name: "About the household" });
    expect(within(household).getByText("Mark takes his coffee black")).toBeInTheDocument();
    expect(within(household).getByText("Anna is vegetarian")).toBeInTheDocument();
    expect(within(household).queryByText("The hallway lamp is slow to respond")).toBeNull();

    const noted = screen.getByRole("list", { name: "What Niles has noted" });
    expect(within(noted).getByText("The hallway lamp is slow to respond")).toBeInTheDocument();
    expect(screen.getByText(/reads them before it answers/)).toBeInTheDocument();
  });

  it("says so when a group has nothing in it", async () => {
    show({ user: [], agent: ["The hallway lamp is slow to respond"] });
    expect(
      await screen.findByText(/Niles keeps what you tell it about yourselves here/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "About the household" })).toBeNull();
    expect(screen.queryByText(/Niles notes here what it learns/)).toBeNull();
  });

  it("says so for both when there is nothing at all", async () => {
    show({ user: [], agent: [] });
    expect(
      await screen.findByText(/Niles keeps what you tell it about yourselves here/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Niles notes here what it learns/)).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("deletes the exact entry from the right group once it is confirmed", async () => {
    show(HOUSE);
    const forget = vi.spyOn(api, "forgetMemory").mockResolvedValue(undefined);
    const dialog = await pressDelete("What Niles has noted", "The hallway lamp is slow to respond");
    expect(forget).not.toHaveBeenCalled();
    expect(within(dialog).getByText("The hallway lamp is slow to respond")).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(forget).toHaveBeenCalledTimes(1));
    expect(forget.mock.calls[0][0]).toBe("agent");
    expect(forget.mock.calls[0][1]).toBe("The hallway lamp is slow to respond");
    // Asked again, so the row goes.
    await waitFor(() => expect(api.memory).toHaveBeenCalledTimes(2));
  });

  it("keeps the entry when the confirmation is declined", async () => {
    show(HOUSE);
    const forget = vi.spyOn(api, "forgetMemory").mockResolvedValue(undefined);
    const dialog = await pressDelete("About the household", "Anna is vegetarian");
    fireEvent.click(within(dialog).getByRole("button", { name: "Keep it" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(forget).not.toHaveBeenCalled();
  });

  it("shows why a delete was refused", async () => {
    show(HOUSE);
    vi.spyOn(api, "forgetMemory").mockRejectedValue(
      new ApiError("no entry in user memory matches the given text", 404),
    );
    const dialog = await pressDelete("About the household", "Mark takes his coffee black");
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(await within(dialog).findByText("no entry in user memory matches the given text")).toBeInTheDocument();
  });

  it("says why when the instance keeps no memory", async () => {
    vi.spyOn(api, "memory").mockRejectedValue(
      new ApiError("this Niles instance has no memory store", 501),
    );
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <MemoryCard />
      </QueryClientProvider>,
    );
    expect(await screen.findByText("this Niles instance has no memory store")).toBeInTheDocument();
  });
});
