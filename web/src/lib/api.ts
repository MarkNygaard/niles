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
  /** Only the values changed away from the config file. */
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
 * What a device is reporting.
 *
 * Every field is nullable and null means "not reported", which is not
 * the same as off or zero — a lamp that has never been heard from has
 * `on: null`, and the UI shows that as unknown rather than off.
 */
export interface Provider {
  name: string;
  base_url: string;
  /** Which roles it can serve. Empty means anything. */
  serves?: ("stt" | "llm")[];
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
export interface Integration {
  id: string;
  label: string;
  blurb: string;
  kind: "provider" | "service";
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

/** The LG TV, as `GET /tv` reports it. */
export interface TvInfo {
  configured: boolean;
  paired: boolean;
  mac: string | null;
  room: string | null;
  /** Absent before pairing, or when the TV could not be asked. */
  status: { on: boolean; app: string | null } | null;
  error: string | null;
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
   * lamp on it. Absent on an older server, where every plug was one.
   */
  lamp?: boolean;
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
  const body = text ? JSON.parse(text) : null;
  if (!response.ok) {
    const message =
      body && typeof body.error === "string"
        ? body.error
        : `request failed (${response.status})`;
    throw new ApiError(message, response.status);
  }
  return body as T;
}

/** One message to Niles in the app, and the answer. */
export interface Exchange {
  said: string;
  reply: string;
  /** What wrote the reply, when it was not Niles's own models. */
  via?: "claude";
  /** Why Claude Code did not answer, when it was meant to. Only on a
      reply just sent, not on one read back from history. */
  fallback?: string;
}

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
  chat: () => request<Exchange[]>("/chat"),
  sendChat: (text: string) =>
    request<{ reply: string; via?: "claude"; fallback?: string }>("/chat", {
      method: "POST",
      body: JSON.stringify({ text }),
    }),
  forgetChat: () => request<void>("/chat", { method: "DELETE" }),
  dictate: (audio: Blob) =>
    request<{ text: string }>("/chat/dictation", {
      method: "POST",
      headers: { "content-type": audio.type || "audio/webm" },
      body: audio,
    }),
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
  speakers: () => request<SpeakersReport>("/speakers"),
  tv: () => request<TvInfo>("/tv"),
  music: () => request<RoomMusic[]>("/music"),
  musicControl: (room: string, control: MusicControl) =>
    request<void>(`/music/${encodeURIComponent(room)}`, {
      method: "POST",
      body: JSON.stringify(control),
    }),
  /** Shows the prompt on the TV and waits up to a minute for it. */
  pairTv: () => request<{ mac: string | null }>("/tv/pair", { method: "POST" }),
  tvPower: (on: boolean) =>
    request<void>("/tv/power", { method: "POST", body: JSON.stringify({ on }) }),
  voices: () => request<Voice[]>("/voices"),
  me: () => request<Me>("/me"),
  updateMe: (update: MeUpdate) =>
    request<void>("/me", { method: "PUT", body: JSON.stringify(update) }),
  unpairPhone: () => request<void>("/me/phone", { method: "DELETE" }),
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
