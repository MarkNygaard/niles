import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { VoicesCard } from "@/components/VoicesCard";

function setup(props: Partial<React.ComponentProps<typeof VoicesCard>> = {}) {
  const onChange = vi.fn();
  render(
    <VoicesCard
      knownVoicesOnly={false}
      recognitionOn={true}
      onChange={onChange}
      {...props}
    />,
  );
  return { onChange };
}

describe("VoicesCard", () => {
  it("is off until somebody turns it on", () => {
    // A house that upgrades into this setting must not find itself
    // locked by it.
    setup();
    expect(screen.getByRole("switch")).not.toBeChecked();
  });

  it("writes the switch straight through", () => {
    const { onChange } = setup();
    fireEvent.click(screen.getByRole("switch"));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("says how to add somebody before you need to know", () => {
    // The chicken-and-egg: with the lock on, a new voice cannot
    // introduce itself either. Saying so only after somebody is stuck
    // is saying it too late.
    setup({ knownVoicesOnly: true });
    expect(screen.getByText(/switch this off/i)).toBeInTheDocument();
  });

  it("does not explain the way out when there is no way in", () => {
    setup({ knownVoicesOnly: false });
    expect(screen.queryByText(/switch this off/i)).not.toBeInTheDocument();
  });

  it("admits when the switch governs nothing", () => {
    // Recognition is not running, so the lock cannot lock. A switch
    // that looks obeyed and is not is worse than one that says so.
    setup({ recognitionOn: false });
    expect(screen.getByText(/not set up to recognise voices/i)).toBeInTheDocument();
  });

  it("stays quiet about that once recognition is running", () => {
    setup({ recognitionOn: true });
    expect(
      screen.queryByText(/not set up to recognise voices/i),
    ).not.toBeInTheDocument();
  });
});
