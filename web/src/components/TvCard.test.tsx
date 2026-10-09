import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { TvCard, tvLine } from "./TvCard";
import type { TvInfo } from "@/lib/api";

function tv(status: TvInfo["status"], paired = true): TvInfo {
  return { configured: true, paired, mac: null, room: "living_room", status, error: null };
}

describe("TvCard", () => {
  it("says what is on", () => {
    expect(tvLine(tv({ on: true, app: "Netflix" }))).toBe("Netflix");
    expect(tvLine(tv({ on: true, app: null }))).toBe("On");
    expect(tvLine(tv({ on: false, app: null }))).toBe("Off");
    expect(tvLine(tv(null))).toBe("Off");
  });

  it("turns an off TV on, and an on TV off", () => {
    const onPower = vi.fn();
    const { rerender } = render(<TvCard tv={tv({ on: false, app: null })} onPower={onPower} />);
    fireEvent.click(screen.getByRole("button", { name: "Turn the TV on" }));
    expect(onPower).toHaveBeenLastCalledWith(true);
    rerender(<TvCard tv={tv({ on: true, app: "YouTube" })} onPower={onPower} />);
    fireEvent.click(screen.getByRole("button", { name: "Turn the TV off" }));
    expect(onPower).toHaveBeenLastCalledWith(false);
  });

  it("is not there before the TV is paired", () => {
    const { container } = render(<TvCard tv={tv(null, false)} onPower={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });
});
