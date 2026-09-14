import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MorningCard } from "@/components/MorningCard";

function setup(props: Partial<React.ComponentProps<typeof MorningCard>> = {}) {
  const onToggle = vi.fn();
  const onDays = vi.fn();
  render(
    <MorningCard
      enabled
      fireDays={["mon", "tue", "wed", "thu", "fri"]}
      start="05:45"
      end="06:30"
      onToggle={onToggle}
      onDays={onDays}
      row={(spec) => <div key={spec.label}>{spec.label}</div>}
      {...props}
    />,
  );
  return { onToggle, onDays };
}

describe("MorningCard", () => {
  it("says when it runs, since the window lives with the curve", () => {
    // Switch the curve off and its rows are hidden — this was the only
    // feature still using those times, with nowhere to see them.
    setup();
    expect(screen.getByText(/05:45 → 06:30/)).toBeInTheDocument();
  });

  it("hides the settings when it is off, rather than greying them out", () => {
    setup({ enabled: false });
    expect(screen.queryByLabelText("Monday")).toBeNull();
    expect(screen.getByRole("switch")).not.toBeChecked();
  });

  it("turns on without being told anything else", () => {
    const { onToggle } = setup({ enabled: false });
    fireEvent.click(screen.getByRole("switch"));
    // Base UI hands the handler an event-details object as well, which
    // the panel ignores.
    expect(onToggle.mock.calls[0][0]).toBe(true);
  });

  it("drops a day that was on", () => {
    const { onDays } = setup();
    fireEvent.click(screen.getByLabelText("Wednesday"));
    expect(onDays).toHaveBeenCalledWith(["mon", "tue", "thu", "fri"]);
  });

  it("keeps the week in order whichever day is added", () => {
    // Appending would store ["sat", "mon"], which reads as nonsense in
    // the config file and in the log line at startup.
    const { onDays } = setup({ fireDays: ["sat"] });
    fireEvent.click(screen.getByLabelText("Monday"));
    expect(onDays).toHaveBeenCalledWith(["mon", "sat"]);
  });

  it("says outright that no days means it never fires", () => {
    setup({ fireDays: [] });
    expect(screen.getByText(/never fires/)).toBeInTheDocument();
  });

  it("marks the days that are on for a screen reader, not just with a tint", () => {
    setup({ fireDays: ["mon"] });
    expect(screen.getByLabelText("Monday")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByLabelText("Saturday")).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });
});
