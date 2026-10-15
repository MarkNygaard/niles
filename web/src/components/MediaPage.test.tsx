import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
import { ApiError, api } from "@/lib/api";
import type { MediaGroup, MediaSpeaker, MediaView, RoomMusic, TvInfo } from "@/lib/api";

function room(name: string, playing: boolean): RoomMusic {
  return { room: name, playing, what: null, kind: playing ? "music" : null, volume: 20 };
}

function speaker(id: string, name: string): MediaSpeaker {
  return { id, name, room: null, volume: 20, soundbar: false };
}

function group(kind: MediaGroup["kind"], leader: string, speakers: MediaSpeaker[]): MediaGroup {
  return { leader, kind, what: null, playing: false, speakers };
}

const bar = { ...speaker("RINCON_BAR", "Living Room"), soundbar: true };
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

const noTv: TvInfo = {
  configured: false,
  paired: false,
  mac: null,
  room: null,
  status: null,
  error: null,
};

const tvOn: TvInfo = {
  configured: true,
  paired: true,
  mac: null,
  room: "living_room",
  status: { on: true, app: "Netflix", volume: 14, muted: false, sound_output: "tv_speaker" },
  error: null,
};

describe("the Media page", () => {
  afterEach(() => vi.restoreAllMocks());

  it("keeps the radio and Spotify for a house with Sonos", async () => {
    page(view, tvOn);
    expect(await screen.findByRole("region", { name: "Radio" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Spotify" })).toBeInTheDocument();
    const tv = await screen.findByRole("region", { name: "TV" });
    expect(
      within(tv).getByRole("button", { name: /The TV's sound to the soundbar/ }),
    ).toBeInTheDocument();
  });

  it("puts the TV's own volume on its card when the Sonos are not a soundbar", async () => {
    // A speaker in the office and a TV playing through its own: the
    // soundbar button there sent the sound nowhere, and the volume
    // that worked was only on the room's card.
    page({ groups: [group("music", "RINCON_MOVE", [move])], idle: [] }, tvOn);
    const tv = await screen.findByRole("region", { name: "TV" });
    expect(within(tv).getByRole("slider", { name: "TV volume" })).toBeInTheDocument();
    expect(within(tv).queryByRole("button", { name: /soundbar/ })).toBeNull();
    expect(screen.getByRole("region", { name: "Radio" })).toBeInTheDocument();
  });

  it("leaves the TV's sound alone when turning it on without a soundbar", async () => {
    page(
      { groups: [group("music", "RINCON_MOVE", [move])], idle: [] },
      { ...tvOn, status: { on: false, app: null } },
    );
    const power = vi.spyOn(api, "tvPower").mockResolvedValue({ on: true });
    const sound = vi.spyOn(api, "tvSound").mockResolvedValue(undefined);
    const tv = await screen.findByRole("region", { name: "TV" });
    fireEvent.click(within(tv).getByRole("button", { name: "TV — off" }));
    await waitFor(() => expect(power).toHaveBeenCalledWith(true));
    expect(sound).not.toHaveBeenCalled();
  });

  function page(house: MediaView = view, tv: TvInfo = noTv) {
    vi.spyOn(api, "media").mockResolvedValue(house);
    vi.spyOn(api, "tv").mockResolvedValue(tv);
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
    page({ groups: [view.groups[0]], idle: [back, move] });
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

  it("has one play button for a card that plays", async () => {
    page();
    const spotify = await screen.findByRole("region", { name: "Spotify" });
    expect(within(spotify).queryByRole("button", { name: /^Play$/ })).toBeNull();
    expect(
      within(spotify).getByRole("button", { name: "Play Living Room, Walk In Closet" }),
    ).toBeInTheDocument();
  });

  it("opens the picker on a group's speakers from what it plays", async () => {
    page();
    vi.spyOn(api, "mediaChoices").mockResolvedValue([]);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "Play something else on Living Room, Walk In Closet",
      }),
    );
    expect(await screen.findByRole("checkbox", { name: /Walk In Closet/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByRole("checkbox", { name: /Living Room Back/ })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });
});

describe("the Media page without Sonos", () => {
  afterEach(() => vi.restoreAllMocks());

  const noSonos = () => Promise.reject(new ApiError("there are no speakers set up", 501));

  function page(
    tv: () => Promise<TvInfo>,
    media: () => Promise<MediaView> = noSonos,
  ) {
    vi.spyOn(api, "media").mockImplementation(media);
    vi.spyOn(api, "tv").mockImplementation(tv);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <MediaPage />
      </QueryClientProvider>,
    );
  }

  it("puts the TV's own volume on its card", async () => {
    page(() => Promise.resolve(tvOn));
    const set = vi
      .spyOn(api, "tvVolume")
      .mockResolvedValue({ volume: 15, muted: false, sound_output: "tv_speaker" });
    const tv = await screen.findByRole("region", { name: "TV" });
    expect(within(tv).getByText("Netflix")).toBeInTheDocument();
    expect(within(tv).getByRole("slider", { name: "TV volume" })).toBeInTheDocument();
    fireEvent.click(within(tv).getByRole("button", { name: "TV volume up" }));
    await waitFor(() => expect(set).toHaveBeenCalledWith({ delta: 1 }));
    fireEvent.click(within(tv).getByRole("button", { name: "Mute the TV" }));
    await waitFor(() => expect(set).toHaveBeenCalledWith({ mute: true }));
  });

  it("has nothing that needs a Sonos", async () => {
    page(() => Promise.resolve(tvOn));
    await screen.findByRole("region", { name: "TV" });
    expect(screen.queryByRole("button", { name: /soundbar/ })).toBeNull();
    expect(screen.queryByRole("region", { name: "Radio" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Spotify" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Not playing" })).toBeNull();
  });

  it("turns the TV on without sending its sound to a soundbar", async () => {
    page(() => Promise.resolve({ ...tvOn, status: { on: false, app: null } }));
    const power = vi.spyOn(api, "tvPower").mockResolvedValue({ on: true });
    const sound = vi.spyOn(api, "tvSound").mockResolvedValue(undefined);
    const tv = await screen.findByRole("region", { name: "TV" });
    expect(within(tv).queryByRole("slider", { name: "TV volume" })).toBeNull();
    fireEvent.click(within(tv).getByRole("button", { name: "TV — off" }));
    await waitFor(() => expect(power).toHaveBeenCalledWith(true));
    expect(sound).not.toHaveBeenCalled();
  });

  it("says when the TV did not answer the wake-up", async () => {
    page(() => Promise.resolve({ ...tvOn, status: { on: false, app: null } }));
    const message = "Sent the wake signal, but the TV hasn't answered: check the wake MAC.";
    vi.spyOn(api, "tvPower").mockResolvedValue({ on: false, message });
    const tv = await screen.findByRole("region", { name: "TV" });
    fireEvent.click(within(tv).getByRole("button", { name: "TV — off" }));
    expect(await screen.findByRole("status")).toHaveTextContent(message);
  });

  it("says where to add something when there is nothing to play on", async () => {
    page(() => Promise.resolve(noTv));
    const link = await screen.findByRole("link", { name: /Add Sonos speakers or an LG TV/ });
    expect(link).toHaveAttribute("href", "#/me/settings/addons");
    expect(screen.getByText("Nothing to play on yet.")).toBeInTheDocument();
  });

  it("draws neither layout before the speakers have answered", async () => {
    let answer: (view: MediaView) => void = () => {};
    page(
      () => Promise.resolve(tvOn),
      () => new Promise((resolve) => (answer = resolve)),
    );
    await waitFor(() => expect(api.tv).toHaveBeenCalled());
    // Long enough for the TV to have answered.
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(screen.queryByRole("region", { name: "TV" })).toBeNull();
    expect(screen.queryByRole("link", { name: /Add Sonos/ })).toBeNull();
    act(() => answer(view));
    expect(await screen.findByRole("region", { name: "Radio" })).toBeInTheDocument();
  });

  it("waits for the TV before saying there is nothing", async () => {
    let answer: (tv: TvInfo) => void = () => {};
    page(() => new Promise((resolve) => (answer = resolve)));
    await waitFor(() => expect(api.media).toHaveBeenCalled());
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(screen.queryByRole("link", { name: /Add Sonos/ })).toBeNull();
    act(() => answer(tvOn));
    expect(await screen.findByRole("region", { name: "TV" })).toBeInTheDocument();
  });
});
