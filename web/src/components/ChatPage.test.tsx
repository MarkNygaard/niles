import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ChatPage, landingOn, panelStatus } from "@/components/ChatPage";
import { ApiError, api } from "@/lib/api";
import type {
  Artifact,
  ArtifactDetail,
  ArtifactList,
  ArtifactRef,
  Conversation,
  ConversationSummary,
  Turn,
} from "@/lib/api";

const MINUTE = 60_000;
const ago = (ms: number) => new Date(Date.now() - ms).toISOString();

function summary(id: number, title: string, rest: Partial<ConversationSummary> = {}) {
  return {
    id,
    title,
    started_at: ago(10 * MINUTE),
    last_at: ago(MINUTE),
    turns: 1,
    spoken: false,
    ...rest,
  } satisfies ConversationSummary;
}

function turn(said: string, reply: string, rest: Partial<Turn> = {}): Turn {
  return { at: ago(MINUTE), said, reply, via: "app", room: null, by: null, proposal: null, ...rest };
}

const CURTAINS = summary(42, "Make an automation that reminds me to close the curtains", {
  last_at: ago(2 * MINUTE),
  turns: 2,
  spoken: true,
});
const MILK = summary(7, "Add milk to the list", { last_at: ago(26 * 60 * MINUTE) });

const CURTAINS_TURNS: Turn[] = [
  turn("Is anyone home?", "Only you, Sir."),
  turn("make an automation that reminds me to close the curtains", "… Shall I save it?", {
    via: "voice",
    room: "living_room",
    proposal: { sentence: 'When it is 30 minutes after sunset, say "Please close the curtains".' },
  }),
];

function conversationOf(s: ConversationSummary, turns: Turn[]): Conversation {
  return { id: s.id, title: s.title, started_at: s.started_at, last_at: s.last_at, turns };
}

/** `sendChat` as the server does it: the turn lands in `id`, where the
    refetch after it will find it. */
function reply(conversations: Record<number, Conversation>, id: number, answer: string) {
  return async (text: string) => {
    const old = conversations[id] ?? conversationOf(summary(id, text), []);
    conversations[id] = { ...old, turns: [...old.turns, turn(text, answer)] };
    return { reply: answer, conversation_id: id };
  };
}

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

function frame(event: object) {
  act(() => FakeSocket.last!.onmessage!({ data: JSON.stringify(event) }));
}

const NOTHING_SAVED: ArtifactList = { artifacts: [], assistant: "claude" };

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
    created_at: ago(MINUTE),
    updated_at: ago(MINUTE),
    current_revision: 1,
    preview: { lines: [] },
    ...rest,
  };
}

function show(
  list: ConversationSummary[],
  conversations: Record<number, Conversation> = {},
  saved: ArtifactList = NOTHING_SAVED,
) {
  vi.stubGlobal("WebSocket", FakeSocket);
  if (!vi.isMockFunction(api.artifacts)) vi.spyOn(api, "artifacts").mockResolvedValue(saved);
  vi.spyOn(api, "conversations").mockResolvedValue(list);
  vi.spyOn(api, "conversation").mockImplementation(async (id) => {
    if (!conversations[id]) throw new ApiError("no such conversation", 404);
    return conversations[id];
  });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ChatPage />
    </QueryClientProvider>,
  );
}

async function openPanel() {
  fireEvent.click(await screen.findByRole("button", { name: "Open Niles" }));
  return within(await screen.findByRole("dialog", { name: "Niles" }));
}

const field = () => screen.getByRole("textbox", { name: "Message Niles" });

function type(text: string) {
  fireEvent.change(field(), { target: { value: text } });
  fireEvent.keyDown(field(), { key: "Enter" });
}

describe("landingOn", () => {
  const now = Date.parse("2026-10-14T12:00:00Z");
  const at = (ms: number) => new Date(now - ms).toISOString();

  it("opens the latest conversation while it is still going", () => {
    expect(landingOn([summary(3, "a", { last_at: at(4 * MINUTE) })], now)).toBe(3);
  });

  it("opens a new chat after five quiet minutes", () => {
    expect(landingOn([summary(3, "a", { last_at: at(5 * MINUTE) })], now)).toBeNull();
  });

  it("opens a new chat when there is nothing yet", () => {
    expect(landingOn([], now)).toBeNull();
  });
});

describe("panelStatus", () => {
  const now = new Date("2026-10-14T12:00:00Z");

  it("is thinking while a reply is on its way", () => {
    expect(panelStatus(true, [summary(1, "a")], undefined, now)).toBe("Thinking…");
  });

  it("says when Niles was last talked to", () => {
    const last = summary(1, "a", { last_at: new Date(now.getTime() - 5 * MINUTE).toISOString() });
    expect(panelStatus(false, [last], undefined, now)).toBe("Last talked 5 minutes ago");
  });

  it("says what is being written in the background", () => {
    const working = artifact(1, "Thailand travel plan", {
      status: "working",
      status_text: "Writing your Thailand travel plan…",
    });
    expect(panelStatus(false, [summary(1, "a")], [working], now)).toBe(
      "✍️ Writing your Thailand travel plan…",
    );
    expect(panelStatus(false, [], [artifact(2, "Notes")], now)).toBe("Ready");
  });

  it("is ready when nothing has been said yet", () => {
    expect(panelStatus(false, [], undefined, now)).toBe("Ready");
    expect(panelStatus(false, undefined, undefined, now)).toBe("Ready");
  });
});

describe("ChatPage", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("opens the panel on an artifact from its card in the chat", async () => {
    const card: ArtifactRef = {
      id: 9,
      kind: "document",
      title: "Thailand travel plan",
      summary: "Two weeks in March",
      status: "ready",
      preview: { lines: [{ text: "Bangkok", style: "bullet" }] },
    };
    show([CURTAINS], {
      42: conversationOf(CURTAINS, [
        turn("plan our Thailand trip", "I'm putting it together."),
        turn("", "Thailand travel plan is ready.", { artifacts: [card] }),
      ]),
    });
    const plan: ArtifactDetail = {
      ...artifact(9, "Thailand travel plan", { summary: "Two weeks in March" }),
      content: { markdown: "# Thailand\n\n- Bangkok" },
      revisions: [{ n: 1, reason: "First draft", author: "claude", at: ago(MINUTE) }],
    };
    vi.spyOn(api, "artifact").mockResolvedValue(plan);

    fireEvent.click(await screen.findByRole("button", { name: "Open Thailand travel plan" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Niles" }));
    expect(dialog.getByRole("tab", { name: "Saved" })).toHaveAttribute("aria-selected", "true");
    expect(
      await dialog.findByRole("heading", { name: "Thailand travel plan" }),
    ).toBeInTheDocument();
    expect(dialog.getByRole("heading", { name: "Thailand" })).toBeInTheDocument();
    expect(api.artifact).toHaveBeenCalledWith(9);

    fireEvent.click(dialog.getByRole("button", { name: "Saved" }));
    expect(await dialog.findByText(/Nothing saved yet/)).toBeInTheDocument();
  });

  it("shows the job being written in the status line, as the stream says", async () => {
    const working = artifact(3, "Thailand travel plan", {
      status: "working",
      status_text: "Writing your Thailand travel plan…",
    });
    // What the server would answer now: nothing, until the job starts.
    let now: ArtifactList = NOTHING_SAVED;
    vi.spyOn(api, "artifacts").mockImplementation(async () => now);
    show([MILK]);
    const dialog = await openPanel();
    expect(dialog.getByRole("status")).toHaveTextContent("Last talked yesterday");
    await waitFor(() => expect(FakeSocket.last?.onmessage).toBeTruthy());

    now = { artifacts: [working], assistant: "claude" };
    frame({
      type: "artifact_changed",
      id: 3,
      kind: "document",
      title: "Thailand travel plan",
      status: "working",
      status_text: "Writing your Thailand travel plan…",
    });
    await waitFor(() =>
      expect(dialog.getByRole("status")).toHaveTextContent(
        "✍️ Writing your Thailand travel plan…",
      ),
    );
    fireEvent.click(dialog.getByRole("tab", { name: "Saved" }));
    expect(await dialog.findByText("being written…")).toBeInTheDocument();

    now = { artifacts: [{ ...working, status: "ready", status_text: null }], assistant: "claude" };
    frame({
      type: "artifact_changed",
      id: 3,
      kind: "document",
      title: "Thailand travel plan",
      status: "ready",
      status_text: null,
    });
    await waitFor(() =>
      expect(dialog.getByRole("status")).toHaveTextContent("Last talked yesterday"),
    );
  });

  it("says in Saved that Claude or ChatGPT makes these, when neither is on", async () => {
    show([MILK], {}, { artifacts: [], assistant: null });
    const dialog = await openPanel();
    fireEvent.click(dialog.getByRole("tab", { name: "Saved" }));
    expect(await dialog.findByText(/Claude or ChatGPT makes these/)).toBeInTheDocument();
    expect(
      dialog.getByRole("link", { name: "Switch one on in Settings → Voice & AI" }),
    ).toBeInTheDocument();
  });

  it("lists past conversations, marking the spoken ones", async () => {
    show([CURTAINS, MILK], { 42: conversationOf(CURTAINS, CURTAINS_TURNS) });
    const dialog = await openPanel();
    const rows = within(dialog.getByRole("list", { name: "Conversations" })).getAllByRole("listitem");
    expect(rows).toHaveLength(2);

    expect(within(rows[0]).getByText(CURTAINS.title)).toBeInTheDocument();
    expect(within(rows[0]).getByRole("img", { name: "Said to a satellite" })).toBeInTheDocument();
    expect(within(rows[0]).getByText(/2 minutes ago · 2 turns/)).toBeInTheDocument();

    expect(within(rows[1]).getByText(MILK.title)).toBeInTheDocument();
    expect(within(rows[1]).queryByRole("img", { name: "Said to a satellite" })).toBeNull();
    expect(within(rows[1]).getByText(/yesterday · 1 turn/)).toBeInTheDocument();
  });

  it("opens on the latest conversation while it is still going", async () => {
    show([CURTAINS, MILK], { 42: conversationOf(CURTAINS, CURTAINS_TURNS) });
    expect(await screen.findByText("Only you, Sir.")).toBeInTheDocument();
    expect(api.conversation).toHaveBeenCalledWith(42);
  });

  it("opens a conversation from the list, with the room of each spoken turn", async () => {
    show([MILK], { 7: conversationOf(MILK, [turn("add milk to the list", "Milk is on the list.")]) });
    // MILK is a day old, so the tab opens on an empty chat.
    const dialog = await openPanel();
    expect(api.conversation).not.toHaveBeenCalled();
    fireEvent.click(dialog.getByText(MILK.title));

    expect(await screen.findByText("Milk is on the list.")).toBeInTheDocument();
    expect(screen.queryByText("Living room")).toBeNull();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("starts a new chat from the panel, and closes it", async () => {
    const conversations = { 42: conversationOf(CURTAINS, CURTAINS_TURNS) };
    show([CURTAINS], conversations);
    const send = vi.spyOn(api, "sendChat").mockImplementation(reply(conversations, 44, "Hello."));
    await screen.findByText("Only you, Sir.");

    const dialog = await openPanel();
    fireEvent.click(dialog.getByRole("button", { name: "New chat" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByText("Only you, Sir.")).toBeNull();
    type("hello");
    await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
    expect(send.mock.calls[0][1]).toEqual({ new: true });
  });

  it("says Niles is thinking while a reply is on its way", async () => {
    show([MILK]);
    vi.spyOn(api, "sendChat").mockReturnValue(new Promise(() => {}));
    const before = await openPanel();
    expect(before.getByText("Last talked yesterday")).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    type("Is anyone home?");
    const during = await openPanel();
    expect(during.getByRole("status")).toHaveTextContent("Thinking…");
  });

  it("shows the room a turn was spoken in", async () => {
    show([CURTAINS], { 42: conversationOf(CURTAINS, CURTAINS_TURNS) });
    expect(await screen.findByText("Living room")).toBeInTheDocument();
    expect(screen.getAllByText(/Shall I save it\?/)).toHaveLength(1);
  });

  it("starts a new conversation from an empty chat", async () => {
    const conversations: Record<number, Conversation> = {};
    show([MILK], conversations);
    const send = vi.spyOn(api, "sendChat").mockImplementation(reply(conversations, 43, "Nobody, Sir."));
    await screen.findByRole("button", { name: "Open Niles" });
    type("Is anyone home?");

    await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
    expect(send.mock.calls[0][0]).toBe("Is anyone home?");
    expect(send.mock.calls[0][1]).toEqual({ new: true });
    expect(await screen.findByText("Nobody, Sir.")).toBeInTheDocument();
  });

  it("carries on the conversation on screen", async () => {
    const conversations = { 42: conversationOf(CURTAINS, CURTAINS_TURNS) };
    show([CURTAINS], conversations);
    const send = vi.spyOn(api, "sendChat").mockImplementation(reply(conversations, 42, "Done."));
    await screen.findByText("Only you, Sir.");

    type("and the kitchen lights?");
    await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
    expect(send.mock.calls[0][1]).toEqual({ conversation_id: 42 });
    expect(await screen.findByText("Done.")).toBeInTheDocument();
    expect(screen.getByText("Only you, Sir.")).toBeInTheDocument();
  });

  it("sends the next message to the conversation the last one landed in", async () => {
    const conversations: Record<number, Conversation> = {};
    show([], conversations);
    const send = vi.spyOn(api, "sendChat").mockImplementation(reply(conversations, 45, "Yes, Sir."));
    await screen.findByRole("button", { name: "Open Niles" });

    type("hello");
    expect(await screen.findByText("Yes, Sir.")).toBeInTheDocument();
    type("and again");
    await waitFor(() => expect(send).toHaveBeenCalledTimes(2));
    expect(send.mock.calls[1][1]).toEqual({ conversation_id: 45 });
  });

  it("names the assistant that did not answer a message just sent", async () => {
    const conversations: Record<number, Conversation> = {};
    show([], conversations);
    const answered = reply(conversations, 46, "Nobody, Sir.");
    vi.spyOn(api, "sendChat").mockImplementation(async (text) => ({
      ...(await answered(text)),
      fallback: "ChatGPT is not signed in",
      fallback_from: "chatgpt" as const,
    }));
    await screen.findByRole("button", { name: "Open Niles" });
    type("Is anyone home?");
    expect(
      await screen.findByText("Answered without ChatGPT — ChatGPT is not signed in"),
    ).toBeInTheDocument();
  });

  it("goes back to a new chat on New chat", async () => {
    const conversations = { 42: conversationOf(CURTAINS, CURTAINS_TURNS) };
    show([CURTAINS], conversations);
    const send = vi.spyOn(api, "sendChat").mockImplementation(reply(conversations, 44, "Hello."));
    await screen.findByText("Only you, Sir.");

    fireEvent.click(screen.getByRole("button", { name: "New chat" }));
    expect(screen.queryByText("Only you, Sir.")).toBeNull();
    type("hello");
    await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
    expect(send.mock.calls[0][1]).toEqual({ new: true });
  });

  it("answers a proposal for the turn that made it, then shows what came of it", async () => {
    const conversations = { 42: conversationOf(CURTAINS, CURTAINS_TURNS) };
    show([CURTAINS], conversations);
    const answer = vi.spyOn(api, "answerProposal").mockImplementation(async () => {
      conversations[42] = conversationOf(CURTAINS, [
        ...CURTAINS_TURNS,
        turn("Yes", "Saved. When it is 30 minutes after sunset…"),
      ]);
      return { reply: "Saved. When it is 30 minutes after sunset…" };
    });

    fireEvent.click(await screen.findByRole("button", { name: "Save" }));
    await waitFor(() => expect(answer).toHaveBeenCalledTimes(1));
    expect(answer.mock.calls[0]).toEqual([42, 1, true]);

    expect(await screen.findByText("Saved. When it is 30 minutes after sunset…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Don't save" })).toBeNull();
  });

  it("says no to a proposal", async () => {
    show([CURTAINS], { 42: conversationOf(CURTAINS, CURTAINS_TURNS) });
    const answer = vi
      .spyOn(api, "answerProposal")
      .mockResolvedValue({ reply: "All right, I won't save it." });
    fireEvent.click(await screen.findByRole("button", { name: "Don't save" }));
    await waitFor(() => expect(answer).toHaveBeenCalledTimes(1));
    expect(answer.mock.calls[0]).toEqual([42, 1, false]);
  });

  it("asks before deleting a conversation", async () => {
    show([CURTAINS, MILK], { 42: conversationOf(CURTAINS, CURTAINS_TURNS) });
    const remove = vi.spyOn(api, "deleteConversation").mockResolvedValue(undefined);
    await screen.findByText("Only you, Sir.");
    const dialog = await openPanel();
    const row = dialog.getByText(MILK.title).closest("li")!;
    fireEvent.click(within(row).getByRole("button", { name: "Delete" }));

    expect(remove).not.toHaveBeenCalled();
    expect(await dialog.findByText("Delete this conversation?")).toBeInTheDocument();
    expect(dialog.getByText(MILK.title)).toBeInTheDocument();

    fireEvent.click(dialog.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(1));
    expect(remove.mock.calls[0][0]).toBe(7);
    // The list is asked again, and the one on screen stays.
    await waitFor(() => expect(api.conversations).toHaveBeenCalledTimes(2));
    expect(screen.getByText("Only you, Sir.")).toBeInTheDocument();
  });

  it("keeps a conversation when the delete is called off", async () => {
    show([MILK]);
    const remove = vi.spyOn(api, "deleteConversation").mockResolvedValue(undefined);
    const dialog = await openPanel();
    fireEvent.click(dialog.getByRole("button", { name: "Delete" }));
    fireEvent.click(await dialog.findByRole("button", { name: "Keep it" }));
    expect(await dialog.findByText(MILK.title)).toBeInTheDocument();
    expect(remove).not.toHaveBeenCalled();
  });
});
