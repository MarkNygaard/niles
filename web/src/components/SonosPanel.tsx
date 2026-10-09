import { CommitField } from "@/components/CommitField";
import { Switch } from "@/components/ui/switch";
import type { SpeakersReport } from "@/lib/api";

export interface SonosPanelProps {
  enabled: boolean;
  host: string;
  /** What the speaker at `host` described, once there is one. */
  found?: SpeakersReport;
  saving?: boolean;
  onEnabled: (enabled: boolean) => void;
  onHost: (host: string) => void;
}

/** "Found 4 Sonos rooms: Living Room, …", or why not. */
export function foundText(found?: SpeakersReport): string | null {
  if (!found?.configured) return null;
  if (found.error) return found.error;
  const answering = found.sonos.filter((s) => s.answering);
  if (answering.length === 0) return "The speaker answered, but it knows of no rooms.";
  const rooms = answering.length === 1 ? "1 Sonos room" : `${answering.length} Sonos rooms`;
  return `Found ${rooms}: ${answering.map((s) => s.name).join(", ")}.`;
}

/**
 * Sonos: where to find it, and a switch.
 *
 * One address, of any speaker — each describes the whole household, so
 * this is not the one that plays. Which room each plays in is under
 * Settings → Speakers, beside the satellites.
 */
export function SonosPanel({ enabled, host, found, saving, onEnabled, onHost }: SonosPanelProps) {
  const text = foundText(found);
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        Niles plays music and radio on your Sonos, and turns them down while it
        speaks — the TV included.
      </p>

      <label className="flex items-center justify-between gap-4">
        <span className="min-w-0">
          <span className="block text-sm font-medium">Use Sonos</span>
          <span className="text-muted-foreground block text-xs">
            Off keeps the address and the rooms.
          </span>
        </span>
        <Switch checked={enabled} disabled={saving} onCheckedChange={(next) => onEnabled(next)} />
      </label>

      <CommitField
        label="Address of any one Sonos"
        value={host}
        placeholder="192.168.10.174"
        disabled={saving}
        onCommit={onHost}
      />

      {text && (
        <p className={found?.error ? "text-destructive text-sm" : "text-sm"}>{text}</p>
      )}

      <p className="text-muted-foreground text-xs">
        Choose which room each plays in under Settings → Speakers. Your router
        lists the speakers' addresses; any of them will do.
      </p>
    </div>
  );
}
