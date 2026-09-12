import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import {
  ColorField,
  hsvToRgb,
  parseHex,
  rgbToHsv,
  toHex,
} from "@/components/ColorField";

describe("colour conversion", () => {
  it("round-trips a colour through HSV and back", () => {
    // The wheel picks in HSV and stores hex; a colour that shifted on
    // the way round would drift every time the marker was redrawn.
    for (const hex of ["#ff8000", "#2200aa", "#ffffff", "#000000", "#3d7f2a"]) {
      const rgb = parseHex(hex)!;
      const { hue, saturation, value } = rgbToHsv(...rgb);
      expect(toHex(hsvToRgb(hue, saturation, value))).toBe(hex);
    }
  });

  it("reads a colour with or without the hash", () => {
    expect(parseHex("#ff8000")).toEqual([255, 128, 0]);
    expect(parseHex("ff8000")).toEqual([255, 128, 0]);
    expect(parseHex("  #FF8000 ")).toEqual([255, 128, 0]);
  });

  it("refuses anything that isn't a colour", () => {
    expect(parseHex("burnt orange")).toBeNull();
    expect(parseHex("#fff")).toBeNull();
    expect(parseHex("#ff80")).toBeNull();
    expect(parseHex("")).toBeNull();
  });

  it("puts full saturation at the rim and none at the centre", () => {
    expect(hsvToRgb(0, 1, 1)).toEqual([255, 0, 0]);
    expect(hsvToRgb(0, 0, 1)).toEqual([255, 255, 255]);
    expect(hsvToRgb(120, 1, 1)).toEqual([0, 255, 0]);
    expect(hsvToRgb(240, 1, 1)).toEqual([0, 0, 255]);
  });
});

describe("ColorField", () => {
  function setup(value: string) {
    const onChange = vi.fn();
    render(
      <ColorField
        id="lighting.ambient_color"
        aria-label="Colour"
        value={value}
        onChange={onChange}
      />,
    );
    return { onChange };
  }

  it("shows the colour it was given", () => {
    setup("#ff8000");
    expect(screen.getByLabelText("Colour")).toHaveAttribute(
      "aria-valuetext",
      "#ff8000",
    );
  });

  it("says when nothing is set", () => {
    setup("");
    expect(screen.getByLabelText("Colour")).toHaveAttribute(
      "aria-valuetext",
      "not set",
    );
  });

  it("accepts a colour typed as hex", () => {
    const { onChange } = setup("");
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "#2200aa" },
    });
    expect(onChange).toHaveBeenCalledWith("#2200aa");
  });

  it("waits for a complete colour before saving", () => {
    // Half-typed hex would be refused by the server on every keystroke.
    const { onChange } = setup("");
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "#22" } });
    expect(onChange).not.toHaveBeenCalled();
  });
});
