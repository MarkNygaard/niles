import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ChatView, SUGGESTIONS } from "@/components/ChatView";
import type { ChatViewProps, ShownTurn } from "@/components/ChatView";

function turn(said: string, reply: string, rest: Partial<ShownTurn> = {}): ShownTurn {
  return {
    at: "2026-10-14T12:00:00Z",
    said,
    reply,
    via: "app",
    room: null,
    by: null,
    proposal: null,
    ...rest,
  };
}

function renderChat(props: Partial<ChatViewProps> = {}) {
  const handlers = { onSend: vi.fn(), onNew: vi.fn(), onOpenPanel: vi.fn(), onAnswer: vi.fn() };
  render(<ChatView turns={[]} {...handlers} {...props} />);
  return handlers;
}

const field = () => screen.getByRole("textbox", { name: "Message Niles" });

const PLAN = {
  id: 9,
  kind: "document",
  title: "Thailand travel plan",
  summary: "Two weeks in March",
  status: "ready" as const,
  preview: {
    lines: [
      { text: "Thailand, two weeks", style: "heading" as const },
      { text: "Bangkok", style: "bullet" as const },
      { text: "Book flights", style: "check" as const, checked: true },
      { text: "Ferry to Koh Tao", style: "check" as const, checked: false },
    ],
  },
};

describe("ChatView", () => {
  it("shows an artifact a reply made as a card with its first lines, and opens it", () => {
    const onOpenArtifact = vi.fn();
    renderChat({
      turns: [turn("plan our Thailand trip", "Here it is.", { artifacts: [PLAN] })],
      onOpenArtifact,
    });
    const card = screen.getByRole("button", { name: "Open Thailand travel plan" });
    expect(card).toHaveTextContent("Two weeks in March");
    expect(card).toHaveTextContent("Thailand, two weeks");
    expect(card).toHaveTextContent("• Bangkok");
    expect(card).toHaveTextContent("☑ Book flights");
    // Three lines at most: a card, not the document.
    expect(card).not.toHaveTextContent("Ferry to Koh Tao");
    fireEvent.click(card);
    expect(onOpenArtifact).toHaveBeenCalledWith(9);
  });

  it("says a card's artifact is being written instead of its lines", () => {
    renderChat({
      turns: [
        turn("plan it", "I'm putting it together.", {
          artifacts: [{ ...PLAN, status: "working", preview: { lines: [] } }],
        }),
      ],
    });
    expect(screen.getByRole("button", { name: "Open Thailand travel plan" })).toHaveTextContent(
      "being written…",
    );
  });

  it("shows a turn nobody said anything in as Niles alone", () => {
    renderChat({ turns: [turn("", "Thailand travel plan is ready.", { artifacts: [PLAN] })] });
    expect(screen.getByText("Thailand travel plan is ready.")).toBeInTheDocument();
    const list = screen.getByRole("list", { name: "Conversation" });
    // The reply's bubble and the card; no empty bubble from the person.
    expect(list.querySelectorAll(".bg-primary")).toHaveLength(0);
  });

  it("shows both sides of the conversation", () => {
    renderChat({ turns: [turn("Is anyone home?", "Only you, Sir.")] });
    expect(screen.getByText("Is anyone home?")).toBeInTheDocument();
    expect(screen.getByText("Only you, Sir.")).toBeInTheDocument();
  });

  it("sends on Enter and empties the field", () => {
    const { onSend } = renderChat();
    fireEvent.change(field(), { target: { value: "  add milk to the list " } });
    fireEvent.keyDown(field(), { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("add milk to the list");
    expect(field()).toHaveValue("");
  });

  it("keeps Shift+Enter for a new line", () => {
    const { onSend } = renderChat();
    fireEvent.change(field(), { target: { value: "first line" } });
    fireEvent.keyDown(field(), { key: "Enter", shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
  });

  it("does not send while an answer is coming", () => {
    // One question at a time: the second would be asked without the
    // first answer in its context.
    const { onSend } = renderChat({ pending: "Is anyone home?" });
    fireEvent.change(field(), { target: { value: "and the lights?" } });
    fireEvent.keyDown(field(), { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Niles is answering")).toBeInTheDocument();
  });

  it("offers something to ask on an empty chat", () => {
    const { onSend } = renderChat();
    fireEvent.click(screen.getByRole("button", { name: SUGGESTIONS[0] }));
    expect(onSend).toHaveBeenCalledWith(SUGGESTIONS[0]);
  });

  it("says when Claude wrote a reply", () => {
    renderChat({ turns: [turn("hi", "Good evening.", { by: "claude" })] });
    expect(screen.getByText("Claude")).toBeInTheDocument();
  });

  it("says why Claude did not answer", () => {
    renderChat({
      turns: [turn("hi", "Good evening.", { fallback: "Claude Code has no token" })],
    });
    expect(screen.getByText(/Answered without Claude — Claude Code has no token/)).toBeInTheDocument();
  });

  it("says when ChatGPT wrote a reply", () => {
    renderChat({ turns: [turn("hi", "Good evening.", { by: "chatgpt" })] });
    expect(screen.getByText("ChatGPT")).toBeInTheDocument();
    expect(screen.queryByText("Claude")).toBeNull();
  });

  it("names ChatGPT when it was ChatGPT that did not answer", () => {
    renderChat({
      turns: [
        turn("hi", "Good evening.", {
          fallback: "ChatGPT is not signed in",
          fallback_from: "chatgpt",
        }),
      ],
    });
    expect(
      screen.getByText("Answered without ChatGPT — ChatGPT is not signed in"),
    ).toBeInTheDocument();
  });

  it("starts a new chat", () => {
    const { onNew } = renderChat({ turns: [turn("hi", "Good evening.")] });
    fireEvent.click(screen.getByRole("button", { name: "New chat" }));
    expect(onNew).toHaveBeenCalled();
  });

  it("offers no new chat on one that is already empty", () => {
    renderChat();
    expect(screen.queryByRole("button", { name: "New chat" })).toBeNull();
  });

  it("opens Niles's panel from the avatar", () => {
    const { onOpenPanel } = renderChat();
    const avatar = screen.getByRole("button", { name: "Open Niles" });
    expect(avatar).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(avatar);
    expect(onOpenPanel).toHaveBeenCalled();
  });

  it("says which room a spoken turn was said in", () => {
    renderChat({
      turns: [
        turn("turn on the lamp", "Done.", { via: "voice", room: "living_room" }),
        turn("and the kitchen?", "Done."),
      ],
    });
    expect(screen.getAllByText("Done.")).toHaveLength(2);
    // The typed one is not marked.
    expect(screen.getAllByText("Living room")).toHaveLength(1);
  });

  it("answers an approval card by its id, even after more was said", () => {
    const onDecide = vi.fn();
    const approval = {
      id: "a1",
      title: "Let Linear create issue?",
      subtitle: "Linear",
      icon: "linear",
      fields: [{ label: "Title", value: "WLED presets" }],
      body: null,
      allow_conversation: true,
      status: "pending" as const,
      outcome: null,
    };
    // A card waits an hour, so unlike a proposal it stays answerable
    // under later turns.
    renderChat({
      turns: [turn("file it", "It's waiting for you.", { approval }), turn("thanks", "Any time.")],
      onDecide,
    });
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    expect(onDecide).toHaveBeenCalledWith("a1", "once");
  });

  describe("a proposal", () => {
    const proposal = { sentence: 'When it is 30 minutes after sunset, say "Close the curtains".' };
    const asked = turn("remind me to close the curtains", "… Shall I save it?", { proposal });

    it("can be saved, or not, from the last turn", () => {
      const { onAnswer } = renderChat({ turns: [turn("hi", "Good evening."), asked] });
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      expect(onAnswer).toHaveBeenLastCalledWith(1, true);
      fireEvent.click(screen.getByRole("button", { name: "Don't save" }));
      expect(onAnswer).toHaveBeenLastCalledWith(1, false);
    });

    it("is not offered once another turn follows it", () => {
      renderChat({ turns: [asked, turn("Yes", "Saved.")] });
      expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Don't save" })).toBeNull();
    });

    it("is not offered on a turn without one", () => {
      renderChat({ turns: [turn("hi", "Good evening.")] });
      expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    });
  });

  it("gives the tab bar back when it unmounts mid-typing", () => {
    const { unmount } = render(
      <ChatView turns={[]} onSend={vi.fn()} onNew={vi.fn()} onOpenPanel={vi.fn()} onAnswer={vi.fn()} />,
    );
    fireEvent.focus(field());
    expect(document.documentElement.dataset.typing).toBe("");
    unmount();
    expect(document.documentElement.dataset.typing).toBeUndefined();
  });

  it("offers no microphone where the browser cannot record", () => {
    // jsdom, like an old browser, has no MediaRecorder.
    renderChat({ onDictate: vi.fn() });
    expect(screen.queryByRole("button", { name: "Dictate" })).toBeNull();
  });

  describe("with a microphone", () => {
    class FakeRecorder {
      static isTypeSupported = () => false;
      state = "inactive";
      mimeType = "audio/mp4";
      ondataavailable?: (e: { data: Blob }) => void;
      onstop?: () => void;
      start() {
        this.state = "recording";
      }
      stop() {
        this.state = "inactive";
        this.ondataavailable?.({ data: new Blob(["sound"], { type: "audio/mp4" }) });
        this.onstop?.();
      }
    }

    const track = { stop: vi.fn() };

    function withMicrophone() {
      vi.stubGlobal("MediaRecorder", FakeRecorder);
      Object.defineProperty(navigator, "mediaDevices", {
        configurable: true,
        value: { getUserMedia: vi.fn().mockResolvedValue({ getTracks: () => [track] }) },
      });
    }

    afterEach(() => {
      vi.unstubAllGlobals();
      Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: undefined });
    });

    it("puts what was said in the field, to read before sending", async () => {
      withMicrophone();
      const onDictate = vi.fn().mockResolvedValue("add letmælk to the list");
      const { onSend } = renderChat({ onDictate });

      fireEvent.click(screen.getByRole("button", { name: "Dictate" }));
      fireEvent.click(await screen.findByRole("button", { name: "Stop dictating" }));

      expect(await screen.findByDisplayValue("add letmælk to the list")).toBeInTheDocument();
      expect(onDictate.mock.calls[0][0].type).toBe("audio/mp4");
      expect(onSend).not.toHaveBeenCalled();
      // And the phone stops showing the microphone as in use.
      expect(track.stop).toHaveBeenCalled();
    });

    it("says why when the transcriber fails", async () => {
      withMicrophone();
      renderChat({ onDictate: vi.fn().mockRejectedValue(new Error("could not transcribe that")) });
      fireEvent.click(screen.getByRole("button", { name: "Dictate" }));
      fireEvent.click(await screen.findByRole("button", { name: "Stop dictating" }));
      expect(await screen.findByText("could not transcribe that")).toBeInTheDocument();
    });
  });
});
