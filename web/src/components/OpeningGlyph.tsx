import type { Opening } from "@/lib/rooms";

/**
 * A door or a window, drawn only when it is open.
 *
 * Presence is the signal, the way the old HA frontend did it: a shut
 * door is not a state worth an icon on a card this size, it is the
 * ordinary case. Something appearing means go and look.
 *
 * Material Symbols `sensor_door` and `sensor_window` (Apache 2.0),
 * outlined rather than filled so they carry the same weight as the
 * thermometer and the droplet already sitting in that row. They are
 * drawn for this exact job — Lucide has a door but offers a set of
 * blinds for a window, which reads as a blind.
 */
const PATHS: Record<Opening["kind"], string> = {
  door: "M18,4v16H6V4H18 M18,2H6C4.9,2,4,2.9,4,4v18h16V4C20,2.9,19.1,2,18,2L18,2z M15.5,10.5c-0.83,0-1.5,0.67-1.5,1.5 s0.67,1.5,1.5,1.5c0.83,0,1.5-0.67,1.5-1.5S16.33,10.5,15.5,10.5z",
  window:
    "M18,2H6C4.9,2,4,2.9,4,4v16c0,1.1,0.9,2,2,2h12c1.1,0,2-0.9,2-2V4C20,2.9,19.1,2,18,2z M18,4v7h-4v-1h-4v1H6V4H18z M6,20 v-7h12v7H6z",
};

export function OpeningGlyph({ kind }: { kind: Opening["kind"] }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      className="size-3 shrink-0"
      aria-hidden
      focusable="false"
    >
      <path d={PATHS[kind]} />
    </svg>
  );
}

/** "Door open", "2 windows open" — what the icon is saying. */
export function openingLabel({ kind, count }: Opening): string {
  const noun = count === 1 ? kind : `${count} ${kind}s`;
  return `${noun.charAt(0).toUpperCase()}${noun.slice(1)} open`;
}
