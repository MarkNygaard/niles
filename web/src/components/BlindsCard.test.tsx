import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { BlindsCard } from "@/components/BlindsCard";
import type { BlindOverride, Device } from "@/lib/api";

const OFFICE_BLIND: Device = {
  id: "z2m:office/blind",
  source: "z2m",
  room: "office",
  name: "blind",
  class: "cover",
  state: {
    on: null,
    brightness: null,
    color_temp_kelvin: null,
    rgb: null,
    temperature_celsius: null,
    humidity_percent: null,
    battery_percent: null,
    open: null,
    motion: null,
    illuminance_lux: null,
    position: null,
  },
  supports_rgb: false,
  supports_color_temp: false,
};

const KITCHEN_BLIND: Device = {
  ...OFFICE_BLIND,
  id: "z2m:kitchen/blind",
  room: "kitchen",
};

const rowStub = () =>
  vi.fn(() => <div data-testid="house-row">house level row</div>);

describe("BlindsCard", () => {
  it("draws nothing when there are no blinds", () => {
    const { container } = render(
      <BlindsCard
        blinds={[]}
        houseLevel={100}
        overrides={[]}
        onChange={vi.fn()}
        row={rowStub()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("lists one row per blind with the house level as the placeholder", () => {
    render(
      <BlindsCard
        blinds={[KITCHEN_BLIND, OFFICE_BLIND]}
        houseLevel={75}
        overrides={[]}
        onChange={vi.fn()}
        row={rowStub()}
      />,
    );
    // Sorted by room: kitchen before office.
    const fields = screen.getAllByLabelText(/opens to$/);
    expect(fields).toHaveLength(2);
    expect(fields[0]).toHaveAttribute("placeholder", "75");
    expect(fields[1]).toHaveAttribute("placeholder", "75");
  });

  it("writes an override when a level is committed", () => {
    const onChange = vi.fn();
    render(
      <BlindsCard
        blinds={[OFFICE_BLIND]}
        houseLevel={75}
        overrides={[]}
        onChange={onChange}
        row={rowStub()}
      />,
    );
    fireEvent.change(screen.getByLabelText("Blind opens to"), {
      target: { value: "60" },
    });
    fireEvent.blur(screen.getByLabelText("Blind opens to"));
    expect(onChange).toHaveBeenCalledWith([
      { device: "z2m:office/blind", normal_open: 60 },
    ]);
  });

  it("clears a bare-ID override when the field is emptied", () => {
    const overrides: BlindOverride[] = [
      { device: "office/blind", normal_open: 60 },
    ];
    const onChange = vi.fn();
    render(
      <BlindsCard
        blinds={[OFFICE_BLIND]}
        houseLevel={75}
        overrides={overrides}
        onChange={onChange}
        row={rowStub()}
      />,
    );
    const field = screen.getByLabelText("Blind opens to");
    expect(field).toHaveValue("60");
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("refuses 0 and nonsense without calling onChange", () => {
    const onChange = vi.fn();
    render(
      <BlindsCard
        blinds={[OFFICE_BLIND]}
        houseLevel={75}
        overrides={[]}
        onChange={onChange}
        row={rowStub()}
      />,
    );
    for (const bad of ["0", "abc"]) {
      const field = screen.getByLabelText("Blind opens to");
      fireEvent.change(field, { target: { value: bad } });
      fireEvent.blur(field);
      expect(screen.getByText("A blind opens to 1–100%.")).toBeInTheDocument();
    }
    expect(onChange).not.toHaveBeenCalled();
  });

  it("replaces a bare-ID override without duplicating the device", () => {
    const onChange = vi.fn();
    const other = { device: "x_shades:office/blind", normal_open: 80 };
    render(
      <BlindsCard
        blinds={[OFFICE_BLIND]}
        houseLevel={75}
        overrides={[{ device: "office/blind", normal_open: 60 }, other]}
        onChange={onChange}
        row={rowStub()}
      />,
    );
    const field = screen.getByLabelText("Blind opens to");
    expect(field).toHaveValue("60");
    fireEvent.change(field, { target: { value: "65" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([
      { device: "z2m:office/blind", normal_open: 65 },
      other,
    ]);
  });
});
