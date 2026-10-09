import { useState } from "react";
import { Button } from "@/components/ui/button";
import { CommitField } from "@/components/CommitField";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { roomChoices } from "@/components/SatellitesCard";
import { humanize } from "@/lib/rooms";
import { ApiError, api } from "@/lib/api";
import type { TvInfo } from "@/lib/api";

export interface TvPanelProps {
  host: string;
  room: string;
  showAnnouncements: boolean;
  tv?: TvInfo;
  rooms: string[];
  saving?: boolean;
  onChange: (entries: { path: string; value: unknown }[]) => void;
  /** After a pairing, so the page reads the key and MAC back. */
  onPaired: () => void;
}

/** What to say about the pairing. */
export function pairingLine(tv?: TvInfo): string {
  if (!tv?.configured) return "Give the TV's address, then pair it.";
  if (!tv.paired) return "Not paired yet.";
  if (!tv.mac) return "Paired. Its MAC address is not known, so Niles cannot turn it on.";
  return `Paired. Woken by ${tv.mac}.`;
}

/**
 * An LG TV: where it is, the pairing, and whether Niles's
 * announcements also go on the screen.
 */
export function TvPanel({
  host,
  room,
  showAnnouncements,
  tv,
  rooms,
  saving,
  onChange,
  onPaired,
}: TvPanelProps) {
  const [pairing, setPairing] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  async function pair() {
    setPairing(true);
    setProblem(null);
    try {
      await api.pairTv();
      onPaired();
    } catch (e) {
      setProblem(e instanceof ApiError ? e.message : String(e));
    } finally {
      setPairing(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        “Turn on the TV”, “put on Netflix on the TV”, “switch the TV to HDMI 2” —
        and Niles's announcements in the corner of the screen while it is on.
        Straight to the TV on your network; no LG account.
      </p>

      <CommitField
        label="The TV's address"
        value={host}
        placeholder="192.168.69.10"
        disabled={saving}
        onCommit={(value) => onChange([{ path: "tv.host", value }])}
      />

      <label className="flex flex-col gap-1">
        <span className="text-muted-foreground text-xs">Room</span>
        <Select
          value={room || null}
          disabled={saving}
          onValueChange={(next: string | null) => {
            if (next) onChange([{ path: "tv.room", value: next }]);
          }}
        >
          <SelectTrigger aria-label="TV room" className="h-9 w-full sm:w-52">
            <SelectValue placeholder="Pick a room">
              {(value: string) => humanize(value)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            {roomChoices(rooms, room || undefined).map((choice) => (
              <SelectItem key={choice} value={choice}>
                {humanize(choice)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </label>

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-3">
          <span className="text-sm">{pairingLine(tv)}</span>
          <Button
            variant="outline"
            disabled={saving || pairing || !host.trim()}
            onClick={pair}
          >
            {tv?.paired ? "Pair again" : "Pair"}
          </Button>
        </div>
        {pairing && (
          <p className="text-sm">
            The TV is asking whether to allow Niles. Accept it with the remote — it
            waits a minute.
          </p>
        )}
        {problem && <p className="text-destructive text-sm">{problem}</p>}
      </div>

      <label className="flex items-center justify-between gap-4">
        <span className="min-w-0">
          <span className="block text-sm font-medium">Announcements on the TV</span>
          <span className="text-muted-foreground block text-xs">
            What Niles says aloud — a delivery, a reminder — also shown on the screen.
          </span>
        </span>
        <Switch
          checked={showAnnouncements}
          disabled={saving}
          onCheckedChange={(next) =>
            onChange([{ path: "tv.show_announcements", value: next }])
          }
        />
      </label>

      <p className="text-muted-foreground text-xs">
        Turning it on needs “Turn on via Wi-Fi” on the TV (Settings → General →
        Devices → External Devices → TV On With Mobile), and a router that lets
        Niles's wake-up reach the TV's network.
      </p>
    </div>
  );
}
