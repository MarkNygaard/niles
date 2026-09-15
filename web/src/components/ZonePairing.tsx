import { AlertTriangle } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { humanize } from "@/lib/rooms";
import { cn } from "@/lib/utils";
import type { Zone } from "@/lib/api";

export interface ZonePairingProps {
  zones: Zone[];
  /** Canonical room names Niles knows about, from the devices it sees. */
  rooms: string[];
  saving?: boolean;
  onPair: (zoneId: number, room: string) => void;
}

/**
 * Which room each tado zone is.
 *
 * Niles can guess when the names happen to agree — "Living Room" is
 * `living_room` — but that is luck, not an answer. A house whose zones
 * are called "Stue" and "Soveværelse" matches nothing, and without this
 * there would be no way to say so.
 *
 * A guess is shown as one. The dropdown is filled either way, so the
 * common case is confirming rather than choosing, but a room nobody
 * picked is labelled `from the name` — it moves if either name changes,
 * and that is worth knowing before relying on it.
 */
export function ZonePairing({
  zones,
  rooms,
  saving,
  onPair,
}: ZonePairingProps) {
  if (zones.length === 0) return null;

  return (
    <div className="flex flex-col gap-3">
      <div>
        <h4 className="text-sm font-medium">Rooms</h4>
        <p className="text-muted-foreground text-xs">
          Which room each tado zone heats. Niles fills this in when the names
          match; where they do not, nothing is assumed.
        </p>
      </div>

      {zones.map((zone) => (
        <div key={zone.id} className="flex items-center gap-3">
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm">{zone.name}</span>
            <span className="text-muted-foreground block text-xs">
              {describe(zone)}
            </span>
          </span>
          <Select
            value={zone.room}
            disabled={saving}
            onValueChange={(next: string | null) => {
              if (next) onPair(zone.id, next);
            }}
          >
            <SelectTrigger
              aria-label={`Room for ${zone.name}`}
              className={cn(
                "h-9 w-40",
                // An unplaced zone is the one thing on this list that
                // needs doing, so it says so rather than sitting quietly
                // among the ones that are done.
                zone.placed_by === "nowhere" && "ring-destructive/50 ring-1",
              )}
            >
              <SelectValue placeholder="Pick a room" />
            </SelectTrigger>
            <SelectContent>
              {roomOptions(rooms, zone.room).map((room) => (
                <SelectItem key={room} value={room}>
                  {humanize(room)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      ))}
    </div>
  );
}

/** What the zone is doing, under its name. */
export function describe(zone: Zone): string {
  if (!zone.reachable) return "Not answering";
  const now =
    zone.temperature === null ? "" : `${zone.temperature.toFixed(1)}°`;
  const wanted = !zone.on
    ? "off"
    : zone.target === null
      ? "on"
      : `heating to ${zone.target.toFixed(1)}°`;
  const state = [now, wanted].filter(Boolean).join(" · ");
  return zone.overridden ? `${state} · overridden` : state;
}

/**
 * The rooms to offer, with the one already set included.
 *
 * A zone can be paired to a room that currently has no devices in it —
 * a radiator in a room with no lights is still a room — and dropping it
 * would make the box claim the zone is somewhere it is not.
 */
export function roomOptions(rooms: string[], current: string | null): string[] {
  if (!current || rooms.includes(current)) return rooms;
  return [...rooms, current];
}

/** Whether anything here still needs a person. */
export function unplaced(zones: Zone[]): number {
  return zones.filter((z) => z.placed_by === "nowhere").length;
}

/** The warning for a list with unplaced zones, or nothing. */
export function UnplacedNotice({ zones }: { zones: Zone[] }) {
  const count = unplaced(zones);
  if (count === 0) return null;
  return (
    <p className="text-muted-foreground flex items-center gap-2 text-xs">
      <AlertTriangle aria-hidden className="size-3.5 shrink-0" />
      {count === 1
        ? "One zone has no room yet, so it will not appear on the dashboard."
        : `${count} zones have no room yet, so they will not appear on the dashboard.`}
    </p>
  );
}
