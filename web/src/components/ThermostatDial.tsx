import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

/** tado's own range. */
export const MIN_C = 5;
export const MAX_C = 25;

/**
 * How much of the column below 5°C means off.
 *
 * Off is not a colder temperature, it is the absence of one — so it
 * gets a place of its own at the bottom rather than being what 5°
 * quietly turns into. Small, because it is a corner of the control you
 * should be able to reach deliberately and not fall into.
 */
const OFF_ZONE = 0.06;

export interface ThermostatDialProps {
  /** The temperature it is holding, or `null` when the zone is off. */
  value: number | null;
  /** Under the reading — tado's "Frost protection", when it is off. */
  offLabel?: string;
  disabled?: boolean;
  /** Fires when a drag ends, not while it moves. `null` is off. */
  onCommit: (celsius: number | null) => void;
}

/** Where a setting sits in the column, 0 at the bottom. */
export function fractionOf(celsius: number | null): number {
  if (celsius === null) return 0;
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
}: ThermostatDialProps) {
  const [draft, setDraft] = useState<number | null>(value);
  const [dragging, setDragging] = useState(false);
  const column = useRef<HTMLDivElement | null>(null);

  // While a drag is in flight the draft is the truth; afterwards the
  // zone is, so a value that came back different is shown rather than
  // the one that was asked for.
  useEffect(() => {
    if (!dragging) setDraft(value);
  }, [value, dragging]);

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
      <div className="text-center">
        {draft === null ? (
          <>
            <div className="font-heading text-4xl leading-none font-medium">
              Off
            </div>
            {offLabel && (
              <p className="text-muted-foreground mt-1 text-xs">{offLabel}</p>
            )}
          </>
        ) : (
          <div className="font-heading text-5xl leading-none font-medium tabular-nums">
            {draft.toFixed(1)}
            <span className="align-top text-2xl">°</span>
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
          "bg-muted relative h-64 w-32 touch-none overflow-hidden rounded-[2rem] select-none",
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
          onCommit(next);
        }}
        onKeyDown={(e) => {
          const by =
            e.key === "ArrowUp" ? 0.5 : e.key === "ArrowDown" ? -0.5 : 0;
          if (by === 0) return;
          e.preventDefault();
          const next = stepped(by);
          setDraft(next);
          onCommit(next);
        }}
      >
        <div
          aria-hidden
          className="bg-background absolute inset-x-0 bottom-0 transition-[height] duration-100"
          style={{ height: `${fractionOf(draft) * 100}%` }}
        />
        {/* The grip, where a thumb expects one. Hidden at the very
            bottom, where there is no fill to sit on the edge of. */}
        {draft !== null && (
          <div
            aria-hidden
            className="bg-muted-foreground/40 absolute left-1/2 h-1 w-10 -translate-x-1/2 rounded-full"
            style={{ bottom: `calc(${fractionOf(draft) * 100}% - 0.75rem)` }}
          />
        )}
      </div>
    </div>
  );
}
