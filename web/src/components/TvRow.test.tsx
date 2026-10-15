import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { TvRow, afterChange, tvLine, tvOn, tvVolume, withTvVolume } from "./TvRow";
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

describe("TvRow volume", () => {
  const playing = tv({ on: true, app: "Netflix", volume: 14, muted: false });

  it("is only there while the TV is on and says how loud it is", () => {
    expect(tvVolume(playing)).toEqual({ volume: 14, muted: false });
    expect(tvVolume(tv({ on: false, app: null }))).toBeNull();
    expect(tvVolume(tv({ on: true, app: null }))).toBeNull();
    expect(tvVolume(undefined)).toBeNull();
    // Sent to a soundbar, the TV's own number is not what anyone hears.
    expect(
      tvVolume(tv({ on: true, app: null, volume: 14, sound_output: "external_arc" })),
    ).toBeNull();
    expect(
      tvVolume(tv({ on: true, app: null, volume: 14, sound_output: "tv_speaker" })),
    ).toEqual({ volume: 14, muted: false });

    const { rerender } = render(
      <TvRow tv={tv({ on: false, app: null })} onPower={vi.fn()} onVolume={vi.fn()} />,
    );
    expect(screen.queryByRole("slider")).not.toBeInTheDocument();
    rerender(<TvRow tv={playing} onPower={vi.fn()} />);
    expect(screen.queryByRole("slider")).not.toBeInTheDocument();
    rerender(<TvRow tv={playing} onPower={vi.fn()} onVolume={vi.fn()} />);
    expect(screen.getByRole("slider", { name: "TV volume" })).toHaveAttribute(
      "aria-valuenow",
      "14",
    );
  });

  it("steps down and up a notch", () => {
    const onVolume = vi.fn();
    render(<TvRow tv={playing} onPower={vi.fn()} onVolume={onVolume} />);
    fireEvent.click(screen.getByRole("button", { name: "TV volume down" }));
    expect(onVolume).toHaveBeenLastCalledWith({ delta: -1 });
    fireEvent.click(screen.getByRole("button", { name: "TV volume up" }));
    expect(onVolume).toHaveBeenLastCalledWith({ delta: 1 });
  });

  it("mutes, and unmutes a muted TV", () => {
    const onVolume = vi.fn();
    const { rerender } = render(<TvRow tv={playing} onPower={vi.fn()} onVolume={onVolume} />);
    fireEvent.click(screen.getByRole("button", { name: "Mute the TV" }));
    expect(onVolume).toHaveBeenLastCalledWith({ mute: true });
    rerender(
      <TvRow
        tv={tv({ on: true, app: "Netflix", volume: 14, muted: true })}
        onPower={vi.fn()}
        onVolume={onVolume}
      />,
    );
    const unmute = screen.getByRole("button", { name: "Unmute the TV" });
    expect(unmute).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(unmute);
    expect(onVolume).toHaveBeenLastCalledWith({ mute: false });
  });

  it("cannot step past either end", () => {
    const { rerender } = render(
      <TvRow tv={tv({ on: true, app: null, volume: 0 })} onPower={vi.fn()} onVolume={vi.fn()} />,
    );
    expect(screen.getByRole("button", { name: "TV volume down" })).toBeDisabled();
    rerender(
      <TvRow tv={tv({ on: true, app: null, volume: 100 })} onPower={vi.fn()} onVolume={vi.fn()} />,
    );
    expect(screen.getByRole("button", { name: "TV volume up" })).toBeDisabled();
  });

  it("is drawn where a change leaves it before the TV answers", () => {
    const now = { volume: 14, muted: false };
    expect(afterChange(now, { volume: 30 })).toEqual({ volume: 30, muted: false });
    expect(afterChange(now, { delta: -1 })).toEqual({ volume: 13, muted: false });
    expect(afterChange({ volume: 100, muted: false }, { delta: 1 }).volume).toBe(100);
    expect(afterChange({ volume: 0, muted: false }, { delta: -1 }).volume).toBe(0);
    expect(afterChange(now, { mute: true })).toEqual({ volume: 14, muted: true });
  });

  it("keeps the rest of what the TV said when its volume changes", () => {
    const changed = withTvVolume(playing, { volume: 20, muted: true });
    expect(changed?.status).toEqual({ on: true, app: "Netflix", volume: 20, muted: true });
    expect(withTvVolume(tv(null), { volume: 20, muted: false })?.status).toBeNull();
    expect(withTvVolume(undefined, { volume: 20, muted: false })).toBeUndefined();
  });
});
