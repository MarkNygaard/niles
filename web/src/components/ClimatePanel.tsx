import { CalendarSync, Power } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ThermostatDial } from "@/components/ThermostatDial";
import type { Zone } from "@/lib/api";

export interface ClimatePanelProps {
  zone: Zone;
  saving?: boolean;
  onHeat: (celsius: number) => void;
  onOff: () => void;
  onResume: () => void;
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
}: ClimatePanelProps) {
  if (!zone.reachable) {
    return (
      <div className="flex flex-col gap-2 py-6 text-center">
        <p className="text-sm font-medium">Not answering</p>
        <p className="text-muted-foreground text-xs">
          tado cannot reach this valve, so there is nothing to read and
          nothing to set. Its last reading is not shown, because it is not
          current.
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center gap-5 py-4">
      <div className="text-muted-foreground flex items-center gap-4 text-xs">
        {zone.temperature !== null && (
          <span>
            Inside now{" "}
            <span className="text-foreground font-medium tabular-nums">
              {zone.temperature.toFixed(1)}°
            </span>
          </span>
        )}
        {zone.humidity !== null && (
          <span>
            Humidity{" "}
            <span className="text-foreground font-medium tabular-nums">
              {Math.round(zone.humidity)}%
            </span>
          </span>
        )}
      </div>

      {zone.on ? (
        <ThermostatDial
          value={zone.target}
          disabled={saving}
          onCommit={onHeat}
        />
      ) : (
        <div className="flex flex-col items-center gap-2 py-8">
          <div className="font-heading text-4xl leading-none font-medium">
            Off
          </div>
          {/* tado's own word for it, and worth borrowing: a zone that is
              off is not doing nothing — it still heats below about 5°
              so the pipes survive. */}
          <p className="text-muted-foreground text-xs">Frost protection</p>
        </div>
      )}

      <p className="text-muted-foreground text-center text-xs">
        {summarise(zone)}
      </p>

      <div className="flex flex-wrap items-center justify-center gap-2">
        {zone.overridden && (
          <Button variant="outline" disabled={saving} onClick={onResume}>
            <CalendarSync aria-hidden /> Resume schedule
          </Button>
        )}
        {zone.on ? (
          <Button variant="ghost" disabled={saving} onClick={onOff}>
            <Power aria-hidden /> Turn off
          </Button>
        ) : (
          <Button variant="ghost" disabled={saving} onClick={() => onHeat(20)}>
            <Power aria-hidden /> Turn on
          </Button>
        )}
      </div>
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
