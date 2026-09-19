import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

/** A light that is on is on: 0% is what the switch is for. */
export const MIN_PERCENT = 1;
export const MAX_PERCENT = 100;

/**
 * How much of the bar the dimmest setting still fills.
 *
 * The same trick the thermostat's column plays at its bottom end, for
 * the same reason: a fill of one percent is a fill nobody can see, and
 * an empty track reads as broken rather than as dim. It also has to
 * hold the grip, which sits inside the fill's leading edge.
 */
const FLOOR = 0.12;

/** Where a level sits along the bar. */
export function fractionOf(percent: number): number {
  const clamped = clamp(percent);
  const within = (clamped - MIN_PERCENT) / (MAX_PERCENT - MIN_PERCENT);
  return FLOOR + within * (1 - FLOOR);
}

/** The level at a fraction of the way across. */
export function percentAt(fraction: number): number {
  const within = (fraction - FLOOR) / (1 - FLOOR);
  const raw = MIN_PERCENT + within * (MAX_PERCENT - MIN_PERCENT);
  return clamp(Math.round(raw));
}

function clamp(percent: number): number {
  return Math.min(MAX_PERCENT, Math.max(MIN_PERCENT, percent));
}

export interface BrightnessBarProps {
  /** What the light reports, 1–100. */
  value: number;
  label: string;
  /**
   * The light's own colour, as any CSS colour.
   *
   * The bar is filled with it rather than with a neutral white: on a
   * page of several lights it is the one place the colour each one is
   * actually set to can be seen without opening anything.
   */
  color: string;
  disabled?: boolean;
  /** Fires when a drag ends, not while it moves. */
  onCommit: (percent: number) => void;
  /** Fires on every change, mid-drag included. */
  onDraft?: (percent: number) => void;
}

/**
 * Brightness, as the thermostat does temperature — lying down.
 *
 * The same control turned on its side: a thick track, a fill you drag
 * the end of, and a grip painted the colour of the surface behind so
 * it reads as a slot cut through the fill rather than a bar laid on
 * it. Shallower than the thermostat's is wide, because this one is a
 * row in a list of lights and not the only thing on its screen.
 *
 * It commits on release. Every position under a moving thumb is a real
 * MQTT message to a real bulb; sending them all would flood the broker
 * and the light would visibly chase the thumb.
 */
export function BrightnessBar({
  value,
  label,
  color,
  disabled,
  onCommit,
  onDraft,
}: BrightnessBarProps) {
  const [draft, setDraftState] = useState(value);
  const [dragging, setDragging] = useState(false);
  const track = useRef<HTMLDivElement | null>(null);

  const setDraft = (next: number) => {
    setDraftState(next);
    onDraft?.(next);
  };

  /**
   * What was asked for and has not come back yet.
   *
   * Letting go used to hand the bar straight back to the light, and a
   * light does not answer at once or in one piece. It fades — fifteen
   * seconds of it, by default — reporting itself on the way, and a
   * group reports again as each of its bulbs catches up. Every one of
   * those is a number that is true and is not what you asked for, so
   * the bar walked back down the fade and up again.
   *
   * Holding what was asked for until the light agrees removes all of
   * it. `at` is there so a request that is never agreed to — refused,
   * lost, rounded somewhere unexpected — gives up rather than leaving
   * the bar showing something that is not true.
   */
  const pending = useRef<{ want: number; at: number } | null>(null);

  // While a finger is down the draft is the truth. Afterwards the light
  // is — once it has caught up, or once waiting for it has stopped
  // being reasonable.
  useEffect(() => {
    if (dragging) return;
    const waiting = pending.current;
    if (waiting) {
      // Within one, not exactly: a percentage goes to Z2M as 0–254 and
      // comes back rounded, so asking for 43 can honestly answer 42.
      const agreed = Math.abs(waiting.want - value) <= 1;
      const stale = Date.now() - waiting.at > 15_000;
      if (!agreed && !stale) return;
      pending.current = null;
    }
    setDraftState(value);
    onDraft?.(value);
    // `onDraft` is a fresh closure every render; depending on it would
    // run this on every one.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, dragging]);

  /** Ask for a level, and go on showing it until the light agrees. */
  const commit = (next: number) => {
    pending.current = { want: next, at: Date.now() };
    onCommit(next);
  };

  function percentAtX(clientX: number): number {
    const box = track.current?.getBoundingClientRect();
    if (!box) return draft;
    return percentAt((clientX - box.left) / box.width);
  }

  const filled = fractionOf(draft) * 100;

  return (
    <div
      ref={track}
      role="slider"
      aria-label={label}
      aria-valuemin={MIN_PERCENT}
      aria-valuemax={MAX_PERCENT}
      aria-valuenow={draft}
      aria-valuetext={`${draft}%`}
      aria-disabled={disabled}
      tabIndex={disabled ? -1 : 0}
      className={cn(
        "relative h-12 w-full touch-none overflow-hidden rounded-2xl select-none",
        // Darker than the surface rather than lighter: this one sits on
        // a pale sheet, where the thermostat's track sits on a coloured
        // one. Same idea either way — the track is a recess.
        "bg-black/10",
        "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
        disabled && "pointer-events-none opacity-50",
      )}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        setDragging(true);
        setDraft(percentAtX(e.clientX));
      }}
      onPointerMove={(e) => {
        if (!dragging) return;
        setDraft(percentAtX(e.clientX));
      }}
      onPointerUp={(e) => {
        if (!dragging) return;
        const next = percentAtX(e.clientX);
        setDragging(false);
        setDraft(next);
        commit(next);
      }}
      onKeyDown={(e) => {
        const by = e.key === "ArrowRight" ? 5 : e.key === "ArrowLeft" ? -5 : 0;
        if (by === 0) return;
        e.preventDefault();
        const next = clamp(draft + by);
        setDraft(next);
        commit(next);
      }}
    >
      {/* Clipped rather than resized, so the gradient stays fixed to
          the track: a fill that carried its own gradient would squeeze
          the whole ramp into whatever width it happened to have, and
          the colour under the grip would change as you dragged. */}
      <div
        aria-hidden
        className={cn(
          "absolute inset-0",
          // Only between positions, never during a drag: a fill that
          // eases towards your thumb is always behind it, which on a
          // control this size reads as lag.
          !dragging && "transition-[clip-path] duration-100",
        )}
        style={{
          backgroundImage: `linear-gradient(90deg, color-mix(in oklab, ${color} 30%, transparent), ${color})`,
          clipPath: `inset(0 ${100 - filled}% 0 0)`,
        }}
      />

      {/* The grip, where a thumb expects one, painted the colour of the
          sheet behind rather than a dark wash — so it reads as a slot
          cut through the fill. */}
      <div
        aria-hidden
        className="bg-popover absolute top-1/2 h-4 w-1 -translate-y-1/2 rounded-full"
        style={{ left: `calc(${filled}% - 0.75rem)` }}
      />
    </div>
  );
}
