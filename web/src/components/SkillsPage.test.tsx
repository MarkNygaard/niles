import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SkillsPage } from "./SkillsPage";
import { ApiError, api } from "@/lib/api";
import type { Skill, SkillDetail } from "@/lib/api";

const windDown: Skill = {
  name: "evening-wind-down",
  description: "Dim the house in steps before bed.",
  version: "0.1.0",
  status: "active",
  provenance: "agent-created",
  pinned: false,
  created_at: "2026-10-01T19:00:00Z",
  last_used_at: null,
  last_viewed_at: null,
  last_patched_at: null,
  usage_count: 0,
  view_count: 0,
  patch_count: 0,
};

const coffee: Skill = {
  ...windDown,
  name: "morning-coffee",
  description: "Start the kettle when the alarm goes.",
  status: "stale",
  provenance: "user-created",
  pinned: true,
};

const oldParty: Skill = {
  ...windDown,
  name: "party-lights",
  description: "Colour the living room for a party.",
  status: "archived",
};

const BODY = "## Steps\n\n1. Dim the living room to 40%.\n2. Turn off the kitchen.";

function detail(skill: Skill): SkillDetail {
  return { ...skill, body: BODY };
}

function show(skills: Skill[]) {
  vi.spyOn(api, "skills").mockResolvedValue(skills);
  vi.spyOn(api, "skill").mockImplementation(async (name) =>
    detail(skills.find((s) => s.name === name)!),
  );
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <SkillsPage />
    </QueryClientProvider>,
  );
}

async function open(title: string) {
  fireEvent.click(await screen.findByRole("button", { name: new RegExp(title) }));
  return screen.findByRole("dialog");
}

describe("SkillsPage", () => {
  afterEach(() => vi.restoreAllMocks());

  it("lists skills by their humanised name, with the raw name beside it", async () => {
    show([windDown]);
    expect(await screen.findByText("Evening wind down")).toBeInTheDocument();
    expect(screen.getByText("evening-wind-down")).toBeInTheDocument();
    expect(screen.getByText("Dim the house in steps before bed.")).toBeInTheDocument();
    expect(screen.getByText("Never used")).toBeInTheDocument();
  });

  it("keeps archived skills in their own section, collapsed", async () => {
    show([windDown, oldParty]);
    expect(await screen.findByText("Evening wind down")).toBeInTheDocument();
    expect(screen.queryByText("Party lights")).toBeNull();

    const toggle = screen.getByRole("button", { name: /Archived \(1\)/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");

    const archived = screen.getByRole("list", { name: "Archived skills" });
    expect(within(archived).getByText("Party lights")).toBeInTheDocument();
    expect(within(archived).queryByText("Evening wind down")).toBeNull();
  });

  it("badges status, who made it, and pinned", async () => {
    show([windDown, coffee]);
    expect(await screen.findByText("Active")).toBeInTheDocument();
    expect(screen.getByText("Stale")).toBeInTheDocument();
    expect(screen.getByText("Learned by Niles")).toBeInTheDocument();
    expect(screen.getByText("Asked for")).toBeInTheDocument();
    expect(screen.getAllByText("Pinned")).toHaveLength(1);
  });

  it("explains, with nothing yet, where skills come from", async () => {
    show([]);
    expect(
      await screen.findByText(/When Niles works out how to do something for the household/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("says why when the instance keeps no skills", async () => {
    vi.spyOn(api, "skills").mockRejectedValue(new ApiError("no skill store configured", 501));
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <SkillsPage />
      </QueryClientProvider>,
    );
    expect(await screen.findByText("no skill store configured")).toBeInTheDocument();
  });

  it("opens a skill and shows its procedure", async () => {
    show([windDown]);
    const dialog = await open("Evening wind down");
    expect(await within(dialog).findByText(/Dim the living room to 40%/)).toBeInTheDocument();
    expect(vi.mocked(api.skill).mock.calls[0][0]).toBe("evening-wind-down");
  });

  it("pins a skill with its switch", async () => {
    show([windDown]);
    const update = vi.spyOn(api, "updateSkill").mockResolvedValue(undefined);
    const dialog = await open("Evening wind down");
    const pin = await within(dialog).findByRole("switch", { name: "Pin Evening wind down" });
    expect(pin).not.toBeChecked();
    fireEvent.click(pin);
    await waitFor(() => expect(update).toHaveBeenCalled());
    expect(update.mock.calls[0][0]).toBe("evening-wind-down");
    expect(update.mock.calls[0][1]).toEqual({ pinned: true });
  });

  it("archives an active skill and restores an archived one", async () => {
    show([windDown, oldParty]);
    const update = vi.spyOn(api, "updateSkill").mockResolvedValue(undefined);
    let dialog = await open("Evening wind down");
    fireEvent.click(await within(dialog).findByRole("button", { name: "Archive" }));
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    expect(update.mock.calls[0][1]).toEqual({ status: "archived" });

    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: /Archived \(1\)/ }));
    dialog = await open("Party lights");
    fireEvent.click(await within(dialog).findByRole("button", { name: "Restore" }));
    await waitFor(() => expect(update).toHaveBeenCalledTimes(2));
    expect(update.mock.calls[1][0]).toBe("party-lights");
    expect(update.mock.calls[1][1]).toEqual({ status: "active" });
  });

  it("deletes a skill once it is confirmed", async () => {
    show([windDown]);
    const remove = vi.spyOn(api, "deleteSkill").mockResolvedValue(undefined);
    const dialog = await open("Evening wind down");
    fireEvent.click(await within(dialog).findByRole("button", { name: "Delete" }));
    expect(remove).not.toHaveBeenCalled();
    expect(within(dialog).getByText(/Delete this skill\?/)).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(remove).toHaveBeenCalled());
    expect(remove.mock.calls[0][0]).toBe("evening-wind-down");
  });

  it("will not delete a pinned skill, and says why", async () => {
    show([coffee]);
    const remove = vi.spyOn(api, "deleteSkill").mockResolvedValue(undefined);
    const dialog = await open("Morning coffee");
    const button = await within(dialog).findByRole("button", { name: "Delete" });
    expect(button).toBeDisabled();
    expect(within(dialog).getByText("Unpin it to delete it.")).toBeInTheDocument();
    fireEvent.click(button);
    expect(within(dialog).queryByText(/Delete this skill\?/)).toBeNull();
    expect(remove).not.toHaveBeenCalled();
  });
});
