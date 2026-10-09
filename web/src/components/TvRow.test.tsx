import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { TvRow, tvLine, tvOn } from "./TvRow";
import type { TvInfo } from "@/lib/api";

function tv(status: TvInfo["status"]): TvInfo {
  return { configured: true, paired: true, mac: null, room: "living_room", status, error: null };
}

describe("TvRow", () => {
  it("says what is on", () => {
    expect(tvLine(tv({ on: true, app: "Netflix" }))).toBe("Netflix");
    expect(tvLine(tv({ on: true, app: null }))).toBe("On");
    expect(tvLine(tv({ on: false, app: null }))).toBe("Off");
    expect(tvLine(tv(null))).toBe("Off");
    expect(tvOn(tv({ on: true, app: null }))).toBe(true);
    expect(tvOn(undefined)).toBe(false);
  });

  it("turns an off TV on, and an on TV off", () => {
    const onPower = vi.fn();
    const { rerender } = render(<TvRow tv={tv({ on: false, app: null })} onPower={onPower} />);
    fireEvent.click(screen.getByRole("button"));
    expect(onPower).toHaveBeenLastCalledWith(true);
    rerender(<TvRow tv={tv({ on: true, app: "YouTube" })} onPower={onPower} />);
    fireEvent.click(screen.getByRole("button"));
    expect(onPower).toHaveBeenLastCalledWith(false);
  });
});
