import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MusicRow, musicLine, musicPlaying } from "./MusicRow";
import type { RoomMusic } from "@/lib/api";

function music(over: Partial<RoomMusic>): RoomMusic {
  return {
    room: "living_room",
    playing: true,
    what: "Chariot by Gavin DeGraw",
    kind: "music",
    volume: 30,
    ...over,
  };
}

describe("MusicRow", () => {
  it("says what plays", () => {
    expect(musicLine(music({}))).toBe("Chariot by Gavin DeGraw");
    expect(musicLine(music({ kind: "radio", what: null }))).toBe("The radio");
    expect(musicLine(music({ kind: "tv", what: null }))).toBe("The TV's sound");
    expect(musicLine(music({ playing: false }))).toBe("Paused");
  });

  it("marks music and the radio, not the TV's sound", () => {
    expect(musicPlaying(music({}))).toBe(true);
    expect(musicPlaying(music({ kind: "radio" }))).toBe(true);
    expect(musicPlaying(music({ kind: "tv" }))).toBe(false);
    expect(musicPlaying(music({ playing: false }))).toBe(false);
    expect(musicPlaying(undefined)).toBe(false);
  });

  it("pauses what plays and plays what is paused", () => {
    const onPause = vi.fn();
    const onPlay = vi.fn();
    const props = { onPause, onPlay, onVolume: vi.fn() };
    const { rerender } = render(<MusicRow music={music({})} {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Pause the music" }));
    expect(onPause).toHaveBeenCalled();
    rerender(<MusicRow music={music({ playing: false })} {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Play the music" }));
    expect(onPlay).toHaveBeenCalled();
  });
});
