import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  MediaPage,
  destinations,
  dropAt,
  everySpeaker,
  moveFor,
  somethingPlays,
} from "./MediaPage";
import { choiceSections } from "./StartDialog";
import { api } from "@/lib/api";
import type { MediaGroup, MediaSpeaker, MediaView, RoomMusic } from "@/lib/api";

function room(name: string, playing: boolean): RoomMusic {
  return { room: name, playing, what: null, kind: playing ? "music" : null, volume: 20 };
}

function speaker(id: string, name: string): MediaSpeaker {
  return { id, name, room: null, volume: 20, soundbar: false };
}

function group(kind: MediaGroup["kind"], leader: string, speakers: MediaSpeaker[]): MediaGroup {
  return { leader, kind, what: null, playing: false, speakers };
}

const bar = speaker("RINCON_BAR", "Living Room");
const closet = speaker("RINCON_CLOSET", "Walk In Closet");
const move = speaker("RINCON_MOVE", "Kitchen");
const back = speaker("RINCON_BACK", "Living Room Back");

// This morning's house: the living room and the closet paused together
// with Spotify loaded, the kitchen on the radio, the back speaker idle.
const view: MediaView = {
  groups: [
    group("spotify", "RINCON_BAR", [bar, closet]),
    group("radio", "RINCON_MOVE", [move]),
  ],
  idle: [back],
};

describe("MediaPage", () => {
  it("knows when something plays", () => {
    expect(somethingPlays([room("kitchen", false)], false)).toBe(false);
    expect(somethingPlays([room("kitchen", true)], false)).toBe(true);
    expect(somethingPlays([], true)).toBe(true);
    expect(somethingPlays(undefined, undefined)).toBe(false);
  });

  it("reads where a speaker was put down", () => {
    expect(dropAt("group:RINCON_BAR")).toEqual({ to: "group", leader: "RINCON_BAR" });
    expect(dropAt("idle")).toEqual({ to: "idle" });
    expect(dropAt("start:spotify")).toEqual({ to: "start", source: "spotify" });
    expect(dropAt(null)).toBeNull();
    expect(dropAt("somewhere")).toBeNull();
  });

  it("joins a speaker dropped on another group", () => {
    expect(moveFor(view, "RINCON_BACK", { to: "group", leader: "RINCON_BAR" })).toEqual({
      action: "join",
      leader: "RINCON_BAR",
    });
  });

  it("does nothing for a speaker put back where it was", () => {
    expect(moveFor(view, "RINCON_CLOSET", { to: "group", leader: "RINCON_BAR" })).toBeNull();
    expect(moveFor(view, "RINCON_BACK", { to: "idle" })).toBeNull();
  });

  it("stops a speaker dropped on not playing", () => {
    expect(moveFor(view, "RINCON_CLOSET", { to: "idle" })).toEqual({ action: "leave" });
  });

  it("joins nothing to the TV", () => {
    const tv: MediaView = { groups: [group("tv", "RINCON_BAR", [bar])], idle: [back] };
    expect(moveFor(tv, "RINCON_BACK", { to: "group", leader: "RINCON_BAR" })).toBeNull();
    expect(destinations(tv, "RINCON_BACK").map((d) => d.label)).not.toContain(
      expect.stringContaining("TV"),
    );
  });

  it("offers the moves a drag makes", () => {
    expect(destinations(view, "RINCON_CLOSET").map((d) => d.label)).toEqual([
      "Radio (Kitchen)",
      "Start the radio",
      "Start Spotify",
      "Not playing",
    ]);
  });

  it("lists every speaker by name", () => {
    expect(everySpeaker(view).map((s) => s.name)).toEqual([
      "Kitchen",
      "Living Room",
      "Living Room Back",
      "Walk In Closet",
    ]);
  });

  it("puts carrying on first among the choices", () => {
    const sections = choiceSections([
      { kind: "favorite", id: "Chill", label: "Chill" },
      { kind: "queue", id: "RINCON_BAR", label: "Carry on: Chariot (Living Room)" },
    ]);
    expect(sections.map((s) => s.title)).toEqual(["Carry on", "Favorites"]);
  });
});

describe("the Media page", () => {
  afterEach(() => vi.restoreAllMocks());

  function page() {
    vi.spyOn(api, "media").mockResolvedValue(view);
    vi.spyOn(api, "tv").mockResolvedValue({
      configured: false,
      paired: false,
      mac: null,
      room: null,
      status: null,
      error: null,
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <MediaPage />
      </QueryClientProvider>,
    );
  }

  it("puts a paused group under what it has loaded", async () => {
    page();
    const spotify = await screen.findByRole("region", { name: "Spotify" });
    expect(within(spotify).getByText("Walk In Closet")).toBeInTheDocument();
    expect(within(spotify).getByText("Paused")).toBeInTheDocument();
    const idle = screen.getByRole("region", { name: "Not playing" });
    expect(within(idle).getByText("Living Room Back")).toBeInTheDocument();
  });

  it("stops a speaker moved to not playing", async () => {
    page();
    const control = vi.spyOn(api, "speakerControl").mockResolvedValue(undefined);
    fireEvent.click(await screen.findByRole("button", { name: "Move Walk In Closet" }));
    fireEvent.click(await screen.findByRole("button", { name: "Not playing" }));
    await waitFor(() =>
      expect(control).toHaveBeenCalledWith("RINCON_CLOSET", { action: "leave" }),
    );
  });

  it("asks which speakers before starting the radio", async () => {
    page();
    vi.spyOn(api, "mediaChoices").mockResolvedValue([
      { kind: "favorite", id: "DR P3", label: "DR P3" },
    ]);
    const start = vi.spyOn(api, "mediaStart").mockResolvedValue(undefined);
    const radio = await screen.findByRole("region", { name: "Radio" });
    fireEvent.click(within(radio).getByRole("button", { name: /Play$/ }));
    const p3 = await screen.findByRole("button", { name: "DR P3" });
    expect(p3).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox", { name: /Living Room Back/ }));
    fireEvent.click(p3);
    await waitFor(() =>
      expect(start).toHaveBeenCalledWith(["RINCON_BACK"], {
        kind: "favorite",
        id: "DR P3",
        label: "DR P3",
      }),
    );
  });
});
