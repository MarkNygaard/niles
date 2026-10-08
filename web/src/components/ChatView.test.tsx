import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ChatView, SUGGESTIONS } from "@/components/ChatView";
import type { ChatViewProps } from "@/components/ChatView";

function renderChat(props: Partial<ChatViewProps> = {}) {
  const handlers = { onSend: vi.fn(), onForget: vi.fn() };
  render(<ChatView exchanges={[]} {...handlers} {...props} />);
  return handlers;
}

const field = () => screen.getByRole("textbox", { name: "Message Niles" });

describe("ChatView", () => {
  it("shows both sides of the conversation", () => {
    renderChat({ exchanges: [{ said: "Is anyone home?", reply: "Only you, Sir." }] });
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

  it("starts over", () => {
    const { onForget } = renderChat({ exchanges: [{ said: "hi", reply: "Good evening." }] });
    fireEvent.click(screen.getByRole("button", { name: "New conversation" }));
    expect(onForget).toHaveBeenCalled();
  });

  it("gives the tab bar back when it unmounts mid-typing", () => {
    const { unmount } = render(<ChatView exchanges={[]} onSend={vi.fn()} onForget={vi.fn()} />);
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
