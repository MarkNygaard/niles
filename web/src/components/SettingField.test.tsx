import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { SettingField } from "@/components/SettingField";

function setup(props: Partial<React.ComponentProps<typeof SettingField>> = {}) {
  const onSave = vi.fn();
  const onReset = vi.fn();
  render(
    <SettingField
      path="lighting.ambient_brightness"
      label="Brightness"
      kind="number"
      value={undefined}
      overridden={false}
      hot
      onSave={onSave}
      onReset={onReset}
      {...props}
    />,
  );
  return {
    onSave,
    onReset,
    input: screen.getByLabelText(props.label ?? "Brightness"),
    saveButton: screen.getByRole("button", { name: "Save" }),
  };
}

function type(input: HTMLElement, value: string) {
  fireEvent.change(input, { target: { value } });
}

describe("SettingField", () => {
  it("offers a setting that is not configured yet", () => {
    // The whole point: an optional value nobody has set is exactly what
    // someone opens this page to set.
    const { input, saveButton, onSave } = setup();
    expect(input).toHaveValue("");
    expect(saveButton).toBeDisabled();

    type(input, "25");
    expect(saveButton).toBeEnabled();
    fireEvent.click(saveButton);
    expect(onSave).toHaveBeenCalledWith(25);
  });

  it("sends a number as a number even when the current value is unset", () => {
    const { input, onSave } = setup({ value: undefined });
    type(input, "2200");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSave).toHaveBeenCalledWith(2200);
  });

  it("refuses to save an emptied number, which would write zero", () => {
    const { input, saveButton, onSave } = setup({ value: 25 });
    type(input, "");
    expect(saveButton).toBeDisabled();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSave).not.toHaveBeenCalled();
  });

  it("refuses a number that isn't one", () => {
    const { input, saveButton } = setup({ value: 25 });
    type(input, "bright");
    expect(saveButton).toBeDisabled();
  });

  it("edits a list as comma-separated ids", () => {
    const { input, onSave } = setup({
      path: "ambient_lights.devices",
      label: "Ambient lights",
      kind: "list",
      value: ["living_room/tv_lightstrip"],
    });
    expect(input).toHaveValue("living_room/tv_lightstrip");

    type(input, "living_room/tv_lightstrip, office/lamp");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSave).toHaveBeenCalledWith([
      "living_room/tv_lightstrip",
      "office/lamp",
    ]);
  });

  it("lets a list be emptied, because no ambient lights is a choice", () => {
    const { input, saveButton, onSave } = setup({
      path: "ambient_lights.devices",
      label: "Ambient lights",
      kind: "list",
      value: ["living_room/tv_lightstrip"],
    });
    type(input, "");
    expect(saveButton).toBeEnabled();
    fireEvent.click(saveButton);
    expect(onSave).toHaveBeenCalledWith([]);
  });

  it("says when a value only takes effect after a restart", () => {
    setup({ hot: false });
    expect(screen.getByText("restart required")).toBeInTheDocument();
  });
});
