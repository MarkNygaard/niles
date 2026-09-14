import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { humanize } from "@/lib/rooms";

export interface Satellite {
  name: string;
  ip: string;
  room: string;
}

export interface SatellitesCardProps {
  satellites: Satellite[];
  /** Canonical room names Niles knows about, from the devices it sees. */
  rooms: string[];
  saving?: boolean;
  error?: string;
  onChange: (entries: { path: string; value: unknown }[]) => void;
  /** Drop a satellite's whole entry. A map has no way to say "gone" in
      a patch — a smaller map merges rather than replaces. */
  onRemove: (name: string) => void;
}

/** A dotted path is split on dots, so a name with one in it would land
    somewhere else entirely. */
const NAME = /^[a-z0-9_]+$/;

/**
 * The satellites, and which room each is standing in.
 *
 * The room is the whole point of the entry. A satellite that has not
 * been placed still hears you and still answers, but "turn the lights
 * off" has to mean the room you said it in, and without this it means
 * nothing in particular.
 *
 * Rooms are offered rather than typed because Niles already knows which
 * ones exist — it has just been told about every light in them — and a
 * mistyped room is a satellite that silently belongs nowhere.
 */
export function SatellitesCard({
  satellites,
  rooms,
  saving,
  error,
  onChange,
  onRemove,
}: SatellitesCardProps) {
  const [name, setName] = useState("");
  const [ip, setIp] = useState("");
  const [problem, setProblem] = useState<string | null>(null);

  function add(event: React.FormEvent) {
    event.preventDefault();
    const id = name.trim().toLowerCase().replace(/\s+/g, "_");
    const address = ip.trim();
    if (!id || !address) return;
    if (!NAME.test(id)) {
      setProblem("A name can hold letters, numbers and underscores.");
      return;
    }
    if (satellites.some((s) => s.name === id)) {
      setProblem(`There is already a satellite called ${id}.`);
      return;
    }
    setProblem(null);
    onChange([
      {
        path: `satellites.${id}`,
        // Placed in the first room Niles knows about rather than left
        // blank: the config refuses a room that is not a canonical
        // name, so an empty one could not be saved at all.
        value: { ip: address, room: rooms[0] ?? "living_room" },
      },
    ]);
    setName("");
    setIp("");
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Satellites</CardTitle>
        <CardDescription>
          The microphones around the house. Which room one stands in is what
          makes “turn the lights off” mean this room — Niles matches the
          address it hears from against these. Changing them takes a restart.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {satellites.length === 0 && (
          <p className="text-muted-foreground text-sm">
            None yet. A satellite works without being listed here; it just
            cannot be answered about “this room”.
          </p>
        )}

        {satellites.map((satellite) => (
          <div
            key={satellite.name}
            className="bg-muted/40 flex flex-col gap-3 rounded-lg px-3 py-2.5"
          >
            <div className="flex items-center gap-3">
              <span className="min-w-0 flex-1 truncate text-sm font-medium">
                {humanize(satellite.name)}
              </span>
              <Button
                variant="ghost"
                aria-label={`Remove ${satellite.name}`}
                disabled={saving}
                onClick={() => onRemove(satellite.name)}
              >
                <Trash2 aria-hidden />
              </Button>
            </div>
            <div className="flex flex-col gap-2 sm:flex-row">
              <label className="flex min-w-0 flex-1 flex-col gap-1">
                <span className="text-muted-foreground text-xs">Address</span>
                <AddressField
                  value={satellite.ip}
                  label={`${satellite.name} address`}
                  disabled={saving}
                  onCommit={(value) =>
                    onChange([
                      { path: `satellites.${satellite.name}.ip`, value },
                    ])
                  }
                />
              </label>
              <label className="flex flex-col gap-1 sm:w-52">
                <span className="text-muted-foreground text-xs">Room</span>
                <Select
                  value={satellite.room}
                  disabled={saving}
                  onValueChange={(next: string | null) => {
                    if (next)
                      onChange([
                        {
                          path: `satellites.${satellite.name}.room`,
                          value: next,
                        },
                      ]);
                  }}
                >
                  <SelectTrigger
                    aria-label={`${satellite.name} room`}
                    className="h-9 w-full"
                  >
                    <SelectValue placeholder="Pick a room" />
                  </SelectTrigger>
                  <SelectContent>
                    {roomChoices(rooms, satellite.room).map((room) => (
                      <SelectItem key={room} value={room}>
                        {humanize(room)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
            </div>
          </div>
        ))}

        <form onSubmit={add} className="flex flex-col gap-2 sm:flex-row">
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="kitchen_echo"
            aria-label="Satellite name"
            className="sm:w-52"
          />
          <Input
            value={ip}
            onChange={(e) => setIp(e.target.value)}
            placeholder="192.168.42.30"
            aria-label="Satellite address"
            className="font-mono sm:flex-1"
          />
          <Button type="submit" variant="outline" disabled={saving}>
            <Plus aria-hidden /> Add
          </Button>
        </form>

        {(problem || error) && (
          <p className="text-destructive text-sm">{problem ?? error}</p>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * The rooms to offer, with the one already set included.
 *
 * A room Niles has no device in is not in the list, and dropping it
 * would make the box claim the satellite was somewhere it is not.
 */
export function roomChoices(rooms: string[], current?: string): string[] {
  if (!current || rooms.includes(current)) return rooms;
  return [...rooms, current];
}

/** Writes when you leave it, so half an address is never saved. */
function AddressField({
  value,
  label,
  disabled,
  onCommit,
}: {
  value: string;
  label: string;
  disabled?: boolean;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(value);
  }

  return (
    <Input
      value={draft}
      aria-label={label}
      disabled={disabled}
      spellCheck={false}
      className="font-mono"
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => {
        if (draft.trim() && draft !== value) onCommit(draft.trim());
        else setDraft(value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") setDraft(value);
      }}
    />
  );
}
