import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { AmbientControls } from "@/components/AmbientControls";

function setup(props: Partial<React.ComponentProps<typeof AmbientControls>> = {}) {
  const onChange = vi.fn();
  render(<AmbientControls onChange={onChange} {...props} />);
  return { onChange };
}

describe("AmbientControls", () => {
  it("says what each button holds without opening it", () => {
    // The row has to be readable at a glance, or three unlabelled
    // circles are worse than the three boxes they replaced.
    setup({ brightness: 40, color: "#ff8000", kelvin: 2200 });
    expect(screen.getByRole("button", { name: "Brightness — 40%" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Colour — #ff8000" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Colour temperature — 2200K" }),
    ).toBeInTheDocument();
  });

  it("says when nothing is configured", () => {
    setup();
    expect(screen.getByRole("button", { name: "Brightness — not set" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Colour — not set" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Colour temperature — not set" }),
    ).toBeInTheDocument();
  });

  it("leaves every control reachable while disabled is off", () => {
    setup({ color: "#ff8000" });
    for (const name of [/^Brightness/, /^Colour —/, /^Colour temperature/]) {
      expect(screen.getByRole("button", { name })).toBeEnabled();
    }
  });

  it("disables the lot while a write is in flight", () => {
    setup({ disabled: true });
    for (const name of [/^Brightness/, /^Colour —/, /^Colour temperature/]) {
      expect(screen.getByRole("button", { name })).toBeDisabled();
    }
  });
});
