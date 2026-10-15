import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ConfigPanel, musicByRoom, satellitesAt, sonosAt } from "@/components/ConfigPanel";
import { ApiError, api } from "@/lib/api";

/**
 * The panel owns which queries go stale after a write, and nothing
 * smaller can tell you it got that wrong: every card below it was
 * correct on its own while the page as a whole showed a switch that
 * would not move.
 */
function stubApi() {
  vi.spyOn(api, "getConfig").mockResolvedValue({
    effective: { presence: { enabled: false } },
    overrides: {},
    sections: [],
    persistent: true,
  });
  vi.spyOn(api, "history").mockResolvedValue([]);
  vi.spyOn(api, "devices").mockResolvedValue([]);
  vi.spyOn(api, "secrets").mockResolvedValue({ writable: true, secrets: [] });
  vi.spyOn(api, "setup").mockResolvedValue({ set_up: true, gaps: [] });
  vi.spyOn(api, "timezones").mockResolvedValue(["UTC"]);
  vi.spyOn(api, "integrations").mockResolvedValue([
    {
      id: "tado",
      label: "tado°",
      blurb: "Who is home.",
      needs: "tado° thermostats.",
      kind: "service",
      group: { id: "presence", label: "Presence & climate", blurb: "", requirement: null },
      importance: "optional",
      system: false,
      default_for: [],
      icon: "thermometer",
      secret_keys: [],
      base_url: null,
      serves: [],
      added: true,
      secret_key: null,
    },
  ]);
  vi.spyOn(api, "tadoStatus").mockResolvedValue({
    connectable: true,
    authorised: true,
    presence_enabled: false,
  });
  return vi.spyOn(api, "patchConfig").mockResolvedValue({
    revision: 1,
    noop: false,
    changes: [],
    summary: "presence.enabled → true",
    needs_restart: [],
  });
}

function renderPanel() {
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <ConfigPanel />
    </QueryClientProvider>,
  );
}

describe("ConfigPanel", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    // The open section lives in the address, which tests share.
    window.location.hash = "";
  });

  it("re-asks what the server works out from the config, not only the config", async () => {
    // The bug: a write invalidated `config` and `history` and nothing
    // else, so the tado card — which reads /presence/tado — kept showing
    // the old answer while presence really did turn on behind it.
    const patch = stubApi();
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: /^Add-ons/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Configure/ }));
    const before = vi.mocked(api.tadoStatus).mock.calls.length;

    fireEvent.click(await screen.findByRole("switch"));

    await waitFor(() => expect(patch).toHaveBeenCalled());
    await waitFor(() =>
      expect(vi.mocked(api.tadoStatus).mock.calls.length).toBeGreaterThan(
        before,
      ),
    );
  });

  it("sets the sign-in keys on People, and leaves the API key to its own page", async () => {
    stubHouse();
    const secret = (key: string, label: string) => ({
      key,
      label,
      source: "unset" as const,
    });
    vi.mocked(api.secrets).mockResolvedValue({
      writable: true,
      secrets: [
        secret("mqtt.username", "Zigbee2MQTT broker username"),
        secret("mqtt.password", "Zigbee2MQTT broker password"),
        secret("auth.github_client_id", "GitHub OAuth client ID"),
        secret("auth.github_client_secret", "GitHub OAuth client secret"),
        secret("auth.api_token", "API token"),
        secret("auth.session_secret", "Session signing secret"),
      ],
    });
    renderPanel();

    await open(/People/);
    expect(await screen.findByLabelText("GitHub OAuth client secret")).toBeInTheDocument();
    // Under Advanced → API access now, where it can be shown and replaced.
    expect(screen.queryByLabelText("API token")).toBeNull();
    expect(screen.queryByText(/openssl/)).toBeNull();
    // Made by Niles itself; nobody needs to see it.
    expect(screen.queryByLabelText("Session signing secret")).toBeNull();
  });

  it("re-asks the credentials too, since a key's source depends on the config", async () => {
    stubApi();
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: /^Add-ons/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Configure/ }));
    const before = vi.mocked(api.secrets).mock.calls.length;

    fireEvent.click(await screen.findByRole("switch"));

    await waitFor(() =>
      expect(vi.mocked(api.secrets).mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("does not call recognition off because the file never says it is on", async () => {
    // On by default when the image carries the model, so the file is
    // silent on exactly the installs where it runs. Reading the file
    // told every one of them it was not set up.
    stubHouse();
    vi.spyOn(api, "voices").mockResolvedValue([]);
    renderPanel();

    await open(/People/);
    expect(await screen.findByText(/Nobody yet/)).toBeInTheDocument();
    expect(screen.queryByText(/not set up to recognise voices/i)).toBeNull();
  });

  it("calls recognition off when the server says so", async () => {
    stubHouse();
    vi.spyOn(api, "voices").mockRejectedValue(
      new ApiError("speaker recognition is not configured", 501),
    );
    renderPanel();

    await open(/People/);
    expect(
      await screen.findByText(/not set up to recognise voices/i),
    ).toBeInTheDocument();
  });
});

/**
 * The settings that used to live only in the ConfigMap. Each is written
 * through the same PATCH as everything else, so what matters is the
 * path and the value it is sent as.
 */
function stubHouse() {
  const patch = stubApi();
  vi.mocked(api.getConfig).mockResolvedValue({
    effective: {
      stt: { language: "en" },
      lighting: {
        enabled: true,
        morning_start: "05:45",
        morning_end: "06:30",
        sunset_start: "21:30",
        sunset_end: "23:00",
        night_floor_brightness: 15,
        daytime_brightness: 100,
        color_temp_anchors: [
          { time: "00:00", kelvin: 2000 },
          { time: "12:00", kelvin: 4500 },
        ],
      },
    },
    overrides: {
      stt: { language: "en" },
      lighting: {
        color_temp_anchors: [
          { time: "00:00", kelvin: 2000 },
          { time: "12:00", kelvin: 4500 },
        ],
      },
    },
    defaults: {
      mqtt: { port: 1883, client_id: "niles", z2m_prefix: "zigbee2mqtt" },
      tts: {
        base_url: "http://piper.home-automation.svc.cluster.local:5000",
        default_voice: "en_GB-alan-medium",
      },
      lighting: {
        enabled: false,
        color_temp_anchors: [
          { time: "00:00", kelvin: 2000 },
          { time: "12:00", kelvin: 4500 },
          { time: "23:59", kelvin: 2000 },
        ],
      },
    },
    sections: [
      { name: "mqtt", reload: "boot", overridden: false },
      { name: "tts", reload: "boot", overridden: false },
      { name: "stt", reload: "hot", overridden: true },
      { name: "lighting", reload: "hot", overridden: true },
    ],
    persistent: true,
  });
  const reset = vi.spyOn(api, "resetPath").mockResolvedValue({
    revision: 2,
    noop: false,
    changes: [],
    summary: "reset",
    needs_restart: [],
  });
  return { patch, reset };
}

async function open(section: RegExp) {
  fireEvent.click(await screen.findByRole("button", { name: section }));
}

/** The row a setting sits in, found by its label. */
function rowOf(label: string): HTMLElement {
  return screen.getByText(label, { selector: "label, span" }).closest("div.grid") as HTMLElement;
}

async function setAndSave(label: string, value: string) {
  fireEvent.change(await screen.findByLabelText(label), { target: { value } });
  const enabled = screen
    .getAllByRole("button", { name: "Save" })
    .filter((b) => !(b as HTMLButtonElement).disabled);
  expect(enabled).toHaveLength(1);
  fireEvent.click(enabled[0]);
}

describe("ConfigPanel, the settings that left the ConfigMap", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    window.location.hash = "";
  });

  it("sets the broker up under System, on Devices", async () => {
    const { patch } = stubHouse();
    renderPanel();
    await open(/Devices/);
    const host = await screen.findByLabelText("Broker address");
    fireEvent.change(host, { target: { value: "mosquitto.local" } });
    fireEvent.blur(host);
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect(patch.mock.calls[0][0]).toEqual({ mqtt: { host: "mosquitto.local" } });
  });

  it.each([
    ["Piper address", "http://piper:5000", { tts: { base_url: "http://piper:5000" } }],
    ["Voice voice model", "en_US-amy-low", { tts: { default_voice: "en_US-amy-low" } }],
    ["Language ISO 639-1, like en", "da", { stt: { language: "da" } }],
  ])("writes %s from Voice & AI", async (label, value, body) => {
    const { patch } = stubHouse();
    renderPanel();
    await open(/Voice & AI/);
    await setAndSave(label, value);
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect(patch.mock.calls[0][0]).toEqual(body);
  });

  it("says Piper only changes after a restart, and the language does not", async () => {
    stubHouse();
    renderPanel();
    await open(/Voice & AI/);
    await screen.findByLabelText("Piper address");
    expect(within(rowOf("Piper")).getByText("restart required")).toBeInTheDocument();
    expect(within(rowOf("Language")).queryByText("restart required")).toBeNull();
  });

  it("clears the language back to auto-detect by dropping it, not by writing null", async () => {
    // The PATCH body becomes TOML, which has no null: the server
    // answers 400 to `{"stt":{"language":null}}`. Dropping the value is
    // the one request that means "unset".
    const { patch, reset } = stubHouse();
    renderPanel();
    await open(/Voice & AI/);
    await setAndSave("Language ISO 639-1, like en", "");
    await waitFor(() => expect(reset).toHaveBeenCalledWith("stt.language"));
    expect(patch).not.toHaveBeenCalled();
  });

  it("saves the colour points as one sorted list", async () => {
    const { patch } = stubHouse();
    renderPanel();
    await open(/^Lighting/);
    fireEvent.click(await screen.findByRole("button", { name: /Add a point/ }));
    fireEvent.change(screen.getByLabelText("Point 3 time"), { target: { value: "06:30" } });
    fireEvent.change(screen.getByLabelText("Point 3 kelvin"), { target: { value: "2700" } });
    const enabled = screen
      .getAllByRole("button", { name: "Save" })
      .filter((b) => !(b as HTMLButtonElement).disabled);
    fireEvent.click(enabled[0]);
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect(patch.mock.calls[0][0]).toEqual({
      lighting: {
        color_temp_anchors: [
          { time: "00:00", kelvin: 2000 },
          { time: "06:30", kelvin: 2700 },
          { time: "12:00", kelvin: 4500 },
        ],
      },
    });
  });

  it("resets the colour points to Niles's defaults by dropping the list", async () => {
    const { reset } = stubHouse();
    renderPanel();
    await open(/^Lighting/);
    fireEvent.click(
      await screen.findByRole("button", { name: /Reset to Niles's defaults/ }),
    );
    await waitFor(() =>
      expect(reset).toHaveBeenCalledWith("lighting.color_temp_anchors"),
    );
  });

  it("hides a room from the dashboard through the config save", async () => {
    const { patch } = stubHouse();
    vi.mocked(api.devices).mockResolvedValue([
      {
        id: "z2m:kitchen/bulb",
        source: "z2m",
        room: "kitchen",
        name: "bulb",
        class: "light",
        supports_rgb: false,
        supports_color_temp: false,
        state: {
          on: false,
          brightness: null,
          color_temp_kelvin: null,
          rgb: null,
          temperature_celsius: null,
          humidity_percent: null,
          battery_percent: null,
          open: null,
          motion: null,
          illuminance_lux: null,
          position: null,
        },
      },
    ]);
    renderPanel();
    await open(/^Rooms/);
    fireEvent.click(
      await screen.findByRole("switch", { name: "Show Kitchen on the dashboard" }),
    );
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect(patch.mock.calls[0][0]).toEqual({ rooms: { hidden: ["kitchen"] } });
  });

  it("draws an edited colour point on the chart before it is saved", async () => {
    stubHouse();
    renderPanel();
    await open(/^Lighting/);
    await screen.findByLabelText("Point 2 kelvin");
    const stops = () =>
      Array.from(document.querySelectorAll("#curve-kelvin stop")).map((s) =>
        s.getAttribute("offset"),
      );
    expect(stops()).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: /Add a point/ }));
    fireEvent.change(screen.getByLabelText("Point 3 time"), { target: { value: "18:00" } });
    fireEvent.change(screen.getByLabelText("Point 3 kelvin"), { target: { value: "3000" } });
    await waitFor(() => expect(stops()).toHaveLength(3));
  });

});

const SYSTEM = { id: "system", label: "System", blurb: "", requirement: null };

function stubVoice() {
  const { patch } = stubHouse();
  vi.mocked(api.integrations).mockResolvedValue([
    {
      id: "elevenlabs",
      label: "ElevenLabs",
      blurb: "Hears what you say.",
      needs: "An ElevenLabs account.",
      kind: "provider",
      group: SYSTEM,
      importance: "required",
      system: true,
      default_for: ["stt"],
      icon: "audio-lines",
      secret_keys: ["provider.elevenlabs.api_key"],
      base_url: "https://api.elevenlabs.io/v1",
      serves: ["stt"],
      models: { stt: ["scribe_v2"] },
      added: false,
      secret_key: "provider.elevenlabs.api_key",
    },
  ]);
  vi.mocked(api.secrets).mockResolvedValue({
    writable: true,
    secrets: [{ key: "provider.elevenlabs.api_key", label: "ElevenLabs API key", source: "unset" }],
  });
  vi.mocked(api.setup).mockResolvedValue({
    set_up: false,
    gaps: [
      {
        path: "stt.api_key",
        severity: "degraded",
        consequence:
          "Niles can't hear yet: choose and set up a hearing provider. ElevenLabs needs an API key.",
      },
    ],
  });
  const setSecret = vi.spyOn(api, "setSecret").mockResolvedValue(undefined);
  return { patch, setSecret };
}

describe("ConfigPanel, System", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    window.location.hash = "";
  });

  it("says on Voice & AI what a new install still needs, and counts it in the list", async () => {
    stubVoice();
    renderPanel();
    expect(
      await screen.findByRole("button", { name: /Voice & AI.*1 to set up/ }),
    ).toBeInTheDocument();
    await open(/Voice & AI/);
    expect(await screen.findByText(/Niles can't hear yet/)).toBeInTheDocument();
  });

  it("declares the provider picked, points the role at it, then keeps its key", async () => {
    const order: string[] = [];
    const { patch, setSecret } = stubVoice();
    patch.mockImplementation(async (body) => {
      order.push("patch");
      return { revision: 1, noop: false, changes: [], summary: JSON.stringify(body), needs_restart: [] };
    });
    setSecret.mockImplementation(async () => {
      order.push("key");
    });
    renderPanel();
    await open(/Voice & AI/);
    fireEvent.change(await screen.findByLabelText("ElevenLabs API key"), {
      target: { value: "el-key" },
    });
    fireEvent.click(
      screen
        .getAllByRole("button", { name: "Save" })
        .find((b) => !(b as HTMLButtonElement).disabled)!,
    );
    await waitFor(() => expect(setSecret).toHaveBeenCalled());
    expect(patch.mock.calls[0][0]).toEqual({
      providers: [
        { name: "elevenlabs", base_url: "https://api.elevenlabs.io/v1", serves: ["stt"] },
      ],
      stt: { provider: "elevenlabs", model: "scribe_v2" },
    });
    expect(setSecret).toHaveBeenCalledWith("provider.elevenlabs.api_key", "el-key");
    // The provider is in the config before its key is saved.
    expect(order).toEqual(["patch", "key"]);
  });

  it("opens Add-ons from an old Integrations address", async () => {
    stubHouse();
    window.location.hash = "#/me/settings/integrations";
    renderPanel();
    expect(await screen.findByRole("searchbox", { name: "Search add-ons" })).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe("#/me/settings/addons"));
  });

  it("opens Voice & AI from an old Speech & language address", async () => {
    stubHouse();
    window.location.hash = "#/me/settings/language";
    renderPanel();
    expect(await screen.findByLabelText("Piper address")).toBeInTheDocument();
  });
});

describe("satellitesAt", () => {
  it("reads the volume back", () => {
    // The bug this pins: the slider wrote `satellites.<name>.volume`
    // and this never read it, so every reload showed the shipped 100
    // and the setting looked as though it had not saved.
    const got = satellitesAt({
      satellites: {
        kitchen: { ip: "192.168.42.30", room: "kitchen", volume: 40 },
      },
    });
    expect(got).toEqual([
      { name: "kitchen", ip: "192.168.42.30", room: "kitchen", volume: 40 },
    ]);
  });

  it("leaves volume unset for an entry that has none", () => {
    // Undefined rather than 100: the card decides what an unset volume
    // looks like, and deciding it twice is how the two come to
    // disagree.
    const got = satellitesAt({
      satellites: { kitchen: { ip: "192.168.42.30", room: "kitchen" } },
    });
    expect(got[0].volume).toBeUndefined();
  });

  it("survives a config with no satellites at all", () => {
    expect(satellitesAt({})).toEqual([]);
    expect(satellitesAt(null)).toEqual([]);
  });
});

describe("sonosAt", () => {
  it("reads where each Sonos is placed", () => {
    const effective = {
      speakers: {
        host: "10.0.0.2",
        sonos: {
          RINCON_BAR: { room: "living_room", name: "Living Room" },
          RINCON_MOVE: { room: "kitchen" },
        },
      },
    };
    expect(sonosAt(effective)).toEqual({
      RINCON_BAR: { room: "living_room", name: "Living Room" },
      // Placed by hand without a name: the id is better than nothing.
      RINCON_MOVE: { room: "kitchen", name: "RINCON_MOVE" },
    });
  });

  it("reads nothing placed as nothing", () => {
    expect(sonosAt({})).toEqual({});
  });
});

describe("musicByRoom", () => {
  it("gathers a room's Sonos, so a satellite can say what plays there", () => {
    const effective = {
      speakers: {
        sonos: {
          RINCON_BACK: { room: "living_room", name: "Living Room Back" },
          RINCON_BAR: { room: "living_room", name: "Living Room" },
          RINCON_MOVE: { room: "kitchen", name: "Sonos Move" },
        },
      },
    };
    expect(musicByRoom(effective)).toEqual({
      living_room: ["Living Room", "Living Room Back"],
      kitchen: ["Sonos Move"],
    });
  });
});
