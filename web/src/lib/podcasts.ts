import type { PodcastsTest } from "@/lib/api";

/**
 * The Podcasts add-on's logic: the defaults the config shares, the
 * voice pickers' choices and labels, the address normalisers, and what
 * the connection test's answer means.
 *
 * Mirrors `PodcastsConfig` in crates/niles-config/src/integrations.rs
 * and the test in crates/niles-podcasts: the app shows what that
 * configuration will do, so the two are written as twins.
 */

/** `[integrations.podcasts]` as the config has it, if the add-on is added. */
export interface PodcastsSettings {
  kokoro_url?: string;
  endpoint?: string;
  bucket?: string;
  region?: string;
  first_voice?: string;
  second_voice?: string;
  keep_days?: number;
}

/** The hosts' voices when none are picked. Twin of `PodcastsConfig`'s
    defaults in niles-config. */
export const DEFAULT_VOICES = ["af_heart", "am_michael"] as const;
/** The signing region when none is picked: R2 ignores it. Twin of
    `PodcastsConfig`'s default in niles-config. */
export const DEFAULT_REGION = "auto";
/** How long an episode is kept when nothing is chosen. Twin of
    `PodcastsConfig`'s default in niles-config. */
export const DEFAULT_KEEP_DAYS = 14;

/** Whether a voice id is one of Kokoro's English voices: `af_heart`,
    `bm_george`. Episodes are made in English only, so the pickers
    offer nothing else. */
export function isEnglishVoice(id: string): boolean {
  return /^[ab][a-z]_[a-z0-9_]+$/.test(id);
}

const ACCENTS: Record<string, string> = { a: "American", b: "British" };
const GENDERS: Record<string, string> = { f: "woman", m: "man" };

/** The voice as the picker says it: "Heart · American woman". An
    unknown id says itself, so a saved voice Kokoro no longer lists
    still shows what is saved. */
export function voiceLabel(id: string): string {
  const match = /^([ab])([fm])_([a-z0-9_]+)$/.exec(id);
  if (!match) return id;
  const [, accent, gender, name] = match;
  const label = name
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
  return `${label} · ${ACCENTS[accent]} ${GENDERS[gender]}`;
}

/**
 * What a host's picker offers: Kokoro's English voices minus the other
 * host's, always including the current one — a voice it no longer
 * lists stays visible, so the person sees what is saved and the test
 * flags it — sorted by id.
 */
export function voiceChoices(voices: string[], current: string, other: string): string[] {
  const choices = voices.filter((v) => isEnglishVoice(v) && v !== other);
  if (!choices.includes(current)) choices.push(current);
  return choices.sort();
}

/**
 * The address as Kokoro answers at it: a scheme when none was typed —
 * Kokoro is nearly always on the cluster's own network, so a bare
 * host is http — and without a trailing `/v1` or `/`, which Kokoro
 * does not serve. A pasted `kokoro.ns.svc:8880/v1/` is the commonest
 * way to get it wrong.
 */
export function kokoroAddress(typed: string): string {
  const trimmed = typed.trim();
  if (!trimmed) return "";
  const withScheme = /^https?:\/\//i.test(trimmed) ? trimmed : `http://${trimmed}`;
  return withScheme.replace(/\/+$/, "").replace(/\/v1$/, "").replace(/\/+$/, "");
}

/**
 * The endpoint as the bucket is reached at it: a scheme when none was
 * typed — an R2 endpoint is always https — and without a trailing `/`,
 * so a pasted `https://…r2.cloudflarestorage.com/` works as is.
 */
export function r2Endpoint(typed: string): string {
  const trimmed = typed.trim();
  if (!trimmed) return "";
  const withScheme = /^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
  return withScheme.replace(/\/+$/, "");
}

/** The retention as a number of days, or null when the typed value is
    not an integer from 1 to 365. */
export function keepDays(typed: string): number | null {
  const n = Number(typed);
  return Number.isInteger(n) && n >= 1 && n <= 365 ? n : null;
}

/** Where a test is: not run, running, answered, or the request failed. */
export type Probe =
  | { state: "idle" }
  | { state: "running" }
  | { state: "done"; result: PodcastsTest }
  | { state: "failed"; message: string };

export type Tone = "off" | "pending" | "ok" | "bad";

/**
 * The status line: checking until the test answers; what is still
 * missing when it could not run; "Working" only when Kokoro and the
 * storage both passed — the parts themselves say which failed.
 */
export function statusLine(probe: Probe): { tone: Tone; text: string } {
  switch (probe.state) {
    case "idle":
    case "running":
      return { tone: "pending", text: "Checking…" };
    case "failed":
      return { tone: "bad", text: `Not reachable: ${probe.message}` };
    case "done": {
      const { result } = probe;
      if (!result.configured) {
        return { tone: "bad", text: `Needs its ${result.missing.join(", ")}` };
      }
      return result.kokoro?.ok && result.storage?.ok
        ? { tone: "ok", text: "Working" }
        : { tone: "bad", text: "Not working" };
    }
  }
}

/** What each of the two test parts found, a line each. */
export function testLines(result: PodcastsTest): string[] {
  const line = (label: string, part: { ok: boolean; error: string | null } | null) => {
    if (!part) return null;
    return part.ok ? `${label}: OK.` : `${label}: ${part.error ?? "it did not answer."}`;
  };
  return [line("Kokoro", result.kokoro), line("Storage", result.storage)].filter(
    (l): l is string => l !== null,
  );
}
