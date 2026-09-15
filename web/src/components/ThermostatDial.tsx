import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

/** tado's own range. Below the bottom is off, not colder. */
export const MIN_C = 5;
export const MAX_C = 25;

export interface ThermostatDialProps {
  /** The temperature it is holding, or `null` when the zone is off. */
  value: number | null;
  disabled?: boolean;
  /** Fires when a drag ends, not while it moves. */
  onCommit: (celsius: number) => void;
}

/** Where a temperature sits in the range, 0 at the bottom. */
export function fractionOf(celsius: number): number {
  return (clamp(celsius) - MIN_C) / (MAX_C - MIN_C);
}

/** The temperature at a fraction of the way up, to the nearest half. */
export function celsiusAt(fraction: number): number {
  const raw = MIN_C + fraction * (MAX_C - MIN_C);
  // Halves, because that is the resolution tado accepts and a
  // thermostat you can set to 20.37° is a thermostat that will round
  // your answer without telling you.
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
 * Committing on release rather than per pixel. Every position is a real
 * write to somebody else's API and a radiator that acts on it; sending
 * forty of them on the way to the one you meant would be forty
 * overrides and a rate limit.
 */
export function ThermostatDial({
  value,
  disabled,
  onCommit,
}: ThermostatDialProps) {
  const [draft, setDraft] = useState<number>(value ?? 20);
  const [dragging, setDragging] = useState(false);
  const column = useRef<HTMLDivElement | null>(null);

  // While a drag is in flight the draft is the truth; afterwards the
  // zone is, so a value that came back different is shown rather than
  // the one that was asked for.
  useEffect(() => {
    if (!dragging && value !== null) setDraft(value);
  }, [value, dragging]);

  function temperatureAt(clientY: number): number {
    const box = column.current?.getBoundingClientRect();
    if (!box) return draft;
    // Measured from the bottom: up is warmer.
    return celsiusAt((box.bottom - clientY) / box.height);
  }

  return (
    <div className="flex flex-col items-center gap-4">
      <div className="text-center">
        <div className="font-heading text-5xl leading-none font-medium tabular-nums">
          {draft.toFixed(1)}
          <span className="align-top text-2xl">°</span>
        </div>
      </div>

      <div
        ref={column}
        role="slider"
        aria-label="Target temperature"
        aria-valuemin={MIN_C}
        aria-valuemax={MAX_C}
        aria-valuenow={draft}
        aria-valuetext={`${draft.toFixed(1)} degrees`}
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
          setDraft(temperatureAt(e.clientY));
        }}
        onPointerMove={(e) => {
          if (!dragging) return;
          setDraft(temperatureAt(e.clientY));
        }}
        onPointerUp={(e) => {
          if (!dragging) return;
          const next = temperatureAt(e.clientY);
          setDragging(false);
          setDraft(next);
          onCommit(next);
        }}
        onKeyDown={(e) => {
          // A half-degree a press, the same step a drag lands on.
          const step =
            e.key === "ArrowUp" ? 0.5 : e.key === "ArrowDown" ? -0.5 : 0;
          if (step === 0) return;
          e.preventDefault();
          const next = clamp(draft + step);
          setDraft(next);
          onCommit(next);
        }}
      >
        <div
          aria-hidden
          className="bg-background absolute inset-x-0 bottom-0 transition-[height] duration-100"
          style={{ height: `${fractionOf(draft) * 100}%` }}
        />
        {/* The grip, where a thumb expects one. */}
        <div
          aria-hidden
          className="bg-muted-foreground/40 absolute left-1/2 h-1 w-10 -translate-x-1/2 rounded-full"
          style={{ bottom: `calc(${fractionOf(draft) * 100}% - 0.75rem)` }}
        />
      </div>
    </div>
  );
}
