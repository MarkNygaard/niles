import { describe, expect, it, vi } from "vitest";
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
});
