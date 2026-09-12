import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { SettingRow } from "@/components/SettingRow";
import type { SettingRowProps } from "@/components/SettingRow";

const RAMP = [
  { path: "lighting.morning_start", kind: "text" as const, caption: "starts" },
  { path: "lighting.morning_end", kind: "text" as const, caption: "ends" },
];

function setup(props: Partial<SettingRowProps> = {}) {
  const onSave = vi.fn();
  const onReset = vi.fn();
  render(
    <SettingRow
      label="Morning ramp"
      description="Lights come up across this window."
      settings={RAMP}
      values={{
        "lighting.morning_start": "05:20",
        "lighting.morning_end": "06:10",
      }}
      overridden={[]}
      hot
      onSave={onSave}
      onReset={onReset}
      {...props}
    />,
  );
  return { onSave, onReset };
}

const input = (name: string) => screen.getByLabelText(name);
const saveButton = () => screen.getByRole("button", { name: "Save" });
const type = (el: HTMLElement, value: string) =>
  fireEvent.change(el, { target: { value } });

describe("SettingRow", () => {
  it("saves everything the row changed in one write", () => {
    // The point of a row: a ramp that moved is one change, one
    // revision, and one undo — not two of each.
    const { onSave } = setup();
    type(input("Morning ramp starts"), "06:00");
    type(input("Morning ramp ends"), "06:45");
    fireEvent.click(saveButton());

    expect(onSave).toHaveBeenCalledWith([
      { path: "lighting.morning_start", value: "06:00" },
      { path: "lighting.morning_end", value: "06:45" },
    ]);
  });

  it("sends only the values that actually changed", () => {
    const { onSave } = setup();
    type(input("Morning ramp ends"), "06:45");
    fireEvent.click(saveButton());

    expect(onSave).toHaveBeenCalledWith([
      { path: "lighting.morning_end", value: "06:45" },
    ]);
  });

  it("has nothing to save until something changes", () => {
    setup();
    expect(saveButton()).toBeDisabled();
  });

  it("offers a setting that is not configured yet", () => {
    // The whole reason this renders unset values: an optional setting
    // nobody has set is exactly what someone opens the page to set.
    const { onSave } = setup({
      label: "Held at",
      settings: [
        {
          path: "lighting.ambient_brightness",
          kind: "number",
          caption: "brightness %",
        },
      ],
      values: { "lighting.ambient_brightness": undefined },
    });
    expect(screen.getByText("not set")).toBeInTheDocument();

    type(input("Held at brightness %"), "25");
    fireEvent.click(saveButton());
    expect(onSave).toHaveBeenCalledWith([
      { path: "lighting.ambient_brightness", value: 25 },
    ]);
  });

  it("refuses to save an emptied number, which would write zero", () => {
    const { onSave } = setup({
      label: "Brightness",
      settings: [
        {
          path: "lighting.daytime_brightness",
          kind: "number",
          caption: "day %",
        },
      ],
      values: { "lighting.daytime_brightness": 100 },
    });
    type(input("Brightness day %"), "");
    expect(saveButton()).toBeDisabled();
    expect(onSave).not.toHaveBeenCalled();
  });

  it("refuses a number that isn't one", () => {
    setup({
      label: "Brightness",
      settings: [
        {
          path: "lighting.daytime_brightness",
          kind: "number",
          caption: "day %",
        },
      ],
      values: { "lighting.daytime_brightness": 100 },
    });
    type(input("Brightness day %"), "bright");
    expect(saveButton()).toBeDisabled();
  });

  it("writes as soon as a light is removed, with no Save to press", () => {
    // Picking a light is already a deliberate act; a Save button after
    // it is ceremony. Typing into a box is not, which is why only this
    // kind of row saves on the spot.
    const { onSave } = setup({
      label: "Ambient lights",
      settings: [
        {
          path: "ambient_lights.devices",
          kind: "devices",
          caption: "pick from the lights Niles knows about",
          options: [
            {
              value: "wled:living_room/tv_light",
              label: "Tv light",
              room: "Living room",
              source: "wled",
              supportsRgb: true,
              supportsColorTemp: false,
            },
          ],
        },
      ],
      values: { "ambient_lights.devices": ["wled:living_room/tv_light"] },
    });
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();

    fireEvent.click(
      screen.getByRole("button", { name: "Remove Living room Tv light" }),
    );
    expect(onSave).toHaveBeenCalledWith([
      { path: "ambient_lights.devices", value: [] },
    ]);
  });

  it("does not badge an empty picker as unset — the empty box says so", () => {
    setup({
      label: "Ambient lights",
      settings: [
        {
          path: "ambient_lights.devices",
          kind: "devices",
          caption: "pick from the lights Niles knows about",
          options: [],
        },
      ],
      values: { "ambient_lights.devices": undefined },
    });
    expect(screen.queryByText("not set")).toBeNull();
  });

  it("resets every overridden value in the row at once", () => {
    const { onReset } = setup({ overridden: RAMP.map((s) => s.path) });
    fireEvent.click(screen.getByRole("button", { name: "Reset to the config file" }));
    expect(onReset).toHaveBeenCalledWith([
      "lighting.morning_start",
      "lighting.morning_end",
    ]);
  });

  it("says when a value only takes effect after a restart", () => {
    setup({ hot: false });
    expect(screen.getByText("restart required")).toBeInTheDocument();
  });

  it("shows the server's refusal verbatim", () => {
    setup({ error: "lighting.ambient_kelvin 500K is outside 1000..=10000" });
    expect(
      screen.getByText("lighting.ambient_kelvin 500K is outside 1000..=10000"),
    ).toBeInTheDocument();
  });
});
