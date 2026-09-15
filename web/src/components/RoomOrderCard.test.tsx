import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { RoomOrderCard, moved } from "./RoomOrderCard";

const ROOMS = [
  { name: "bedroom", label: "Bedroom" },
  { name: "kitchen", label: "Kitchen" },
  { name: "living_room", label: "Living room" },
];

describe("moved", () => {
  it("carries one item to a new place and closes the gap behind it", () => {
    expect(moved(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
    expect(moved(["a", "b", "c"], 0, 1)).toEqual(["b", "a", "c"]);
  });

  it("holds the ends", () => {
    // Arrowing up from the top is a no-op, not a room falling off it.
    expect(moved(["a", "b", "c"], 0, -1)).toEqual(["a", "b", "c"]);
    expect(moved(["a", "b", "c"], 2, 3)).toEqual(["a", "b", "c"]);
  });
});

describe("RoomOrderCard", () => {
  it("lists the rooms in the order they are given, numbered", () => {
    render(<RoomOrderCard rooms={ROOMS} onChange={vi.fn()} />);
    const rows = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(rows).toEqual(["Bedroom1", "Kitchen2", "Living room3"]);
  });

  it("moves a room with the arrow keys and saves it", () => {
    // The grip is the same control for a thumb and for a keyboard, so
    // arranging the list never needs a pointer.
    const onChange = vi.fn();
    render(<RoomOrderCard rooms={ROOMS} onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole("button", { name: "Move Living room" }), {
      key: "ArrowUp",
    });
    expect(onChange).toHaveBeenCalledWith(["bedroom", "living_room", "kitchen"]);
  });

  it("saves once, when the row is put down", () => {
    const onChange = vi.fn();
    const { container } = render(
      <RoomOrderCard rooms={ROOMS} onChange={onChange} />,
    );
    const list = container.querySelector("ul")!;
    // jsdom has no layout, so the list is told how tall it is: three
    // rows of 40px, which is what makes a Y a row index.
    list.getBoundingClientRect = () =>
      ({ top: 0, bottom: 120, height: 120, left: 0, right: 300, width: 300 }) as DOMRect;
    const grip = screen.getByRole("button", { name: "Move Living room" });
    grip.setPointerCapture = () => {};

    fireEvent.pointerDown(grip, { clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(grip, { clientY: 60, pointerId: 1 });
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.pointerMove(grip, { clientY: 10, pointerId: 1 });
    fireEvent.pointerUp(grip, { clientY: 10, pointerId: 1 });
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith(["living_room", "bedroom", "kitchen"]);
  });

  it("saves nothing when a row is picked up and put back", () => {
    const onChange = vi.fn();
    const grip = (() => {
      render(<RoomOrderCard rooms={ROOMS} onChange={onChange} />);
      const found = screen.getByRole("button", { name: "Move Kitchen" });
      found.setPointerCapture = () => {};
      return found;
    })();
    fireEvent.pointerDown(grip, { clientY: 50, pointerId: 1 });
    fireEvent.pointerUp(grip, { clientY: 50, pointerId: 1 });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("says so when there are no rooms rather than showing an empty list", () => {
    render(<RoomOrderCard rooms={[]} onChange={vi.fn()} />);
    expect(screen.getByText(/No rooms yet/)).toBeInTheDocument();
    expect(screen.queryByRole("listitem")).toBeNull();
  });
});
