import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { SavedList } from "@/components/SavedList";
import type { SavedListProps } from "@/components/SavedList";
import type { Artifact } from "@/lib/api";

function artifact(id: number, title: string, rest: Partial<Artifact> = {}): Artifact {
  return {
    id,
    kind: "document",
    title,
    summary: "",
    pinned: false,
    archived: false,
    notify: true,
    status: "ready",
    status_text: null,
    error: null,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    current_revision: 1,
    preview: { lines: [] },
    ...rest,
  };
}

const PACKING = artifact(1, "Packing list", { summary: "For the cabin" });
const THAILAND = artifact(2, "Thailand travel plan", {
  pinned: true,
  status: "working",
  status_text: "Writing your Thailand travel plan…",
});
const FAILED = artifact(3, "Wine list", { status: "failed", error: "it timed out" });
const OLD = artifact(4, "Old notes", { archived: true });

function show(props: Partial<SavedListProps> = {}) {
  const handlers = {
    onOpen: vi.fn(),
    onChange: vi.fn().mockResolvedValue(undefined),
    onDelete: vi.fn().mockResolvedValue(undefined),
  };
  render(
    <SavedList
      artifacts={[PACKING, THAILAND, FAILED, OLD]}
      assistant="claude"
      {...handlers}
      {...props}
    />,
  );
  return handlers;
}

const rows = () => within(screen.getByRole("list", { name: "Saved" })).getAllByRole("listitem");

function more(title: string) {
  fireEvent.click(screen.getByRole("button", { name: `More for ${title}` }));
}

describe("SavedList", () => {
  it("lists pinned first, with each one's state, and the archived folded away", () => {
    show();
    const [first, second, third] = rows();
    expect(within(first).getByText("Thailand travel plan")).toBeInTheDocument();
    expect(within(first).getByRole("img", { name: "Pinned" })).toBeInTheDocument();
    expect(within(first).getByText("being written…")).toBeInTheDocument();
    expect(within(second).getByText("For the cabin")).toBeInTheDocument();
    expect(within(second).getByText("just now")).toBeInTheDocument();
    expect(within(third).getByText("didn't finish")).toBeInTheDocument();
    expect(rows()).toHaveLength(3);

    expect(screen.queryByText("Old notes")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Archived (1)" }));
    expect(
      within(screen.getByRole("list", { name: "Archived" })).getByText("Old notes"),
    ).toBeInTheDocument();
  });

  it("opens one", () => {
    const { onOpen } = show();
    fireEvent.click(screen.getByText("Packing list"));
    expect(onOpen).toHaveBeenCalledWith(1);
  });

  it("pins and unpins", async () => {
    const { onChange } = show();
    more("Packing list");
    fireEvent.click(screen.getByRole("button", { name: "Pin" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(1, { pinned: true }));
    more("Thailand travel plan");
    fireEvent.click(screen.getByRole("button", { name: "Unpin" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(2, { pinned: false }));
  });

  it("archives and unarchives", async () => {
    const { onChange } = show();
    more("Packing list");
    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(1, { archived: true }));

    fireEvent.click(screen.getByRole("button", { name: "Archived (1)" }));
    more("Old notes");
    fireEvent.click(screen.getByRole("button", { name: "Unarchive" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(4, { archived: false }));
  });

  it("renames", async () => {
    const { onChange } = show();
    more("Packing list");
    fireEvent.click(screen.getByRole("button", { name: "Rename" }));
    const field = screen.getByRole("textbox", { name: "Title" });
    expect(field).toHaveValue("Packing list");
    fireEvent.change(field, { target: { value: "  Cabin packing " } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(1, { title: "Cabin packing" }));
    expect(await screen.findByRole("list", { name: "Saved" })).toBeInTheDocument();
  });

  it("asks before deleting, and keeps it when called off", async () => {
    const { onDelete } = show();
    more("Packing list");
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByText("Delete this?")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Keep it" }));
    expect(onDelete).not.toHaveBeenCalled();

    more("Packing list");
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(onDelete).toHaveBeenCalledWith(1));
  });

  it("says who makes these, and where to switch one on, with no assistant", () => {
    show({ artifacts: [], assistant: null });
    expect(screen.getByText(/Claude or ChatGPT makes these/)).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Switch one on in Settings → Voice & AI" }),
    ).toHaveAttribute("href", "#/me/settings/voice");
    expect(screen.queryByText(/Nothing saved yet/)).toBeNull();
  });

  it("says what goes here when there is nothing yet", () => {
    show({ artifacts: [] });
    expect(screen.getByText(/Nothing saved yet/)).toBeInTheDocument();
    expect(screen.queryByText(/Claude or ChatGPT makes these/)).toBeNull();
  });
});
