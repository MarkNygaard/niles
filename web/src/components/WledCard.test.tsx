import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { WledCard } from "@/components/WledCard";
import type { WledStrip } from "@/lib/api";

const CEILING: WledStrip = {
  name: "living_room/ceiling",
  topic: "wled/living_room",
  rgb: false,
  white_balance: true,
};

describe("WledCard", () => {
  it("shows which of the three a strip is", () => {
    render(<WledCard strips={[CEILING]} onChange={vi.fn()} />);
    expect(screen.getByRole("radio", { name: "White balance" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "Colour" })).not.toBeChecked();
  });

  it("reads an unwritten strip as a colour one", () => {
    // The config file need not mention either flag, and the defaults
    // are what WLED strips usually are.
    render(
      <WledCard
        strips={[{ name: "office/desk", topic: "wled/office" }]}
        onChange={vi.fn()}
      />,
    );
    expect(screen.getByRole("radio", { name: "Colour" })).toBeChecked();
  });

  it("writes both flags when one is picked", () => {
    // They are two fields in the config and one question here, so a
    // pick has to set the other as well — otherwise switching a strip
    // to white balance would leave it claiming colour too.
    const onChange = vi.fn();
    render(<WledCard strips={[CEILING]} onChange={onChange} />);
    fireEvent.click(screen.getByRole("radio", { name: "Colour" }));
    expect(onChange).toHaveBeenCalledWith([
      { ...CEILING, rgb: true, white_balance: false },
    ]);
  });

  it("refuses a name that is not a room and a device", () => {
    const onChange = vi.fn();
    render(<WledCard strips={[]} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText("Strip name"), {
      target: { value: "ceiling" },
    });
    fireEvent.change(screen.getByLabelText("Strip topic"), {
      target: { value: "wled/living_room" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Add/ }));
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByText(/room and a device/)).toBeInTheDocument();
  });

  it("adds one", () => {
    const onChange = vi.fn();
    render(<WledCard strips={[]} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText("Strip name"), {
      target: { value: "office/desk" },
    });
    fireEvent.change(screen.getByLabelText("Strip topic"), {
      target: { value: "wled/office" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Add/ }));
    expect(onChange).toHaveBeenCalledWith([
      { name: "office/desk", topic: "wled/office", rgb: true, white_balance: false },
    ]);
  });

  it("removes one", () => {
    const onChange = vi.fn();
    render(<WledCard strips={[CEILING]} onChange={onChange} />);
    fireEvent.click(
      screen.getByRole("button", { name: "Remove living_room/ceiling" }),
    );
    expect(onChange).toHaveBeenCalledWith([]);
  });
});
