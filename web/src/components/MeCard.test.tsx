import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MeCard, NOTES_LIMIT, birthdayFrom, birthdayParts } from "@/components/MeCard";
import type { DeviceView, Me } from "@/lib/api";

const ME: Me = {
  email: "mark@example.com",
  speaker: "mark",
  display_name: "Mark",
  spoken_as: null,
  address_as: "Sir",
  notes: "- Takes tea, not coffee",
  birthday: "10-03",
  phone: null,
};

const UNPAIRED: DeviceView = {
  available: true,
  on_home_network: true,
  mac: "aa:bb:cc:dd:ee:ff",
  name: "Mark's iPhone",
  paired: false,
  signed_in: true,
};

function setup(props: Partial<React.ComponentProps<typeof MeCard>> = {}) {
  const onSave = vi.fn();
  const onPair = vi.fn();
  const onUnpair = vi.fn();
  render(
    <MeCard me={ME} onSave={onSave} onPair={onPair} onUnpair={onUnpair} {...props} />,
  );
  return { onSave, onPair, onUnpair };
}

describe("birthday", () => {
  it("is month and day, padded", () => {
    expect(birthdayFrom("10", "3")).toBe("10-03");
    expect(birthdayFrom("2", "29")).toBe("02-29");
  });

  it("is nothing until both halves are there and the day exists", () => {
    expect(birthdayFrom("10", "")).toBeNull();
    expect(birthdayFrom("", "3")).toBeNull();
    expect(birthdayFrom("2", "30")).toBeNull();
    expect(birthdayFrom("4", "31")).toBeNull();
  });

  it("clears when both are emptied", () => {
    expect(birthdayFrom("", "")).toBe("");
  });

  it("reads back what was saved", () => {
    expect(birthdayParts("10-03")).toEqual({ month: "10", day: "3" });
    expect(birthdayParts(null)).toEqual({ month: "", day: "" });
  });
});

describe("MeCard", () => {
  it("shows the notes and saves them only when asked", () => {
    // A paragraph half rewritten is not something Niles should start
    // reading, so nothing is written as you type or when you look away.
    const { onSave } = setup();
    const notes = screen.getByLabelText("Your notes");
    expect(notes).toHaveValue("- Takes tea, not coffee");
    fireEvent.change(notes, { target: { value: "- Takes tea\n- Supports Arsenal" } });
    fireEvent.blur(notes);
    expect(onSave).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save notes" }));
    expect(onSave).toHaveBeenCalledWith({ notes: "- Takes tea\n- Supports Arsenal" });
  });

  it("will not save notes longer than Niles reads", () => {
    setup();
    fireEvent.change(screen.getByLabelText("Your notes"), {
      target: { value: "x".repeat(NOTES_LIMIT + 1) },
    });
    expect(screen.getByRole("button", { name: "Save notes" })).toBeDisabled();
  });

  it("saves how they are addressed, and lets it be cleared", () => {
    const { onSave } = setup();
    const field = screen.getByLabelText("Address me as");
    expect(field).toHaveValue("Sir");
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.blur(field);
    expect(onSave).toHaveBeenCalledWith({ address_as: "" });
  });

  it("saves a respelling of the name", () => {
    const { onSave } = setup();
    const field = screen.getByLabelText("Say my name like");
    fireEvent.change(field, { target: { value: "Mahrk" } });
    fireEvent.blur(field);
    expect(onSave).toHaveBeenCalledWith({ spoken_as: "Mahrk" });
  });

  it("saves the birthday when the day changes", () => {
    const { onSave } = setup();
    const day = screen.getByLabelText("Birthday day");
    expect(day).toHaveValue("3");
    fireEvent.change(day, { target: { value: "4" } });
    fireEvent.blur(day);
    expect(onSave).toHaveBeenCalledWith({ birthday: "10-04" });
  });

  it("does not save a day the month does not have", () => {
    const { onSave } = setup({ me: { ...ME, birthday: "02-01" } });
    const day = screen.getByLabelText("Birthday day");
    fireEvent.change(day, { target: { value: "30" } });
    fireEvent.blur(day);
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByText("That day is not in that month.")).toBeInTheDocument();
  });

  it("explains a sign-in with no voice instead of offering empty fields", () => {
    setup({ me: { ...ME, speaker: null, display_name: null, notes: null } });
    expect(screen.queryByLabelText("Your notes")).not.toBeInTheDocument();
    expect(screen.getByText("Niles does not know your voice yet")).toBeInTheDocument();
  });

  it("offers to pair this phone when it can be", () => {
    const { onPair } = setup({ device: UNPAIRED });
    fireEvent.click(screen.getByRole("button", { name: "This is my phone" }));
    expect(onPair).toHaveBeenCalled();
  });

  it("unpairs a paired phone", () => {
    const { onUnpair } = setup({
      me: { ...ME, phone: "aa:bb:cc:dd:ee:ff" },
      device: { ...UNPAIRED, paired: true },
    });
    expect(screen.getByText("This phone", { exact: false })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Unpair" }));
    expect(onUnpair).toHaveBeenCalled();
  });
});
