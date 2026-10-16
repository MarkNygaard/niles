/**
 * Typed client for Niles's /config routes.
 *
 * Mirrors crates/niles-api/src/config.rs. The shapes are small enough
 * that hand-written types beat a generator here, but they do have to be
 * kept in step with that file.
 */

/** Whether a section is picked up by the running process. */
export type Reload = "hot" | "boot";

export interface SectionView {
  name: string;
  reload: Reload;
  overridden: boolean;
}

export interface ConfigView {
  /** Base with overrides applied — what Niles is actually running. */
  effective: Record<string, unknown>;
  /** Only the values set from the app, away from Niles's defaults. */
  overrides: Record<string, unknown>;
  sections: SectionView[];
  /** False when there is no writable volume: changes are lost on restart. */
  persistent: boolean;
  /**
   * What Niles runs where the config says nothing.
   *
   * `effective` is the merged *file*, so a value nobody wrote is absent
   * from it — which made a working setting look like an empty box.
   */
  defaults?: Record<string, unknown>;
}

/**
 * One WLED strip, as `[[wled.devices]]` has it.
 *
 * `rgb` and `white_balance` are optional here because the config file
 * may simply not mention them: unwritten means the defaults, which are
 * colour and no white balance.
 */
export interface WledStrip {
  name: string;
  topic: string;
  rgb?: boolean;
  white_balance?: boolean;
}

/**
 * Something Niles knows how to connect to.
 *
 * The list is fixed and comes from the server, because whether Niles
 * can talk to a service is a question about the code rather than about
 * the config — and because the endpoint of a known provider is
 * something Niles already knows and nobody should have to type.
 */
export interface IntegrationGroup {
  id: string;
  label: string;
  blurb: string;
  /** What the group as a whole needs, when that is "one of these"
      rather than each of them. */
  requirement: string | null;
}

export type Importance = "required" | "recommended" | "optional";

export interface Integration {
  id: string;
  label: string;
  /** What it does for the house, for a newcomer. */
  blurb: string;
  /** What you need before you start. */
  needs: string;
  kind: "provider" | "service";
  /** The list arrives in page order, so a group goes where it first
      appears. */
  group: IntegrationGroup;
  importance: Importance;
  /** Part of the base system — the broker, the providers that hear and
      think — set up under Settings → System rather than offered as an
      add-on. */
  system: boolean;
  /** The roles that use it when they name no provider. */
  default_for: ("stt" | "llm")[];
  /** A Lucide icon name. */
  icon: string;
  /** Every key it keeps; removing it deletes the stored ones. */
  secret_keys: string[];
  base_url: string | null;
  serves: ("stt" | "llm")[];
  /**
   * What it can be asked for, per role. The first for a role is what
   * Niles uses when nothing is written down.
   */
  models?: Partial<Record<"stt" | "llm", string[]>>;
  added: boolean;
  secret_key: string | null;
}

/** One tool a remote tool server offers. Mirrors `RemoteToolView`. */
export interface RemoteTool {
  /** The server's name for it, which its setting is keyed by. */
  name: string;
  title: string | null;
  description: string;
  read_only: boolean;
  destructive: boolean;
}

/**
 * A remote MCP server and what it offers.
 *
 * Mirrors `ToolServerView` in crates/niles-api/src/tool_servers.rs.
 */
export interface ToolServer {
  id: string;
  status: "connecting" | "working" | "needs_key" | "failed";
  message: string | null;
  tools: RemoteTool[];
}

/**
 * A third-party add-on installed from GitHub.
 *
 * Mirrors `PluginSummary` in crates/niles-api/src/plugins.rs.
 */
export interface PluginSummary {
  id: string;
  name: string;
  version: string;
  description: string;
  authors: string[];
  homepage: string | null;
  /** A catalogue group, or `custom`. */
  group: { id: string; label: string };
  status: "starting" | "running" | "not_set_up" | "failing" | "stopped" | "disabled";
  /** "Working", "Not working: …", "Off". */
  status_line: string;
  /** The last thing that went wrong, or that an update was rolled back. */
  error: string | null;
  dropped_events: number;
  enabled: boolean;
  /** `https://github.com/<owner>/<repo>`. */
  repo: string | null;
  release_tag: string | null;
  release_url: string | null;
  installed_at: string | null;
  updated_at: string | null;
  /** Where Niles serves its logo, when it has one. */
  logo: string | null;
  /** What a rollback goes back to, when there is something. */
  previous_version: string | null;
  update: PluginUpdate | null;
  /** What its setup panel asks for, and what each holds now. */
  settings: PluginSetting[];
  /** Its pages, which join the menu as `<id>.<page>`. */
  menu: PluginPage[];
  /** Its cards for the front page and the rooms. */
  widgets: PluginWidget[];
  /** The `niles:integration` version it was built for. */
  interface: string;
  /** Built for an older interface than the newest this Niles runs. */
  older_interface: boolean;
}

export type SettingType = "string" | "number" | "boolean" | "secret" | "choice" | "room";

/** One of a plugin's settings, as its manifest declares it. */
export interface PluginSetting {
  key: string;
  label: string;
  type: SettingType;
  required: boolean;
  help: string | null;
  default: string | number | boolean | null;
  options: { value: string; label: string }[];
  min: number | null;
  max: number | null;
  /** What the household entered; never a secret's. */
  value: string | number | boolean | null;
  /** Entered, or for a secret, stored. A default does not count. */
  set: boolean;
}

export interface PluginPage {
  id: string;
  label: string;
  /** A Lucide icon name, `zap`. */
  icon: string;
  refresh_every: number | null;
}

export interface PluginWidget {
  id: string;
  title: string;
  /** The room it starts on; none for the front page. */
  room: string | null;
  refresh_every: number | null;
}

export interface PluginLogLine {
  at: string;
  level: "debug" | "info" | "warn" | "error";
  message: string;
  /** Said by Niles about the plugin, not by the plugin. */
  niles: boolean;
}

/**
 * A plugin's page or widget: a flat list of nodes, the root first,
 * each naming its children by index. Mirrors `niles_plugins::ui::Surface`.
 */
export interface Surface {
  nodes: UiNode[];
}

export type Tone = "neutral" | "good" | "warning" | "bad";

export type UiNode = { children: number[] } & (
  | { type: "stack" }
  | { type: "row" }
  | { type: "card"; title: string | null }
  | { type: "heading"; text: string }
  | { type: "text"; value: string; muted: boolean }
  | { type: "reading"; label: string; value: string; unit: string | null }
  | { type: "badge"; label: string; tone: Tone }
  | {
      type: "items";
      items: { title: string; detail: string | null; trailing: string | null }[];
    }
  | {
      type: "button";
      id: string;
      label: string;
      style: "primary" | "secondary" | "destructive";
    }
  | { type: "switch"; id: string; label: string; value: boolean }
  | {
      type: "slider";
      id: string;
      label: string;
      min: number;
      max: number;
      step: number;
      value: number;
      unit: string | null;
    }
  | {
      type: "choice";
      id: string;
      label: string;
      options: { value: string; label: string }[];
      value: string | null;
    }
  | {
      type: "text_field";
      id: string;
      label: string;
      placeholder: string | null;
      value: string | null;
    }
  | {
      type: "chart";
      kind: "line" | "bar";
      labels: string[];
      series: { name: string; values: number[] }[];
      unit: string | null;
    }
  | { type: "image"; src: string; alt: string }
);

/** What somebody did to a control, as the plugin's `on_action` gets it. */
export type ActionValue =
  | { type: "pressed" }
  | { type: "toggled"; value: boolean }
  | { type: "number"; value: number }
  | { type: "chosen"; value: string }
  | { type: "text"; value: string };

export interface ActionOutcome {
  toast: string | null;
  rerender: boolean;
}

export interface PluginUpdate {
  version: string;
  tag: string;
  release_url: string;
  notes: string;
  /** Permissions it adds, in plain words. */
  added: string[];
  removed: string[];
}

/** What the approval screen shows before an install or an update. */
export interface PluginPreview {
  /** Given back to install exactly this. */
  token: string;
  expires_in: number;
  id: string;
  name: string;
  description: string;
  version: string;
  authors: string[];
  homepage: string | null;
  repo: string;
  tag: string;
  release_url: string;
  notes: string;
  group: { id: string; label: string };
  /** The sanitised SVG itself. */
  logo: string | null;
  /** What it may do, one line each. */
  permissions: string[];
  /** What it may not, always the same sentence. */
  cannot: string;
  tools: { name: string; description: string }[];
  settings: {
    key: string;
    label: string;
    type: "string" | "number" | "boolean" | "secret" | "choice" | "room";
    required: boolean;
    help: string | null;
  }[];
  warnings: string[];
  /** The component's sha256, given back with the install. */
  sha256: string;
  manifest_sha256: string;
  /** For an update: the version installed now. */
  current_version: string | null;
  added: string[];
  removed: string[];
}

/** One Sonos room: a speaker, a stereo pair, or a soundbar with the
    speakers around it — always played as one. */
export interface SonosSpeaker {
  /** Sonos's own id (`RINCON_…`), which stays when the address moves. */
  id: string;
  /** What the Sonos app calls it. */
  name: string;
  ip: string | null;
  /** A soundbar with speakers around it: the one a TV plays through. */
  home_theater: boolean;
  /** The Niles room it is placed in, if any. */
  room: string | null;
  /** False for one that is placed but did not answer just now. */
  answering: boolean;
}

/** A room's Sonos, as `GET /music` reports it. */
export interface RoomMusic {
  room: string;
  playing: boolean;
  /** "Chariot by Gavin DeGraw", "DR P3". */
  what: string | null;
  kind: "music" | "radio" | "tv" | null;
  /** Percent, from the room's first speaker. */
  volume: number | null;
}

export type MusicControl =
  | { action: "pause" }
  | { action: "play" }
  | { action: "volume"; percent: number };

/** One Sonos on the Media page: a speaker, a pair, a soundbar with
    its rear speakers. */
export interface MediaSpeaker {
  /** `RINCON_…` */
  id: string;
  name: string;
  room: string | null;
  volume: number | null;
  /** The one the TV plays through. */
  soundbar: boolean;
}

/** Speakers playing one thing together, by what they have loaded —
    playing or paused alike. */
export interface MediaGroup {
  leader: string;
  kind: "tv" | "radio" | "spotify" | "music";
  what: string | null;
  playing: boolean;
  speakers: MediaSpeaker[];
}

/** `GET /media`: the groups, and the speakers with nothing loaded. */
export interface MediaView {
  groups: MediaGroup[];
  idle: MediaSpeaker[];
  /** Spotify's add-on is set up: its Play button can work. */
  spotify?: boolean;
}

/** Something the Media page can start playing. */
export interface MediaChoice {
  kind: "favorite" | "station" | "remembered" | "spotify" | "queue";
  id: string;
  label: string;
}

export type SpeakerControl =
  | { action: "join"; leader: string }
  | { action: "leave" }
  | { action: "volume"; percent: number };

export type CalendarAccess = "read" | "write";

export type CalendarProvider = "microsoft" | "google" | "icloud";

/** One account a person connected, serving Calendar, Mail or both. */
export interface CalendarConnection {
  id: string;
  provider: CalendarProvider;
  /** What they called it: "Private", "Work". */
  name: string;
  /** The account's address. */
  account: string;
  /** Whether it serves Calendar, with `access`. */
  calendar: boolean;
  access: CalendarAccess;
  /** What it may do with mail, when it serves Mail. */
  mail: "read" | "send" | null;
  /** False for iCloud, where read only is Niles's rule, not Apple's. */
  enforced_by_provider: boolean;
}

/** `GET /calendars`: the signed-in person's own, and what can be
    connected. */
export interface CalendarsView {
  providers: { id: CalendarProvider; available: boolean; reason: string | null }[];
  connections: CalendarConnection[];
}

export interface IcloudConnect {
  name: string;
  calendar: "none" | "read" | "write";
  mail: "none" | "read" | "send";
  apple_id: string;
  password: string;
}

/** The LG TV, as `GET /tv` reports it. */
export interface TvInfo {
  configured: boolean;
  paired: boolean;
  /** The MAC it is woken by first. */
  mac: string | null;
  /** "detected" at pairing, or "hand": typed, and kept through a pairing again. */
  mac_source?: "detected" | "hand" | null;
  /** The MAC pairing chose, when the one set by hand is another. */
  detected_mac?: string | null;
  room: string | null;
  /** Absent before pairing, or when the TV could not be asked. */
  status: {
    on: boolean;
    app: string | null;
    /** The TV's own speakers, `0..=100`: only while it is on and says. */
    volume?: number;
    muted?: boolean;
    /** Where its sound goes, on a TV new enough to say: "tv_speaker" is its own. */
    sound_output?: string;
  } | null;
  error: string | null;
}

/** One change to the TV's own speakers. */
export type TvVolumeChange = { volume: number } | { delta: number } | { mute: boolean };

/**
 * What came of the power button. Waking it is `on` only once the TV
 * answered; otherwise `message` says the signal went out and where to
 * look.
 */
export interface TvPowered {
  on: boolean;
  message?: string;
}

/** The TV's own speakers after a change. */
export interface TvVolume {
  volume: number;
  muted: boolean;
  sound_output?: string;
}

/** The Sonos household as a speaker described it just now. */
export interface SpeakersReport {
  /** Whether Niles has an address to ask. */
  configured: boolean;
  error: string | null;
  sonos: SonosSpeaker[];
}

/**
 * One tado heating zone, as `/climate` returns it.
 *
 * `temperature` and `humidity` are absent when the valve is not
 * answering: tado goes on sending the last reading it heard, and a
 * stale number beside live ones is worse than none. `target` survives,
 * because a setpoint is tado's own rather than the valve's.
 */
export interface Zone {
  id: number;
  name: string;
  room: string | null;
  temperature: number | null;
  humidity: number | null;
  target: number | null;
  on: boolean;
  overridden: boolean;
  reachable: boolean;
  /**
   * When the override ends, if it ends by itself. Absent for one that
   * lasts until somebody resumes the schedule — which is the one worth
   * saying out loud, because it is the one that gets forgotten.
   */
  until: string | null;
  /** Whether the room was chosen, guessed from the name, or neither. */
  placed_by: "paired" | "name" | "nowhere";
}

/** What a boost did, so the page can say what it asked for. */
export interface Boosted {
  rooms: number;
  celsius: number;
  minutes: number;
}

/** A place the house might be, as `/places` returns it. */
export interface Place {
  label: string;
  latitude: number;
  longitude: number;
  /** Absent when it could not be looked up; the one set stays. */
  timezone: string | null;
  /** "Vestergade 12, 8000 Aarhus" — only for a street address. */
  address: string | null;
  country_code: string | null;
}

export interface SetupGap {
  path: string;
  severity: "blocking" | "degraded";
  consequence: string;
}

export interface SetupReport {
  set_up: boolean;
  gaps: SetupGap[];
}

export interface Secret {
  key: string;
  label: string;
  /** What it is used against, when Niles knows. */
  hint?: string;
  /**
   * Where Niles reads it from. One field rather than two booleans the
   * page has to combine correctly — which is how the first version got
   * it wrong and showed working credentials as unset.
   */
  source: "environment" | "stored" | "unset";
}

export interface SecretsReport {
  /** False without a database or an encryption key. */
  writable: boolean;
  secrets: Secret[];
}

/**
 * The API key, for a signed-in person to copy into a script.
 *
 * The one secret that can be read back, because copying it is its
 * whole point. `token` comes only with a key Niles keeps; one set by
 * the environment is never shown, and `env_var` says where it is.
 */
export interface ApiToken {
  source: "stored" | "environment" | "none";
  token?: string;
  env_var?: string;
  /** False without a database and its secrets key: nowhere to keep one. */
  writable: boolean;
}

/** Whether ChatGPT is signed in, and as whom. Never the tokens. */
export interface ChatGptStatus {
  signed_in: boolean;
  email: string | null;
  /** When the access token runs out; Niles renews it before then. */
  expires_at: string | null;
  /** False without a database and its secrets key: nowhere to keep one. */
  writable: boolean;
}

export interface TadoPending {
  verification_uri: string;
  user_code: string;
  expires_at: string;
}

export interface TadoStatus {
  /** Whether connecting is possible at all — i.e. there is a database. */
  connectable: boolean;
  authorised: boolean;
  /** Whether presence polling is switched on. Separate from connected. */
  presence_enabled: boolean;
  pending?: TadoPending;
}

export interface DeviceState {
  on: boolean | null;
  brightness: number | null;
  color_temp_kelvin: number | null;
  rgb: [number, number, number] | null;
  temperature_celsius: number | null;
  humidity_percent: number | null;
  battery_percent: number | null;
  /** A door or window: `true` is open. `null` on everything else. */
  open: boolean | null;
  /** A motion sensor: `true` while it sees somebody. */
  motion: boolean | null;
  /** What a motion sensor also reads; below about 30 is dim. */
  illuminance_lux: number | null;
  /** A blind: `0` closed, `100` fully open. */
  position: number | null;
  /**
   * Every other number it reports, by name: `power`, `energy`, `co2`,
   * an electricity `price`. Partial on the event stream like the rest,
   * so merged name by name. Absent on an older server.
   */
  readings?: Record<string, Reading>;
}

/** A number a device reports that has no field of its own. */
export interface Reading {
  value: number;
  /** The source's own unit — `W`, `kWh`, `kr/kWh` — or `null`. */
  unit: string | null;
}

/** A device as `GET /devices` reports it. */
export interface Device {
  /** Fully qualified: `z2m:living_room/lamp`, `wled:living_room/tv`. */
  id: string;
  source: string;
  room: string;
  name: string;
  class: string;
  state: DeviceState;
  /** Whether the device can be told a colour / a colour temperature. */
  supports_rgb: boolean;
  supports_color_temp: boolean;
  /**
   * Whether the source can currently reach it. Absent on an older
   * server, which reads the same as reachable.
   */
  available?: boolean;
  /**
   * One of the house's lights: a light, or a plug listed as having a
   * lamp on it. False for a light its plugin keeps out of its room's
   * lights, such as a printer's chamber light. Absent on an older
   * server, where every plug and every light was one.
   */
  lamp?: boolean;
  /**
   * Whether it reports actions: a switch, or anything that has reported
   * one. Absent on an older server, where only switches did.
   */
  actions?: boolean;
}

/** What a light can be told. Every field is optional; at least one is required. */
export interface SetLight {
  on?: boolean;
  /** Percent, `0..=100`. */
  brightness?: number;
  color_temp_kelvin?: number;
  rgb?: [number, number, number];
}

/** What `GET /auth/status` says before anybody has signed in. */
export interface AuthStatus {
  /** False when nobody is on the allowlist: Niles is open. */
  enabled: boolean;
  /** The address of whoever is holding this browser, or null. */
  signed_in_as: string | null;
  /** Their GitHub avatar, when the session knows which account it is. */
  avatar_url: string | null;
}

export interface Change {
  path: string;
  from: unknown | null;
  to: unknown;
}

export interface Applied {
  revision: number;
  changes: Change[];
  summary: string;
  noop: boolean;
  needs_restart: string[];
}

export interface Revision {
  id: number;
  at: string;
  source: "voice" | "api";
  summary: string;
}

/**
 * The server answers 4xx with `{error}` for things the user can fix —
 * a value out of range, a misspelled key. Surfacing that text verbatim
 * is the whole point: it names the field and says why.
 */
export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    headers: { "content-type": "application/json" },
    ...init,
  });
  const text = await response.text();
  if (!response.ok) {
    throw new ApiError(
      errorMessage(text) ?? `request failed (${response.status})`,
      response.status,
    );
  }
  return (text ? JSON.parse(text) : null) as T;
}

/** The reason in a refusal: `{error}`, or the body itself when it is
    plain text, which is how the automations endpoints say why. */
function errorMessage(text: string): string | undefined {
  try {
    const body = JSON.parse(text);
    return body && typeof body.error === "string" ? body.error : undefined;
  } catch {
    const plain = text.trim();
    // A proxy's error page is not a reason anybody can act on.
    return plain && !plain.startsWith("<") ? plain : undefined;
  }
}

/** How long, in parts: `{ minutes: 10 }`. At most 24 hours in all. */
export interface Duration {
  hours?: number;
  minutes?: number;
  seconds?: number;
}

/** A number every rule knows how to read, whatever the device. */
export type FixedMeasure = "temperature" | "humidity" | "battery" | "brightness" | "illuminance";

/**
 * What a numeric trigger or condition watches: a fixed measure, or the
 * name of a reading the device reports (`price`, `power`, `co2`). The
 * `string & {}` keeps the fixed names offered by the editor's types
 * without closing the list.
 */
export type Measure = FixedMeasure | (string & {});

export type Day = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";

/**
 * What starts an automation. Devices are named `room/name`, without
 * the source prefix: the server fills that in.
 *
 * `id` is how a `triggered_by` condition names the trigger. The server
 * gives every saved trigger one; the app gives a new one its own
 * (`t1`, `t2`…) so a condition can name it before it is saved, and
 * never shows it.
 */
export type Trigger = { id?: string } & (
  /** A door or window: `open` true opens, false closes, absent either.
      With `for`, once it has stayed that way that long. */
  | { type: "contact"; device: string; open?: boolean; for?: Duration }
  /** `detected` true sees somebody, false sees nobody any more, absent
      either. With `for`, "no motion for 2 minutes". */
  | { type: "motion"; device: string; detected?: boolean; for?: Duration }
  | { type: "device_state"; device?: string; room?: string; on?: boolean; for?: Duration }
  | {
      type: "numeric";
      device: string;
      measure: Measure;
      above?: number;
      below?: number;
      for?: Duration;
    }
  /** A button or dimmer press, e.g. `on_press`; absent is any press. */
  | { type: "device_action"; device: string; action?: string }
  | { type: "presence"; state?: "home" | "away"; for?: Duration }
  /** `HH:MM`; no days is every day. */
  | { type: "time"; at: string; days?: Day[] }
  /** Minutes after the event; negative is before it. */
  | { type: "sun"; event: "sunrise" | "sunset"; offset_minutes?: number }
  | { type: "timer_fired"; name?: string }
);

/** What has to hold when a trigger fires for the actions to run. */
export type Condition =
  /** `HH:MM`; the window may wrap past midnight. */
  | { kind: "time_of_day"; after?: string; before?: string; days?: Day[] }
  | {
      kind: "sun";
      after?: "sunrise" | "sunset";
      before?: "sunrise" | "sunset";
      after_offset_minutes?: number;
      before_offset_minutes?: number;
    }
  | { kind: "device_is"; device: string; on: boolean }
  | { kind: "contact_is"; device: string; open: boolean }
  | { kind: "motion_is"; device: string; detected: boolean }
  | { kind: "numeric"; device: string; measure: Measure; above?: number; below?: number }
  | { kind: "presence_is"; state: "home" | "away" }
  /** Which of the rule's triggers started this run, by their ids. */
  | { kind: "triggered_by"; triggers: string[] };

export type Action =
  | {
      do: "set_device";
      device: string;
      on?: boolean;
      /** Percent, `0..=100`. */
      brightness?: number;
      /** `2000..=6500`. */
      kelvin?: number;
    }
  /** Every light in the room. */
  | { do: "set_room"; room: string; on?: boolean; brightness?: number; kelvin?: number }
  /** Every blind in a room, or one blind: exactly one of the two.
      `position` is 0 closed to 100 open. */
  | { do: "set_blinds"; room?: string; device?: string; position: number }
  | {
      do: "notify";
      body: string;
      room?: string;
      priority?: "routine" | "important" | "urgent";
    }
  /** Wait, then go on with the next action. */
  | { do: "delay"; hours?: number; minutes?: number; seconds?: number }
  /** The same conditions a rule takes; `then` and `else` may hold an
      `if` in turn, three deep at most. */
  | { do: "if"; conditions: Condition[]; then: Action[]; else?: Action[] };

export interface Rule {
  id: string;
  description: string;
  enabled: boolean;
  triggers: Trigger[];
  conditions: Condition[];
  actions: Action[];
}

/** A rule as `GET /automations` lists it. */
export interface AutomationView {
  rule: Rule;
  /** `config` rules come from the TOML file, and are read-only here. */
  source: "stored" | "config";
  /** The rule as a sentence, written by the server. */
  sentence: string;
  /** Why the rule is not running, when it is invalid. */
  error: string | null;
}

/** One time a rule started, as `GET /automations/{id}/runs` lists it.
    A trigger whose conditions did not hold is a run too, `stopped`. */
export interface Run {
  /** ISO 8601. */
  started_at: string;
  /** The id of the trigger that started it. */
  trigger: string | null;
  outcome: "running" | "done" | "stopped" | "replaced" | "failed";
  /** Why it stopped ("it was before 07:00"), or the error it failed on. */
  reason: string | null;
  /** What it did, as short sentences. */
  steps: string[];
}

/** Where a skill is in its life: the curator marks one nobody used in
    30 days stale, and one unused in 90 archived. */
export type SkillStatus = "active" | "stale" | "archived";

/** A procedure Niles wrote for itself, as `GET /skills` lists it. */
export interface Skill {
  /** Lowercase and hyphenated: `evening-wind-down`. */
  name: string;
  description: string;
  version: string;
  status: SkillStatus;
  /** `agent-created` it wrote after a conversation; `user-created`
      somebody asked it to. */
  provenance: "agent-created" | "user-created";
  /** Kept from the curator and from deletion. */
  pinned: boolean;
  /** ISO 8601, like the other times. */
  created_at: string;
  last_used_at: string | null;
  last_viewed_at: string | null;
  last_patched_at: string | null;
  usage_count: number;
  view_count: number;
  patch_count: number;
  /** The skill this one was folded into, when Niles retired it. */
  absorbed_into?: string;
}

/** One change to a skill, as it can be undone. */
export interface SkillChangeRecord {
  at: string;
  /** "made", "edited", "rewritten", "folded into evening", "deleted". */
  change: string;
}

/** One skill with its procedure, as `GET /skills/{name}` has it. */
export interface SkillDetail extends Skill {
  /** Markdown. */
  body: string;
  /** What can be undone, newest first. */
  changes: SkillChangeRecord[];
}

/** When Niles last tidied its skills, and what it did. */
export interface SkillTidy {
  at: string | null;
  summary: string | null;
}

/** What `PATCH /skills/{name}` changes. */
export interface SkillChange {
  pinned?: boolean;
  status?: "active" | "archived";
}

/** Which memory document: facts about the household, or what Niles
    noted for itself. */
export type MemoryTarget = "user" | "agent";

/** What Niles remembers, entry by entry, as `GET /memory` has it. */
export interface Memory {
  user: string[];
  agent: string[];
  /** Taken out lately, newest first, by somebody or by Niles tidying. */
  forgotten: { target: MemoryTarget; text: string; at: string }[];
}

/** One conversation in the list: newest first, the signed-in person's
    own. A new one starts after five minutes of quiet. */
export interface ConversationSummary {
  id: number;
  /** The first thing said, shortened by the server. */
  title: string;
  started_at: string;
  last_at: string;
  turns: number;
  /** Whether any of it was said to a satellite. */
  spoken: boolean;
}

/** Niles reading a rule back and asking whether to save it. */
export interface Proposal {
  sentence: string;
}

/** One thing said to Niles, and the answer. */
export interface Turn {
  at: string;
  said: string;
  reply: string;
  via: "voice" | "app";
  /** The satellite's room, snake_case, on a voice turn. */
  room: string | null;
  /** What wrote the reply, when it was not Niles's own models. */
  by: AssistantId | null;
  /** Set when Niles asked for confirmation in this turn. Only the last
      turn's can still be answered. */
  proposal: Proposal | null;
  /** What this turn left waiting for a yes. */
  approval?: ApprovalCard | null;
  /** The artifacts this turn made or changed, for a card under the
      reply. Absent when there are none. */
  artifacts?: ArtifactRef[];
}

export type ArtifactStatus = "ready" | "working" | "failed";

/** One line of an artifact's preview, its markup gone. */
export interface PreviewLine {
  text: string;
  style: "heading" | "bullet" | "number" | "check" | "text";
  /** For a checklist item. */
  checked?: boolean;
}

export interface Preview {
  lines: PreviewLine[];
}

/** An artifact as a chat turn carries it. Mirrors `ArtifactRef` in
    crates/niles-artifacts/src/model.rs. */
export interface ArtifactRef {
  id: number;
  kind: string;
  title: string;
  summary: string;
  status: ArtifactStatus;
  preview: Preview;
}

/** Something Niles made for the signed-in person, as Saved lists it. */
export interface Artifact {
  id: number;
  kind: string;
  title: string;
  summary: string;
  pinned: boolean;
  archived: boolean;
  /** Whether they are told when its job ends. */
  notify: boolean;
  status: ArtifactStatus;
  /** What its job is doing, while it does it. */
  status_text: string | null;
  /** Why its job did not finish. */
  error: string | null;
  created_at: string;
  updated_at: string;
  /** The version shown; 0 before there is one. */
  current_revision: number;
  preview: Preview;
}

export type ArtifactAuthor = "person" | "claude" | "chatgpt" | "niles";

export interface ArtifactRevisionMeta {
  n: number;
  reason: string;
  author: ArtifactAuthor;
  at: string;
}

export interface ArtifactRevision extends ArtifactRevisionMeta {
  content: unknown;
}

export interface ArtifactDetail extends Artifact {
  /** The current version's content; null before there is one. */
  content: unknown;
  /** Newest first. */
  revisions: ArtifactRevisionMeta[];
}

export interface ArtifactList {
  artifacts: Artifact[];
  /** Which assistant makes them now; null when neither is on. */
  assistant: AssistantId | null;
}

export interface ArtifactPatch {
  title?: string;
  pinned?: boolean;
  archived?: boolean;
  notify?: boolean;
  /** A new version, by the person. */
  content?: unknown;
  reason?: string;
}

export type ApprovalStatus = "pending" | "allowed" | "denied" | "lapsed" | "failed";

/** Allow once, Allow for this conversation, or Deny. */
export type Decision = "once" | "conversation" | "deny";

/**
 * Something Niles holds for the person's yes. Mirrors `ApprovalCard` in
 * crates/niles-api/src/chat.rs.
 */
export interface ApprovalCard {
  id: string;
  title: string;
  subtitle: string;
  /** A brand mark's id, or "mail". */
  icon: string | null;
  fields: { label: string; value: string }[];
  /** A long text shown in full: a mail's body. */
  body: string | null;
  allow_conversation: boolean;
  status: ApprovalStatus;
  outcome: string | null;
}

export interface Conversation {
  id: number;
  title: string;
  started_at: string;
  last_at: string;
  turns: Turn[];
}

/** The assistants that can answer in Niles's place. */
export type AssistantId = "claude" | "chatgpt";

export interface SentChat {
  reply: string;
  via?: AssistantId;
  /** Why the assistant did not answer, when it was meant to. Only on a
      reply just sent, never read back from a conversation. */
  fallback?: string;
  /** Which assistant that was. */
  fallback_from?: AssistantId;
  /** The conversation the message landed in. */
  conversation_id: number;
}

/** Where a message goes: on with this conversation, or a new one.
    Neither lets the server pick, by the five-minute rule. */
export type ChatTarget = { conversation_id: number } | { new: true } | Record<string, never>;

/** One thing nemlig.com sells. */
export interface NemligProduct {
  id: string;
  name: string;
  /** Size and brand: "1 l / Arla ØKO". */
  description: string;
  /** Kroner. */
  price: number;
  /** "13,95 kr/l". */
  unit_price: string | null;
  image: string | null;
  available: boolean;
  /** The deal when it is on offer: "12,95 kr", "3 for 50 kr". */
  offer?: string | null;
}

/** An order placed at nemlig.com. Times are Danish local time, written
    without a zone: "2026-10-10T07:00:00". */
export interface NemligOrder {
  id: number;
  status: number;
  total: number | null;
  delivery_start: string | null;
  delivery_end: string | null;
}

/** The account's basket at nemlig.com. */
export interface NemligBasket {
  lines: { product_id: string; name: string; quantity: number; total: number }[];
  /** Kroner, everything included. */
  total: number;
  delivery_price: number;
  /** "Torsdag 9. oktober kl. 7-9", once a time is reserved. */
  delivery: string | null;
  slot_id: number | null;
  minimum_total: number | null;
  meets_minimum: boolean;
  /** How long nemlig holds a time just reserved, in minutes. */
  held_minutes?: number;
}

/** What sending the list to nemlig.com did. */
export interface NemligSent {
  basket: NemligBasket;
  sent: number;
  /** Items still to buy that had no product chosen. */
  without: string[];
  /** Items whose product would not go in: sold out since it was chosen. */
  unavailable: string[];
  /** Where to review the basket and pay. */
  checkout: string;
}

export interface DeliverySlot {
  id: number;
  start_hour: number;
  end_hour: number;
  price: number;
  available: boolean;
  selected: boolean;
  deadline: string | null;
}

export interface DeliveryDay {
  /** "2026-10-09". */
  date: string;
  slots: DeliverySlot[];
}

/** One thing on the shopping list. */
export interface GroceryItem {
  id: number;
  /** What goes in the basket: "Letmælk". */
  name: string;
  /** The words it was asked for in — "milk" — when they are not its
      name. Checking it off teaches Niles that they mean this. */
  said?: string;
  quantity?: string;
  added_at: string;
  /** Set once it is in the basket. */
  checked_at?: string;
  /** The nemlig.com product chosen for it, if any. */
  nemlig?: NemligProduct;
}

export interface GroceryList {
  items: GroceryItem[];
  /** Bought before and not on the list now, most bought first. */
  usual: string[];
  /** Whether nemlig.com is switched on and has a login. */
  nemlig: boolean;
}

/** A change to one item. An empty quantity clears it. */
export interface GroceryEdit {
  name?: string;
  quantity?: string;
  checked?: boolean;
}

/** One voice Niles has been taught. */
export interface Voice {
  speaker: string;
  display_name: string;
  /** How to say it, when spelling and saying it differ. */
  spoken_as: string | null;
  /** How Niles addresses them — "Sir" — beside the name, not instead. */
  address_as: string | null;
  /** One is thin — Niles asks for more until it has three. */
  clip_count: number;
  created_at: string;
  /** Null on a voice enrolled and never matched since, which is the
      shape of an enrolment that is not working. */
  last_seen_at: string | null;
}

/** The signed-in person's own page. */
export interface Me {
  email: string;
  /** The voice they are linked to. Null: no profile until one is. */
  speaker: string | null;
  display_name: string | null;
  spoken_as: string | null;
  address_as: string | null;
  /** Their own USER.md. */
  notes: string | null;
  /** "MM-DD". */
  birthday: string | null;
  /** The MAC presence follows them by. */
  phone: string | null;
}

/** The signed-in person's own Strava. Whether a secret is kept, never
    the secret; never a token. */
export interface StravaStatus {
  /** Their Strava API app's Client ID; null until they save one. */
  client_id: string | null;
  secret_set: boolean;
  connected: boolean;
  /** The athlete's name on Strava, while connected. */
  athlete: string | null;
  last_sync: string | null;
  last_error: string | null;
  syncing: boolean;
  /** Stopped for Strava's request limits; carries on after this. */
  paused_until: string | null;
  activities: number;
  runs: number;
}

/** One of the signed-in person's devices that gets notifications.
    Mirrors `Subscription` in crates/niles-webpush/src/store.rs, which
    never sends its keys back. */
export interface PushDevice {
  endpoint: string;
  /** "iPhone", "Chrome on Android". */
  device: string;
  created_at: string;
  /** When a push service last took a notification for it. */
  last_ok_at: string | null;
}

/** What became of a notification: taken for a device, forgotten as
    gone, or not taken this time. */
export interface PushDelivery {
  sent: number;
  gone: number;
  failed: number;
}

/** A blank secret keeps the saved one, for the same Client ID. */
export interface StravaAppUpdate {
  client_id: string;
  client_secret?: string;
}

/** Each field: absent leaves it, empty clears it. */
export type MeUpdate = Partial<
  Pick<Me, "spoken_as" | "address_as" | "notes" | "birthday">
>;

/** One kept recording of a wake, without its audio. */
export interface Capture {
  id: number;
  heard_at: string;
  transcript: string;
  /** `answered` or `dropped` — the label a training set needs. */
  outcome: string;
  bytes: number;
}

/** What the UniFi console says about whoever is asking. */
export interface DeviceView {
  /** Whether there is a console to ask at all. */
  available: boolean;
  /** Whether this request came from the house rather than the tunnel. */
  on_home_network: boolean;
  mac: string | null;
  /** What the console calls it — "Mark's iPhone". */
  name: string | null;
  paired: boolean;
  /** Whether the signed-in person has a phone paired, this one or not. */
  has_phone: boolean;
  signed_in: boolean;
}

export const api = {
  getConfig: () => request<ConfigView>("/config"),

  /** Merge a partial config document, e.g. `{lighting: {daytime_brightness: 85}}`. */
  setup: () => request<SetupReport>("/setup"),
  secrets: () => request<SecretsReport>("/secrets"),
  setSecret: (key: string, value: string) =>
    request<void>(`/secrets/${encodeURIComponent(key)}`, {
      method: "PUT",
      body: JSON.stringify({ value }),
    }),
  clearSecret: (key: string) =>
    request<void>(`/secrets/${encodeURIComponent(key)}`, { method: "DELETE" }),
  climate: () => request<Zone[]>("/climate"),
  setZone: (
    zone: number,
    body:
      | { action: "heat"; celsius: number }
      | { action: "off" }
      | { action: "resume" },
  ) =>
    request<void>(`/climate/${zone}`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  boostHeating: () =>
    request<Boosted>("/climate/boost", { method: "POST" }),
  resumeZones: (zones: number[]) =>
    request<void>("/climate/resume", {
      method: "POST",
      body: JSON.stringify({ zones }),
    }),
  conversations: () => request<ConversationSummary[]>("/chat/conversations"),
  conversation: (id: number) => request<Conversation>(`/chat/conversations/${id}`),
  sendChat: (text: string, target: ChatTarget = {}) =>
    request<SentChat>("/chat", {
      method: "POST",
      body: JSON.stringify({ text, ...target }),
    }),
  answerProposal: (id: number, turn: number, save: boolean) =>
    request<{ reply: string }>(`/chat/conversations/${id}/proposal`, {
      method: "POST",
      body: JSON.stringify({ turn, save }),
    }),
  decideApproval: (id: number, approval: string, decision: Decision) =>
    request<{ reply: string }>(
      `/chat/conversations/${id}/approvals/${encodeURIComponent(approval)}`,
      { method: "POST", body: JSON.stringify({ decision }) },
    ),
  deleteConversation: (id: number) =>
    request<void>(`/chat/conversations/${id}`, { method: "DELETE" }),
  dictate: (audio: Blob) =>
    request<{ text: string }>("/chat/dictation", {
      method: "POST",
      headers: { "content-type": audio.type || "audio/webm" },
      body: audio,
    }),
  artifacts: () => request<ArtifactList>("/me/artifacts"),
  artifact: (id: number) => request<ArtifactDetail>(`/me/artifacts/${id}`),
  artifactRevision: (id: number, n: number) =>
    request<ArtifactRevision>(`/me/artifacts/${id}/revisions/${n}`),
  patchArtifact: (id: number, patch: ArtifactPatch) =>
    request<ArtifactDetail>(`/me/artifacts/${id}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  restoreArtifact: (id: number, n: number) =>
    request<ArtifactDetail>(`/me/artifacts/${id}/restore/${n}`, { method: "POST" }),
  retryArtifact: (id: number) =>
    request<Artifact>(`/me/artifacts/${id}/retry`, { method: "POST" }),
  deleteArtifact: (id: number) =>
    request<void>(`/me/artifacts/${id}`, { method: "DELETE" }),
  groceries: () => request<GroceryList>("/groceries"),
  addGrocery: (name: string) =>
    request<{ item: GroceryItem; already: boolean }>("/groceries", {
      method: "POST",
      body: JSON.stringify({ name }),
    }),
  editGrocery: (id: number, edit: GroceryEdit) =>
    request<GroceryItem>(`/groceries/${id}`, {
      method: "PATCH",
      body: JSON.stringify(edit),
    }),
  removeGrocery: (id: number) =>
    request<void>(`/groceries/${id}`, { method: "DELETE" }),
  nemligSearch: (q: string) =>
    request<NemligProduct[]>(`/groceries/nemlig/search?q=${encodeURIComponent(q)}`),
  chooseNemlig: (id: number, product: NemligProduct | null) =>
    request<GroceryItem>(`/groceries/${id}/nemlig`, {
      method: "PUT",
      body: JSON.stringify({ product }),
    }),
  nemligNext: () => request<NemligOrder | null>("/groceries/nemlig/next"),
  nemligCheck: () => request<NemligProduct[]>("/groceries/nemlig/check"),
  nemligSend: () => request<NemligSent>("/groceries/nemlig/basket", { method: "POST" }),
  nemligDelivery: () => request<DeliveryDay[]>("/groceries/nemlig/delivery"),
  nemligReserve: (slot_id: number) =>
    request<NemligBasket>("/groceries/nemlig/delivery", {
      method: "POST",
      body: JSON.stringify({ slot_id }),
    }),
  clearGroceries: () =>
    request<{ cleared: number }>("/groceries/clear", { method: "POST" }),
  scenes: () => request<string[]>("/scenes"),
  applyScene: (name: string) =>
    request<void>(`/scenes/${encodeURIComponent(name)}`, { method: "POST" }),
  integrations: () => request<Integration[]>("/integrations"),
  plugins: () => request<PluginSummary[]>("/plugins"),
  toolServers: () => request<ToolServer[]>("/tool-servers"),
  refreshToolServer: (id: string) =>
    request<void>(`/tool-servers/${encodeURIComponent(id)}/refresh`, { method: "POST" }),
  previewPlugin: (repo: string) =>
    request<PluginPreview>("/plugins/install/preview", {
      method: "POST",
      body: JSON.stringify({ repo }),
    }),
  installPlugin: (preview: PluginPreview) =>
    request<PluginSummary>("/plugins/install", {
      method: "POST",
      body: JSON.stringify({ token: preview.token, sha256: preview.sha256 }),
    }),
  checkPluginUpdate: (id: string) =>
    request<PluginSummary>(`/plugins/${encodeURIComponent(id)}/check-update`, {
      method: "POST",
    }),
  previewPluginUpdate: (id: string) =>
    request<PluginPreview>(`/plugins/${encodeURIComponent(id)}/update/preview`, {
      method: "POST",
    }),
  updatePlugin: (id: string, preview: PluginPreview) =>
    request<PluginSummary>(`/plugins/${encodeURIComponent(id)}/update`, {
      method: "POST",
      body: JSON.stringify({ token: preview.token, sha256: preview.sha256 }),
    }),
  rollBackPlugin: (id: string) =>
    request<PluginSummary>(`/plugins/${encodeURIComponent(id)}/rollback`, {
      method: "POST",
    }),
  setPluginEnabled: (id: string, enabled: boolean) =>
    request<PluginSummary>(
      `/plugins/${encodeURIComponent(id)}/${enabled ? "enable" : "disable"}`,
      { method: "POST" },
    ),
  removePlugin: (id: string) =>
    request<void>(`/plugins/${encodeURIComponent(id)}`, { method: "DELETE" }),
  pluginLog: (id: string) =>
    request<PluginLogLine[]>(`/plugins/${encodeURIComponent(id)}/log`),
  /** The logo as text, to be drawn inline by `PluginLogo`. */
  pluginLogo: async (id: string): Promise<string> => {
    const response = await fetch(`/plugins/${encodeURIComponent(id)}/logo`);
    if (!response.ok) throw new ApiError("no logo", response.status);
    return response.text();
  },
  surface: (plugin: string, surface: string, room?: string | null) =>
    request<Surface>(
      `/plugins/${encodeURIComponent(plugin)}/surfaces/${encodeURIComponent(surface)}${
        room ? `?room=${encodeURIComponent(room)}` : ""
      }`,
    ),
  act: (
    plugin: string,
    surface: string,
    control: string,
    value: ActionValue,
    room?: string | null,
  ) =>
    request<ActionOutcome>(
      `/plugins/${encodeURIComponent(plugin)}/surfaces/${encodeURIComponent(
        surface,
      )}/actions/${encodeURIComponent(control)}${room ? `?room=${encodeURIComponent(room)}` : ""}`,
      { method: "POST", body: JSON.stringify(value) },
    ),
  speakers: () => request<SpeakersReport>("/speakers"),
  tv: () => request<TvInfo>("/tv"),
  music: () => request<RoomMusic[]>("/music"),
  musicControl: (room: string, control: MusicControl) =>
    request<void>(`/music/${encodeURIComponent(room)}`, {
      method: "POST",
      body: JSON.stringify(control),
    }),
  media: () => request<MediaView>("/media"),
  speakerControl: (id: string, control: SpeakerControl) =>
    request<void>(`/media/speakers/${encodeURIComponent(id)}`, {
      method: "POST",
      body: JSON.stringify(control),
    }),
  mediaGroup: (leader: string, play: boolean) =>
    request<void>(`/media/groups/${encodeURIComponent(leader)}`, {
      method: "POST",
      body: JSON.stringify({ action: play ? "play" : "pause" }),
    }),
  mediaChoices: (kind: "radio" | "spotify", q?: string) =>
    request<MediaChoice[]>(
      `/media/choices?kind=${kind}${q ? `&q=${encodeURIComponent(q)}` : ""}`,
    ),
  mediaStart: (speakers: string[], choice: MediaChoice) =>
    request<void>("/media/start", {
      method: "POST",
      body: JSON.stringify({ speakers, choice }),
    }),
  /** The soundbar to the TV's sound, out of any group. */
  tvSound: () => request<void>("/media/tv", { method: "POST" }),
  calendars: () => request<CalendarsView>("/calendars"),
  removeCalendar: (id: string) =>
    request<void>(`/calendars/${encodeURIComponent(id)}`, { method: "DELETE" }),
  connectIcloud: (body: IcloudConnect) =>
    request<CalendarConnection>("/calendars/icloud", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  /** iCloud only: the others are granted what they serve at sign-in.
      Nothing ticked removes the account, and answers null. */
  setAccountServices: (
    id: string,
    ticks: { calendar: "none" | "read" | "write"; mail: "none" | "read" | "send" },
  ) =>
    request<CalendarConnection | null>(`/calendars/${encodeURIComponent(id)}`, {
      method: "PUT",
      body: JSON.stringify(ticks),
    }),
  /** Shows the prompt on the TV and waits up to a minute for it. */
  pairTv: () => request<{ mac: string | null }>("/tv/pair", { method: "POST" }),
  tvPower: (on: boolean) =>
    request<TvPowered>("/tv/power", { method: "POST", body: JSON.stringify({ on }) }),
  tvVolume: (change: TvVolumeChange) =>
    request<TvVolume>("/tv/volume", { method: "POST", body: JSON.stringify(change) }),
  voices: () => request<Voice[]>("/voices"),
  me: () => request<Me>("/me"),
  updateMe: (update: MeUpdate) =>
    request<void>("/me", { method: "PUT", body: JSON.stringify(update) }),
  unpairPhone: () => request<void>("/me/phone", { method: "DELETE" }),
  /** This Niles's public key, for `applicationServerKey`. */
  pushKey: () => request<{ key: string }>("/push/key"),
  pushDevices: () => request<{ devices: PushDevice[] }>("/me/push"),
  subscribePush: (subscription: PushSubscriptionJSON) =>
    request<void>("/me/push", { method: "POST", body: JSON.stringify(subscription) }),
  unsubscribePush: (endpoint: string) =>
    request<void>("/me/push", { method: "DELETE", body: JSON.stringify({ endpoint }) }),
  testPush: () => request<PushDelivery>("/me/push/test", { method: "POST" }),
  strava: () => request<StravaStatus>("/me/strava"),
  saveStravaApp: (app: StravaAppUpdate) =>
    request<StravaStatus>("/me/strava/app", { method: "PUT", body: JSON.stringify(app) }),
  /** The app and its connection, revoked at Strava first. */
  removeStravaApp: () => request<void>("/me/strava", { method: "DELETE" }),
  syncStrava: () => request<{ started: boolean }>("/me/strava/sync", { method: "POST" }),
  disconnectStrava: () =>
    request<{ revoked_at_strava: boolean }>("/me/strava/connection", { method: "DELETE" }),
  deviceStatus: () => request<DeviceView>("/presence/device"),
  pairDevice: () => request<void>("/presence/device", { method: "POST" }),
  captures: () => request<Capture[]>("/captures"),
  clearCaptures: () => request<void>("/captures", { method: "DELETE" }),
  renameVoice: (speaker: string, display_name: string) =>
    request<void>(`/voices/${encodeURIComponent(speaker)}`, {
      method: "PUT",
      body: JSON.stringify({ display_name }),
    }),
  forgetVoice: (speaker: string) =>
    request<void>(`/voices/${encodeURIComponent(speaker)}`, {
      method: "DELETE",
    }),
  places: (q: string) =>
    request<Place[]>(`/places?q=${encodeURIComponent(q)}`),
  timezones: () => request<string[]>("/timezones"),
  tadoStatus: () => request<TadoStatus>("/presence/tado"),
  chatgptStatus: () => request<ChatGptStatus>("/integrations/chatgpt"),
  /** Start a sign-in: the page to sign in on. The verifier stays with Niles. */
  chatgptSignIn: () =>
    request<{ authorize_url: string }>("/integrations/chatgpt/login", { method: "POST" }),
  /** Finish it with the address the sign-in landed on, or its code. */
  chatgptFinish: (pasted: string) =>
    request<{ email: string | null }>("/integrations/chatgpt/login/complete", {
      method: "POST",
      body: JSON.stringify({ pasted }),
    }),
  chatgptSignOut: () =>
    request<void>("/integrations/chatgpt/login", { method: "DELETE" }),
  tadoConnect: () =>
    request<TadoPending>("/presence/tado/connect", { method: "POST" }),
  patchConfig: (patch: Record<string, unknown>) =>
    request<Applied>("/config", {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),

  /** Drop one override by dotted path, returning it to the file value. */
  resetPath: (path: string) =>
    request<Applied>(`/config/${encodeURIComponent(path)}`, {
      method: "DELETE",
    }),

  devices: () => request<Device[]>("/devices"),

  /**
   * Whether to offer a sign-in button, and to whom.
   *
   * Deliberately outside the gate: asking whether you need to sign in
   * cannot itself require being signed in.
   */
  authStatus: () => request<AuthStatus>("/auth/status"),

  apiToken: () => request<ApiToken>("/auth/api-token"),
  /** A new key; the old one stops working with this answer. */
  rotateApiToken: () =>
    request<ApiToken>("/auth/api-token/rotate", { method: "POST" }),

  /**
   * Set one light.
   *
   * Addressed as `source:name` rather than the bare name: the same name
   * can exist in Z2M and WLED both, and the UI always knows which one
   * it is looking at, so there is nothing for the server to resolve.
   */
  setLight: (device: Device, body: SetLight) =>
    request<null>(
      `/rooms/${encodeURIComponent(device.room)}/${encodeURIComponent(
        `${device.source}:${device.name}`,
      )}`,
      { method: "POST", body: JSON.stringify(body) },
    ),

  /**
   * Move one blind, `0` closed to `100` open.
   *
   * By its bare name, not `source:name` like a light: only Z2M has
   * blinds, so there is nothing to tell apart.
   */
  setPosition: (room: string, device: string, position: number) =>
    request<null>(
      `/rooms/${encodeURIComponent(room)}/${encodeURIComponent(device)}/position`,
      { method: "POST", body: JSON.stringify({ position }) },
    ),

  /**
   * Set every light in the house at once.
   *
   * One request for the same reason the room fan-out is one: over a
   * mobile connection, a request per room arrives as the house going
   * dark room by room.
   */
  setAllLights: (body: SetLight) =>
    request<{ lights: number }>("/lights", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  /**
   * Set every light in a room at once.
   *
   * One request rather than one per light: over a phone's connection
   * the fan-out is the difference between a room changing and a room
   * changing light by light.
   */
  setRoom: (room: string, body: SetLight) =>
    request<{ lights: number }>(`/rooms/${encodeURIComponent(room)}`, {
      method: "POST",
      body: JSON.stringify(body),
    }),

  automations: () => request<{ rules: AutomationView[] }>("/automations"),
  createAutomation: (rule: Omit<Rule, "id">) =>
    request<AutomationView>("/automations", { method: "POST", body: JSON.stringify(rule) }),
  updateAutomation: (rule: Rule) =>
    request<AutomationView>(`/automations/${encodeURIComponent(rule.id)}`, {
      method: "PUT",
      body: JSON.stringify(rule),
    }),
  deleteAutomation: (id: string) =>
    request<void>(`/automations/${encodeURIComponent(id)}`, { method: "DELETE" }),
  /** The rule's last runs, newest first. */
  automationRuns: (id: string) =>
    request<{ runs: Run[] }>(`/automations/${encodeURIComponent(id)}/runs`),

  /** Every skill, archived ones too, by name. */
  skills: () => request<Skill[]>("/skills"),
  skill: (name: string) => request<SkillDetail>(`/skills/${encodeURIComponent(name)}`),
  /** Pin or unpin, archive or restore. */
  updateSkill: (name: string, change: SkillChange) =>
    request<void>(`/skills/${encodeURIComponent(name)}`, {
      method: "PATCH",
      body: JSON.stringify(change),
    }),
  /** Refused while the skill is pinned. */
  deleteSkill: (name: string) =>
    request<void>(`/skills/${encodeURIComponent(name)}`, { method: "DELETE" }),
  /** Back to how it was before its last change; says what was undone. */
  undoSkill: (name: string) =>
    request<SkillChangeRecord>(`/skills/${encodeURIComponent(name)}/undo`, { method: "POST" }),
  skillsTidied: () => request<SkillTidy>("/skills/_tidy"),

  memory: () => request<Memory>("/memory"),
  /** Only the entry whose text is exactly `text`. */
  forgetMemory: (target: MemoryTarget, text: string) =>
    request<void>(`/memory/${target}`, {
      method: "DELETE",
      body: JSON.stringify({ text }),
    }),
  /** Put a forgotten note back. */
  restoreMemory: (target: MemoryTarget, text: string) =>
    request<void>(`/memory/${target}/restore`, {
      method: "POST",
      body: JSON.stringify({ text }),
    }),

  history: () => request<Revision[]>("/config/history"),

  undo: () => request<Applied>("/config/undo", { method: "POST" }),
};

/** Build the nested patch object a dotted path implies. */
export function patchFor(path: string, value: unknown): Record<string, unknown> {
  const segments = path.split(".");
  const leaf = segments.pop();
  if (!leaf) throw new Error(`invalid config path: ${path}`);
  let node: Record<string, unknown> = { [leaf]: value };
  for (const segment of segments.reverse()) {
    node = { [segment]: node };
  }
  return node;
}

/**
 * One patch document covering several dotted paths.
 *
 * A row saves everything it changed in one request, so a ramp whose
 * start and end both moved is one write, one revision, and one undo
 * rather than two of each.
 */
export function patchForAll(
  entries: Array<{ path: string; value: unknown }>,
): Record<string, unknown> {
  return entries.reduce<Record<string, unknown>>(
    (patch, entry) => mergeInto(patch, patchFor(entry.path, entry.value)),
    {},
  );
}

function mergeInto(
  target: Record<string, unknown>,
  source: Record<string, unknown>,
): Record<string, unknown> {
  for (const [key, value] of Object.entries(source)) {
    const existing = target[key];
    target[key] =
      isTable(value) && isTable(existing)
        ? mergeInto({ ...existing }, value)
        : value;
  }
  return target;
}

function isTable(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * The arranged room order, as `[rooms] order` holds it.
 *
 * Read in two places — the dashboard that obeys it and the settings
 * page that writes it — and they have to agree about what a missing or
 * malformed entry means, which is "no opinion" rather than "no rooms".
 */
export function roomOrder(effective: unknown): string[] {
  const value = valueAt(effective, "rooms.order");
  return Array.isArray(value) ? value.filter((v) => typeof v === "string") : [];
}

/**
 * The rooms left off the dashboard's grid, as `[rooms] hidden` holds
 * them. Read the same way as the order and for the same reason: a
 * missing or malformed entry hides nothing.
 */
export function roomsHidden(effective: unknown): string[] {
  const value = valueAt(effective, "rooms.hidden");
  return Array.isArray(value) ? value.filter((v) => typeof v === "string") : [];
}

/** Read a dotted path out of a nested object. */
export function valueAt(root: unknown, path: string): unknown {
  return path
    .split(".")
    .reduce<unknown>(
      (node, segment) =>
        node && typeof node === "object"
          ? (node as Record<string, unknown>)[segment]
          : undefined,
      root,
    );
}
