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
}

/** A device as `GET /devices` reports it. */
export interface Device {
  /** Fully qualified: `z2m:living_room/lamp`, `wled:living_room/tv`. */
  id: string;
  source: string;
  room: string;
  name: string;
  class: string;
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

export const api = {
  getConfig: () => request<ConfigView>("/config"),

  /** Merge a partial config document, e.g. `{lighting: {daytime_brightness: 85}}`. */
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
