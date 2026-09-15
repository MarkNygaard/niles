import { CalendarSync } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ThermostatDial } from "@/components/ThermostatDial";
import type { Zone } from "@/lib/api";

export interface ClimatePanelProps {
  zone: Zone;
  saving?: boolean;
  onHeat: (celsius: number) => void;
  onOff: () => void;
  onResume: () => void;
  /** What the sheet behind this should be coloured, as it is dragged. */
  onDraft?: (celsius: number | null) => void;
}

/**
 * One room's heating, in the drawer the room card already opens.
 *
 * Reads top to bottom the way the question does: what it is now, what
 * it is doing about it, and the control that changes that.
 */
export function ClimatePanel({
  zone,
  saving,
  onHeat,
  onOff,
  onResume,
  onDraft,
}: ClimatePanelProps) {
  if (!zone.reachable) {
    return (
      <div className="flex flex-col gap-2 py-6 text-center">
        <p className="text-sm font-medium">Not answering</p>
        <p className="text-sm opacity-80">
          tado cannot reach this valve, so there is nothing to read and
          nothing to set. Its last reading is not shown, because it is not
          current.
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center gap-5 py-4">
      {/* Upright and spaced, the way a readout is labelled rather than
          the way a sentence is written: these two are captions on the
          numbers beside them, not prose about them. */}
      <div className="flex items-center gap-4 text-sm tracking-wide uppercase opacity-90">
        {zone.temperature !== null && (
          <span>
            Inside now{" "}
            <span className="font-medium tabular-nums">
              {zone.temperature.toFixed(1)}°
            </span>
          </span>
        )}
        {zone.humidity !== null && (
          <span>
            Humidity{" "}
            <span className="font-medium tabular-nums">
              {Math.round(zone.humidity)}%
            </span>
          </span>
        )}
      </div>

      {/* Off is the bottom of the same column rather than a different
          screen: turning the heating off and turning it down are the
          same motion. */}
      <ThermostatDial
        value={zone.on ? zone.target : null}
        // tado's own word, and worth borrowing: a zone that is off is
        // not doing nothing — it still heats below about 5°C so the
        // pipes survive.
        offLabel="Frost protection"
        disabled={saving}
        onDraft={onDraft}
        onCommit={(celsius) =>
          celsius === null ? onOff() : onHeat(celsius)
        }
      />

      {/* Sized up rather than left at `text-xs`: white on these
          colours clears AA for large text and not for small. */}
      <p className="text-center text-sm opacity-80">{summarise(zone)}</p>

      {/* The only button left. On and off are the dial's job now, and
          this is the one thing the dial cannot say: give it back. */}
      {zone.overridden && (
        <Button
          // A dark wash rather than a light one: the sheet behind runs
          // from a dark teal to a light yellow, and only something
          // darker than all of them reads on all of them. No outline
          // either — the fill is the shape.
          variant="ghost"
          disabled={saving}
          onClick={onResume}
          className="rounded-full bg-black/30 px-5 text-inherit hover:bg-black/50"
        >
          <CalendarSync aria-hidden /> Resume schedule
        </Button>
      )}
    </div>
  );
}

/**
 * The line under the dial: whose decision this is, and how long for.
 *
 * An override with no end is the one worth pointing at — it lasts until
 * somebody remembers it, and nothing else will end it.
 */
export function summarise(zone: Zone): string {
  if (!zone.overridden) return "Following the schedule";
  if (!zone.until) return "Set by hand, until you resume the schedule";
  const ends = new Date(zone.until);
  return `Set by hand, until ${ends.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  })}`;
}
