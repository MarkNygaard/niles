import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { VoicesCard, voiceSummary } from "@/components/VoicesCard";

function setup(props: Partial<React.ComponentProps<typeof VoicesCard>> = {}) {
  const onChange = vi.fn();
  render(
    <VoicesCard
      knownVoicesOnly={false}
      recognitionOn={true}
      onChange={onChange}
      {...props}
    />,
  );
  return { onChange };
}

describe("VoicesCard", () => {
  it("is off until somebody turns it on", () => {
    // A house that upgrades into this setting must not find itself
    // locked by it.
    setup();
    expect(screen.getByRole("switch")).not.toBeChecked();
  });

  it("writes the switch straight through", () => {
    const { onChange } = setup();
    fireEvent.click(screen.getByRole("switch"));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("says how to add somebody before you need to know", () => {
    // The chicken-and-egg: with the lock on, a new voice cannot
    // introduce itself either. Saying so only after somebody is stuck
    // is saying it too late.
    setup({ knownVoicesOnly: true });
    expect(screen.getByText(/switch this off/i)).toBeInTheDocument();
  });

  it("does not explain the way out when there is no way in", () => {
    setup({ knownVoicesOnly: false });
    expect(screen.queryByText(/switch this off/i)).not.toBeInTheDocument();
  });

  it("admits when the switch governs nothing", () => {
    // Recognition is not running, so the lock cannot lock. A switch
    // that looks obeyed and is not is worse than one that says so.
    setup({ recognitionOn: false });
    expect(screen.getByText(/not set up to recognise voices/i)).toBeInTheDocument();
  });

  it("stays quiet about that once recognition is running", () => {
    setup({ recognitionOn: true });
    expect(
      screen.queryByText(/not set up to recognise voices/i),
    ).not.toBeInTheDocument();
  });
});

const MARK = {
  speaker: "mark",
  display_name: "Mark",
  spoken_as: null,
  clip_count: 3,
  created_at: "2026-09-22T19:43:12Z",
  last_seen_at: "2026-09-22T20:10:00Z",
};

describe("the enrolled voices", () => {
  it("says nobody is enrolled, and how to change that", () => {
    setup({ voices: [] });
    expect(screen.getByText(/Nobody yet/)).toBeInTheDocument();
  });

  it("shows nothing at all until they have loaded", () => {
    // Undefined is "still asking". Saying "nobody" then would be a
    // claim about the house made from a pending request.
    setup({ voices: undefined });
    expect(screen.queryByText(/Nobody yet/)).not.toBeInTheDocument();
  });

  it("lists who Niles knows", () => {
    setup({ voices: [MARK] });
    expect(screen.getByText("Mark")).toBeInTheDocument();
  });

  it("forgets one by its slug, not its display name", () => {
    // The slug is what the store and `auth.allowed[].speaker` use.
    const onForget = vi.fn();
    render(
      <VoicesCard
        knownVoicesOnly={false}
        recognitionOn
        voices={[MARK]}
        onChange={vi.fn()}
        onForget={onForget}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Forget Mark" }));
    expect(onForget).toHaveBeenCalledWith("mark");
  });
});

describe("voiceSummary", () => {
  it("calls a single clip thin, and asks for more", () => {
    // One clip is the state that cannot recognise anybody, and it is
    // indistinguishable from a working enrolment everywhere else.
    expect(voiceSummary({ ...MARK, clip_count: 1 })).toContain("1 clip");
    expect(voiceSummary({ ...MARK, clip_count: 1 })).toContain("thin");
  });

  it("stops asking once there are three", () => {
    expect(voiceSummary({ ...MARK, clip_count: 3 })).not.toContain("thin");
  });

  it("says plainly when a voice has never been recognised", () => {
    // Which is the shape of an enrolment that is not working, and the
    // reason this card exists.
    expect(voiceSummary({ ...MARK, last_seen_at: null })).toContain(
      "never recognised since",
    );
  });
});

describe("correcting a name", () => {
  it("writes the new name against the slug on blur", () => {
    // Whisper spelled one Danish name four ways in four attempts. The
    // slug is stuck with the first; what anybody reads is not.
    const onRename = vi.fn();
    render(
      <VoicesCard
        knownVoicesOnly={false}
        recognitionOn
        voices={[MARK]}
        onChange={vi.fn()}
        onRename={onRename}
      />,
    );
    const field = screen.getByLabelText("mark name");
    fireEvent.change(field, { target: { value: "Majse" } });
    fireEvent.blur(field);
    expect(onRename).toHaveBeenCalledWith("mark", "Majse");
  });

  it("does not write an unchanged name", () => {
    // Each rename is a write and a matcher rebuild.
    const onRename = vi.fn();
    render(
      <VoicesCard
        knownVoicesOnly={false}
        recognitionOn
        voices={[MARK]}
        onChange={vi.fn()}
        onRename={onRename}
      />,
    );
    fireEvent.blur(screen.getByLabelText("mark name"));
    expect(onRename).not.toHaveBeenCalled();
  });

  it("refuses to blank a name", () => {
    const onRename = vi.fn();
    render(
      <VoicesCard
        knownVoicesOnly={false}
        recognitionOn
        voices={[MARK]}
        onChange={vi.fn()}
        onRename={onRename}
      />,
    );
    const field = screen.getByLabelText("mark name");
    fireEvent.change(field, { target: { value: "   " } });
    fireEvent.blur(field);
    expect(onRename).not.toHaveBeenCalled();
    expect(field).toHaveValue("Mark");
  });
});

describe("saying a name correctly", () => {
  it("writes the respelling against the slug", () => {
    // Piper reads letters, not phonemes, so a Danish "Majse" comes out
    // wrong from an English voice and has to be respelled.
    const onSpokenAs = vi.fn();
    render(
      <VoicesCard
        knownVoicesOnly={false}
        recognitionOn
        voices={[{ ...MARK, speaker: "maisel", display_name: "Majse" }]}
        onChange={vi.fn()}
        onSpokenAs={onSpokenAs}
      />,
    );
    const field = screen.getByLabelText("maisel pronunciation");
    fireEvent.change(field, { target: { value: "Mayse" } });
    fireEvent.blur(field);
    expect(onSpokenAs).toHaveBeenCalledWith("maisel", "Mayse");
  });

  it("lets a respelling be cleared", () => {
    // Empty is a value here, unlike the name: it means "say it the way
    // it is written", which is what most names want.
    const onSpokenAs = vi.fn();
    render(
      <VoicesCard
        knownVoicesOnly={false}
        recognitionOn
        voices={[{ ...MARK, spoken_as: "Mayse" }]}
        onChange={vi.fn()}
        onSpokenAs={onSpokenAs}
      />,
    );
    const field = screen.getByLabelText("mark pronunciation");
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.blur(field);
    expect(onSpokenAs).toHaveBeenCalledWith("mark", "");
  });

  it("shows the respelling that is already set", () => {
    render(
      <VoicesCard
        knownVoicesOnly={false}
        recognitionOn
        voices={[{ ...MARK, spoken_as: "Mayse" }]}
        onChange={vi.fn()}
        onSpokenAs={vi.fn()}
      />,
    );
    expect(screen.getByLabelText("mark pronunciation")).toHaveValue("Mayse");
  });
});
