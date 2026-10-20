import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Approvals, ApprovalsView } from "@/components/ApprovalsList";
import { api } from "@/lib/api";
import type { DecidedApproval, PendingApproval } from "@/lib/api";
import { badgeText, decidedLabel, historyLine } from "@/lib/approvals";

const WAITING: PendingApproval = {
  id: "a1",
  title: "Let Linear create issue?",
  subtitle: "Linear",
  icon: "linear",
  fields: [{ label: "Title", value: "WLED presets" }],
  body: null,
  allow_conversation: true,
  status: "pending",
  outcome: null,
  tool: "Linear",
  summary: "Title: WLED presets",
  conversation_id: 4,
  asked_at: new Date().toISOString(),
  expires_at: new Date(Date.now() + 3_600_000).toISOString(),
  via: "app",
};

const MAIL: PendingApproval = {
  ...WAITING,
  id: "m1",
  title: "Send this email?",
  subtitle: "From your Gmail, mark@gmail.com",
  icon: "mail",
  fields: [{ label: "To", value: "jakob@example.com" }],
  body: "Hi Jakob",
  allow_conversation: false,
  tool: "Mail",
  summary: "To: jakob@example.com",
};

function decided(id: number, rest: Partial<DecidedApproval> = {}): DecidedApproval {
  return {
    id,
    approval: `d${id}`,
    tool: "Linear",
    icon: "linear",
    title: "Let Linear create issue?",
    summary: "Title: Old one",
    status: "allowed",
    decision: "once",
    outcome: "Done",
    decided_via: "app",
    asked_via: "app",
    asked_at: new Date().toISOString(),
    decided_at: new Date().toISOString(),
    conversation_id: 4,
    ...rest,
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("the history's words", () => {
  it("says what became of each", () => {
    expect(decidedLabel(decided(1))).toBe("Allowed once");
    expect(decidedLabel(decided(1, { decision: "conversation" }))).toBe(
      "Allowed for the conversation",
    );
    expect(decidedLabel(decided(1, { status: "denied", decision: "deny" }))).toBe("Denied");
    expect(decidedLabel(decided(1, { status: "lapsed", decision: null }))).toBe("Lapsed");
    expect(decidedLabel(decided(1, { status: "failed" }))).toBe("Allowed, but it didn't work");
    expect(historyLine(decided(1, { decided_via: "voice" }))).toMatch(/^Allowed once by voice · /);
  });

  it("counts on the badge until a count stops being read", () => {
    expect(badgeText(3)).toBe("3");
    expect(badgeText(12)).toBe("9+");
  });
});

describe("ApprovalsView", () => {
  it("puts what needs review first, folded, and the history under it", () => {
    render(
      <ApprovalsView pending={[WAITING]} history={[decided(1)]} onDecide={() => {}} />,
    );
    const card = screen.getByRole("region", { name: "Let Linear create issue?" });
    expect(within(card).getByText("Title: WLED presets")).toBeInTheDocument();
    expect(within(card).getByText("Details")).toBeInTheDocument();
    const history = screen.getByRole("region", { name: "Approvals history" });
    expect(within(history).getByText("Title: Old one")).toBeInTheDocument();
    expect(within(history).getByText(/^Allowed once · /)).toBeInTheDocument();
  });

  it("offers a mail only once", () => {
    render(<ApprovalsView pending={[MAIL]} history={[]} onDecide={() => {}} />);
    expect(screen.getByRole("button", { name: "Allow once" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Allow for this conversation" })).toBeNull();
  });

  it("goes to the conversation a card is in", () => {
    const open = vi.fn();
    render(
      <ApprovalsView pending={[WAITING]} history={[]} onDecide={() => {}} onOpenConversation={open} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Show in the chat" }));
    expect(open).toHaveBeenCalledWith(4);
  });

  it("says when nothing waits, and when nothing is kept", () => {
    render(<ApprovalsView pending={[]} history={[]} kept={false} onDecide={() => {}} />);
    expect(screen.getByText("Nothing is waiting for you.")).toBeInTheDocument();
    expect(screen.getByText(/keeps no history/)).toBeInTheDocument();
  });

  it("draws out the card a link led to", () => {
    const second: PendingApproval = { ...WAITING, id: "a2", title: "Let Linear create another?" };
    render(
      <ApprovalsView pending={[WAITING, second]} history={[]} onDecide={() => {}} highlight="a2" />,
    );
    expect(screen.getByRole("region", { name: "Let Linear create another?" })).toHaveAttribute(
      "aria-current",
      "true",
    );
    expect(screen.getByRole("region", { name: "Let Linear create issue?" })).not.toHaveAttribute(
      "aria-current",
    );
  });
});

describe("Approvals", () => {
  function show() {
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <Approvals />
      </QueryClientProvider>,
    );
  }

  it("answers by the approval's id, each of the three ways", async () => {
    vi.spyOn(api, "approvals").mockResolvedValue({ approvals: [WAITING] });
    vi.spyOn(api, "approvalHistory").mockResolvedValue({ entries: [], next: null, kept: true });
    const decide = vi.spyOn(api, "decideHeld").mockResolvedValue({ reply: "Done." });
    show();
    for (const [name, decision] of [
      ["Allow once", "once"],
      ["Allow for this conversation", "conversation"],
      ["Deny", "deny"],
    ] as const) {
      fireEvent.click(await screen.findByRole("button", { name }));
      await waitFor(() => expect(decide).toHaveBeenLastCalledWith("a1", decision, null));
      await waitFor(() => expect(screen.getByRole("button", { name })).not.toBeDisabled());
    }
  });

  it("asks for an older page while there is one", async () => {
    vi.spyOn(api, "approvals").mockResolvedValue({ approvals: [] });
    const history = vi
      .spyOn(api, "approvalHistory")
      .mockImplementation(async (before) =>
        before === undefined
          ? { entries: [decided(9)], next: 9, kept: true }
          : { entries: [decided(3, { summary: "Title: Older" })], next: null, kept: true },
      );
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Show older" }));
    expect(await screen.findByText("Title: Older")).toBeInTheDocument();
    expect(history).toHaveBeenLastCalledWith(9);
    await waitFor(() => expect(screen.queryByRole("button", { name: "Show older" })).toBeNull());
  });
});
