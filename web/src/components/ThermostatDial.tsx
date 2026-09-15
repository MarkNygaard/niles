import { useEffect, useRef, useState } from "react";
import { heatColor } from "@/lib/heat";
import { cn } from "@/lib/utils";

/** tado's own range. */
export const MIN_C = 5;
export const MAX_C = 25;

/**
 * How much of the column sits below 5°C.
 *
 * Two jobs at once, which is why it is one number. Dragging into it
 * means off — off is not a colder temperature, it is the absence of
 * one, so it gets a place of its own rather than being what 5° quietly
 * turns into. And it is where the fill stands when the zone is off,
 * because an empty column reads as broken rather than as off: the
 * rounded cap and the grip have to be somewhere.
 *
 * Big enough to be a comfortable thumb target for turning the heating
 * off deliberately, and to leave a fill you can see. Small enough that
 * it costs almost nothing off the top of the range.
 */
const OFF_ZONE = 0.12;

export interface ThermostatDialProps {
  /** The temperature it is holding, or `null` when the zone is off. */
  value: number | null;
  /** Under the reading — tado's "Frost protection", when it is off. */
  offLabel?: string;
  disabled?: boolean;
  /** Fires when a drag ends, not while it moves. `null` is off. */
  onCommit: (celsius: number | null) => void;
  /**
   * Fires on every change, including mid-drag.
   *
   * Separate from `onCommit` because they answer different questions:
   * this is what the screen should look like, that is what the radiator
   * should do. One is free and the other costs a write.
   */
  onDraft?: (celsius: number | null) => void;
}

/**
 * Where a setting sits in the column.
 *
 * Off sits exactly where the coldest temperature does, so turning a
 * zone off does not empty the control — it lands it at the bottom of
 * its travel, which is what off looks like on a dial.
 */
export function fractionOf(celsius: number | null): number {
  if (celsius === null) return OFF_ZONE;
  const clamped = clamp(celsius);
  return OFF_ZONE + ((clamped - MIN_C) / (MAX_C - MIN_C)) * (1 - OFF_ZONE);
}

/** The setting at a fraction of the way up. `null` is off. */
export function settingAt(fraction: number): number | null {
  if (fraction < OFF_ZONE) return null;
  const within = (fraction - OFF_ZONE) / (1 - OFF_ZONE);
  const raw = MIN_C + within * (MAX_C - MIN_C);
  // Halves, because that is the resolution tado accepts and a
  // thermostat you can set to 20.37° is one that will round your
  // answer without telling you.
  return clamp(Math.round(raw * 2) / 2);
}

function clamp(celsius: number): number {
  return Math.min(MAX_C, Math.max(MIN_C, celsius));
}

/**
 * The setpoint, as a column you fill.
 *
 * Vertical because warmth is: the fill rises with the temperature, and
 * the number you are setting sits above it rather than beside it. It is
 * also the shape a thumb can work on a phone without a second hand.
 *
 * Off lives at the bottom of the same column rather than behind a
 * button. Turning the heating off and turning it down are the same
 * motion, and a control that swapped itself for a block of text at the
 * end of that motion would break the gesture halfway through.
 *
 * Committing on release rather than per pixel. Every position is a real
 * write to somebody else's API and a radiator that acts on it; sending
 * forty of them on the way to the one you meant would be forty
 * overrides and a rate limit.
 */
export function ThermostatDial({
  value,
  offLabel,
  disabled,
  onCommit,
  onDraft,
}: ThermostatDialProps) {
  const [draft, setDraftState] = useState<number | null>(value);
  const [dragging, setDragging] = useState(false);
  const column = useRef<HTMLDivElement | null>(null);

  /**
   * What was asked for and has not come back yet.
   *
   * Letting go used to snap the dial to `value`, which is still the old
   * target: tado has only just been told, and the page does not ask
   * again for a second or so. So it jumped back to where it started and
   * then jumped forward again when the answer arrived — twice, for one
   * drag.
   *
   * Holding what was asked for until the zone agrees removes both.
   * `at` is there so a request that is never agreed to — refused, lost,
   * rounded somewhere unexpected — gives up rather than leaving the
   * dial permanently showing something that is not true.
   */
  const pending = useRef<{ want: number | null; at: number } | null>(null);

  const setDraft = (next: number | null) => {
    setDraftState(next);
    onDraft?.(next);
  };

  // While a drag is in flight the draft is the truth. Afterwards the
  // zone is — once it has caught up, or once waiting for it has stopped
  // being reasonable.
  useEffect(() => {
    if (dragging) return;
    const waiting = pending.current;
    if (waiting) {
      const agreed = waiting.want === value;
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

  /** Ask for a setting, and go on showing it until the zone agrees. */
  const commit = (next: number | null) => {
    pending.current = { want: next, at: Date.now() };
    onCommit(next);
  };

  function settingAtY(clientY: number): number | null {
    const box = column.current?.getBoundingClientRect();
    if (!box) return draft;
    // Measured from the bottom: up is warmer.
    return settingAt((box.bottom - clientY) / box.height);
  }

  /** One step of the arrow keys, off included at the bottom. */
  function stepped(by: number): number | null {
    if (draft === null) return by > 0 ? MIN_C : null;
    const next = draft + by;
    return next < MIN_C ? null : clamp(next);
  }

  return (
    <div className="flex flex-col items-center gap-4">
      {/* Tall enough for the two-line case, so switching between
          "20.0°" and "Off / Frost protection" does not move the dial
          under your thumb — which it did, at the exact moment you were
          dragging towards it. */}
      <div className="flex min-h-16 flex-col justify-center text-center">
        {draft === null ? (
          <>
            <div className="font-heading text-3xl leading-none font-medium">
              Off
            </div>
            {offLabel && (
              <p className="mt-1 text-sm opacity-80">{offLabel}</p>
            )}
          </>
        ) : (
          <div className="font-heading text-4xl leading-none font-medium tabular-nums">
            {draft.toFixed(1)}
            <span className="align-top text-xl">°</span>
          </div>
        )}
      </div>

      <div
        ref={column}
        role="slider"
        aria-label="Target temperature"
        aria-valuemin={MIN_C}
        aria-valuemax={MAX_C}
        aria-valuenow={draft ?? undefined}
        aria-valuetext={draft === null ? "Off" : `${draft.toFixed(1)} degrees`}
        aria-disabled={disabled}
        tabIndex={disabled ? -1 : 0}
        className={cn(
          "relative h-64 w-32 touch-none overflow-hidden rounded-[2rem] select-none",
          // Both translucent, and lighter over lighter: the track
          // lifts the column off the sheet rather than cutting a hole
          // in it, and the fill is lighter again — so the pair reads
          // the same way against the dark teal and the light yellow.
          "bg-white/10",
          "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          disabled && "pointer-events-none opacity-50",
        )}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          setDragging(true);
          setDraft(settingAtY(e.clientY));
        }}
        onPointerMove={(e) => {
          if (!dragging) return;
          setDraft(settingAtY(e.clientY));
        }}
        onPointerUp={(e) => {
          if (!dragging) return;
          const next = settingAtY(e.clientY);
          setDragging(false);
          setDraft(next);
          commit(next);
        }}
        onKeyDown={(e) => {
          const by =
            e.key === "ArrowUp" ? 0.5 : e.key === "ArrowDown" ? -0.5 : 0;
          if (by === 0) return;
          e.preventDefault();
          const next = stepped(by);
          setDraft(next);
          commit(next);
        }}
      >
        <div
          aria-hidden
          className={cn(
            "absolute inset-x-0 bottom-0 bg-white/95",
            // Only between positions, never during a drag: a fill that
            // eases towards your thumb is a fill that is always behind
            // it, and on a control this size that reads as lag.
            !dragging && "transition-[height] duration-100",
          )}
          style={{ height: `${fractionOf(draft) * 100}%` }}
        />
        {/* The grip, where a thumb expects one — including when the
            zone is off, because that is a position on the dial rather
            than the absence of one.

            Painted the colour of the sheet behind rather than a dark
            wash, so it reads as a slot cut through the fill. Worked out
            here rather than passed in: the dial already knows the value
            the sheet is coloured from, and anything handed down would
            arrive a frame late during a drag. */}
        <div
          aria-hidden
          className="absolute left-1/2 h-1 w-10 -translate-x-1/2 rounded-full"
          style={{
            bottom: `calc(${fractionOf(draft) * 100}% - 0.75rem)`,
            backgroundColor: heatColor(draft),
          }}
        />
      </div>
    </div>
  );
}
