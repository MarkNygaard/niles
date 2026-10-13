import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import {
  AnchorEditor,
  SHIPPED_ANCHORS,
  anchorProblems,
  sortedAnchors,
} from "@/components/AnchorEditor";
import type { Anchor, AnchorEditorProps } from "@/components/AnchorEditor";

const THREE: Anchor[] = [
  { time: "00:00", kelvin: 2000 },
  { time: "12:00", kelvin: 4500 },
  { time: "23:00", kelvin: 2200 },
];

function setup(props: Partial<AnchorEditorProps> = {}) {
  const onSave = vi.fn();
  const onReset = vi.fn();
  const onPreview = vi.fn();
  render(
    <AnchorEditor
      anchors={THREE}
      shipped={SHIPPED_ANCHORS}
      overridden={false}
      hot
      onSave={onSave}
      onReset={onReset}
      onPreview={onPreview}
      {...props}
    />,
  );
  return { onSave, onReset, onPreview };
}

const type = (label: string, value: string) =>
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
const saveButton = () => screen.getByRole("button", { name: "Save" });

describe("AnchorEditor", () => {
  it("edits a point and saves the whole list as one value", () => {
    const { onSave } = setup();
    type("Point 2 kelvin", "5000");
    fireEvent.click(saveButton());
    expect(onSave).toHaveBeenCalledWith([
      { time: "00:00", kelvin: 2000 },
      { time: "12:00", kelvin: 5000 },
      { time: "23:00", kelvin: 2200 },
    ]);
  });

  it("adds a point, and saves the list in time order", () => {
    // The scheduler refuses a list that is not ascending, and a new
    // point is typed at the bottom whatever its time.
    const { onSave } = setup();
    fireEvent.click(screen.getByRole("button", { name: /Add a point/ }));
    type("Point 4 time", "06:30");
    type("Point 4 kelvin", "2700");
    fireEvent.click(saveButton());
    expect(onSave).toHaveBeenCalledWith([
      { time: "00:00", kelvin: 2000 },
      { time: "06:30", kelvin: 2700 },
      { time: "12:00", kelvin: 4500 },
      { time: "23:00", kelvin: 2200 },
    ]);
  });

  it("removes a point", () => {
    const { onSave } = setup();
    fireEvent.click(screen.getByRole("button", { name: "Remove point 2" }));
    fireEvent.click(saveButton());
    expect(onSave).toHaveBeenCalledWith([
      { time: "00:00", kelvin: 2000 },
      { time: "23:00", kelvin: 2200 },
    ]);
  });

  it("keeps the last point, which the curve cannot do without", () => {
    setup({ anchors: [{ time: "00:00", kelvin: 2000 }] });
    expect(screen.getByRole("button", { name: "Remove point 1" })).toBeDisabled();
  });

  it("refuses a time that is not HH:MM", () => {
    const { onSave } = setup();
    type("Point 2 time", "25:00");
    expect(screen.getByText(/Use HH:MM/)).toBeInTheDocument();
    expect(saveButton()).toBeDisabled();
    fireEvent.click(saveButton());
    expect(onSave).not.toHaveBeenCalled();
  });

  it("refuses a kelvin outside what the scheduler accepts", () => {
    setup();
    type("Point 2 kelvin", "12000");
    expect(screen.getByText(/1000 to 10000/)).toBeInTheDocument();
    expect(saveButton()).toBeDisabled();
  });

  it("refuses two points at one time", () => {
    setup();
    type("Point 3 time", "12:00");
    expect(screen.getByText(/already at 12:00/)).toBeInTheDocument();
    expect(saveButton()).toBeDisabled();
  });

  it("hands the chart the edited list while it is valid, and nothing while it is not", () => {
    const { onPreview } = setup();
    type("Point 2 kelvin", "5000");
    expect(onPreview).toHaveBeenLastCalledWith([
      { time: "00:00", kelvin: 2000 },
      { time: "12:00", kelvin: 5000 },
      { time: "23:00", kelvin: 2200 },
    ]);
    type("Point 2 kelvin", "50");
    expect(onPreview).toHaveBeenLastCalledWith(null);
  });

  it("offers a reset to Niles's defaults only for a list that was set", () => {
    const { onReset } = setup({ overridden: true });
    fireEvent.click(screen.getByRole("button", { name: /Reset to Niles's defaults/ }));
    expect(onReset).toHaveBeenCalled();
  });

  it("has no reset when nothing was set", () => {
    setup({ overridden: false });
    expect(screen.queryByRole("button", { name: /Reset to Niles's defaults/ })).toBeNull();
  });

  it("says what Niles ships with when the list differs from it", () => {
    setup();
    expect(screen.getByText(/05:45 2000K/)).toBeInTheDocument();
  });

  it("says nothing about the default when the list is the default", () => {
    setup({ anchors: SHIPPED_ANCHORS });
    expect(screen.queryByText(/ships with/)).toBeNull();
  });
});

describe("anchorProblems", () => {
  it("passes a good list", () => {
    expect(anchorProblems([{ time: "06:30", kelvin: "2700" }])).toEqual([null]);
  });

  it("wants two-digit hours, as HH:MM says", () => {
    expect(anchorProblems([{ time: "6:30", kelvin: "2700" }])[0]).toMatch(/HH:MM/);
  });

  it("takes both ends of the kelvin range and nothing past them", () => {
    const at = (kelvin: string) => anchorProblems([{ time: "06:30", kelvin }])[0];
    expect(at("1000")).toBeNull();
    expect(at("10000")).toBeNull();
    expect(at("999")).not.toBeNull();
    expect(at("10001")).not.toBeNull();
    expect(at("2700.5")).not.toBeNull();
  });
});

describe("sortedAnchors", () => {
  it("orders by time and sends numbers", () => {
    expect(
      sortedAnchors([
        { time: "12:00", kelvin: "4500" },
        { time: " 00:00", kelvin: "2000 " },
      ]),
    ).toEqual([
      { time: "00:00", kelvin: 2000 },
      { time: "12:00", kelvin: 4500 },
    ]);
  });
});
