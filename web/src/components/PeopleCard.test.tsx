import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { PeopleCard, speakerChoices } from "@/components/PeopleCard";
import type { Person } from "@/components/PeopleCard";

function setup(
  people: Person[],
  voices?: { speaker: string; display_name: string }[],
) {
  const onChange = vi.fn();
  render(<PeopleCard people={people} voices={voices} onChange={onChange} />);
  return { onChange };
}

function type(value: string) {
  fireEvent.change(screen.getByLabelText("Email address to allow"), {
    target: { value },
  });
}

describe("PeopleCard", () => {
  it("says sign-in is off while nobody is listed", () => {
    // An empty list is not a locked door, it is an open one, and that
    // is the opposite of what somebody would assume.
    setup([]);
    expect(screen.getByText(/Signing in is off/)).toBeInTheDocument();
  });

  it("adds an address", () => {
    const { onChange } = setup([]);
    type("majse@example.com");
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(onChange).toHaveBeenCalledWith([{ email: "majse@example.com" }]);
  });

  it("refuses a username before it reaches the server", () => {
    // The commonest mistake: pasting a GitHub login. Answering here
    // means the answer arrives while it is still on screen to fix.
    const { onChange } = setup([]);
    type("marknygaard");
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(screen.getByText(/not an email address/)).toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("refuses somebody already listed, whatever the case", () => {
    const { onChange } = setup([{ email: "mark@example.com" }]);
    type("Mark@Example.com");
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(screen.getByText(/already on the list/)).toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("removes somebody while others remain", () => {
    const { onChange } = setup([
      { email: "mark@example.com" },
      { email: "majse@example.com" },
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Remove mark@example.com" }));
    expect(onChange).toHaveBeenCalledWith([{ email: "majse@example.com" }]);
  });

  it("will not remove the last person", () => {
    // With nobody listed, nobody could sign in to put them back — and
    // the button offering it is on the page you would need to reach.
    const { onChange } = setup([{ email: "mark@example.com" }]);
    const remove = screen.getByRole("button", {
      name: /Remove mark@example.com — not while they are the only one/,
    });
    expect(remove).toBeDisabled();
    fireEvent.click(remove);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("shows the voice identity when one is linked", () => {
    setup([{ email: "mark@example.com", speaker: "mark" }], [
      { speaker: "mark", display_name: "Mark" },
    ]);
    expect(
      screen.getByRole("combobox", { name: "mark@example.com voice" }),
    ).toHaveTextContent("mark");
  });

  it("offers no pairing until the voices are known", () => {
    // Undefined is still loading, and a control that offers "No voice"
    // before it knows any would read as "there are none".
    setup([{ email: "mark@example.com", speaker: "mark" }]);
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });
});

// The dropdown cannot be opened in jsdom — Base UI's Select hangs it —
// so the rule that decides what it offers is tested here.
describe("speakerChoices", () => {
  const MARK = { speaker: "mark", display_name: "Mark" };
  const MAJSE = { speaker: "maisel", display_name: "Majse" };

  it("offers the enrolled voices", () => {
    expect(speakerChoices([MARK, MAJSE], undefined)).toEqual([MARK, MAJSE]);
  });

  it("shows the name, not the slug Whisper heard", () => {
    // The slug is "maisel" because that is what Whisper made of a
    // Danish name. It is the value that gets saved; it is not what
    // anybody should have to recognise in a list.
    const [, majse] = speakerChoices([MARK, MAJSE], undefined);
    expect(majse.display_name).toBe("Majse");
    expect(majse.speaker).toBe("maisel");
  });

  it("keeps a pairing whose voice has been deleted", () => {
    // Worth showing rather than silently dropping: the fix is to
    // repair it, and you cannot repair what the page will not display.
    expect(speakerChoices([MAJSE], "mark")).toEqual([
      MAJSE,
      { speaker: "mark", display_name: "mark" },
    ]);
  });

  it("does not offer a voice twice", () => {
    expect(speakerChoices([MARK], "mark")).toEqual([MARK]);
  });

  it("is empty before the voices have loaded", () => {
    expect(speakerChoices(undefined, undefined)).toEqual([]);
  });
});
