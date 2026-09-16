import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { TadoPanel } from "@/components/TadoPanel";
import type { TadoStatus } from "@/lib/api";

const CONNECTED: TadoStatus = {
  connectable: true,
  authorised: true,
  presence_enabled: true,
};

function setup(
  lights = { offWhenAway: false, onWhenHome: [] as string[] },
  status: TadoStatus = CONNECTED,
) {
  const onLightsChange = vi.fn();
  render(
    <TadoPanel
      status={status}
      lights={lights}
      lightOptions={[
        { id: "z2m:hall/lamp", value: "hall/lamp", label: "Hall lamp", room: "hall" },
      ]}
      onLightsChange={onLightsChange}
      onToggle={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  return { onLightsChange };
}

describe("TadoPanel's lights", () => {
  it("asks for the lights to go off when everyone leaves", () => {
    const { onLightsChange } = setup();
    fireEvent.click(
      screen.getByRole("switch", { name: /Lights off when everyone leaves/ }),
    );
    expect(onLightsChange).toHaveBeenCalledWith([
      { path: "presence.lights_off_when_away", value: true },
    ]);
  });

  it("shows what is already set rather than a default", () => {
    setup({ offWhenAway: true, onWhenHome: ["hall/lamp"] });
    expect(
      screen.getByRole("switch", { name: /Lights off when everyone leaves/ }),
    ).toBeChecked();
  });

  it("offers none of it until presence is switched on", () => {
    // Reading who is home and acting on it are two decisions, and the
    // second is not implied by the first.
    setup({ offWhenAway: false, onWhenHome: [] }, {
      ...CONNECTED,
      presence_enabled: false,
    });
    expect(
      screen.queryByRole("switch", { name: /Lights off when everyone leaves/ }),
    ).toBeNull();
  });
});
