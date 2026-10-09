import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { BrandMark } from "@/components/BrandMark";
import { roomChoices } from "@/components/SatellitesCard";
import { humanize } from "@/lib/rooms";
import type { SpeakersReport, SonosSpeaker } from "@/lib/api";

/** The select's value for a Sonos placed nowhere: Base UI wants a
    string, and no room is called this. */
const NOWHERE = "-";

export interface SpeakersCardProps {
  report?: SpeakersReport;
  loading?: boolean;
  /** Where each Sonos is placed, by id, as the config has it now. */
  placed: Record<string, string>;
  /** Canonical room names Niles knows about. */
  rooms: string[];
  saving?: boolean;
  error?: string;
  onPlace: (speaker: SonosSpeaker, room: string) => void;
  onUnplace: (speaker: SonosSpeaker) => void;
}

/** What to say under a speaker's name. */
export function describe(speaker: SonosSpeaker): string {
  if (!speaker.answering) return "Not answering";
  const kind = speaker.home_theater ? "Soundbar, plays the TV" : "Speaker";
  return speaker.ip ? `${kind} · ${speaker.ip}` : kind;
}

/**
 * The Sonos rooms, and which room of the house each plays in.
 *
 * Listed as Sonos found them rather than typed in, so a speaker bought
 * yesterday is here waiting for a room. Several can share one: the
 * living room's soundbar and the speaker at the back play together when
 * music is asked for there, and both go quiet while Niles speaks.
 */
export function SpeakersCard({
  report,
  loading,
  placed,
  rooms,
  saving,
  error,
  onPlace,
  onUnplace,
}: SpeakersCardProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Speakers</CardTitle>
        <CardDescription>
          The Sonos speakers, and which room each plays in. Several in one room
          play together, and all of them are turned down while Niles speaks
          there. Changes apply at once.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {loading ? (
          <>
            <Skeleton className="h-14 w-full" />
            <Skeleton className="h-14 w-full" />
          </>
        ) : !report?.configured ? (
          <p className="text-muted-foreground text-sm">
            Add Sonos under Integrations first, with the address of any one of
            your speakers. Niles finds the rest from it.
          </p>
        ) : (
          <>
            {report.error && <p className="text-destructive text-sm">{report.error}</p>}
            {report.sonos.length === 0 && !report.error && (
              <p className="text-muted-foreground text-sm">
                The speaker answered, but it knows of no rooms.
              </p>
            )}
            {report.sonos.map((speaker) => {
              const room = placed[speaker.id];
              return (
                <div
                  key={speaker.id}
                  className="bg-muted/40 flex flex-col gap-3 rounded-lg px-3 py-2.5 sm:flex-row sm:items-center"
                >
                  <div className="flex min-w-0 flex-1 items-center gap-3">
                    <BrandMark id="sonos" label="Sonos" />
                    <div className="min-w-0">
                      <div className="truncate text-sm font-medium">{speaker.name}</div>
                      <div className="text-muted-foreground truncate text-xs">
                        {describe(speaker)}
                      </div>
                    </div>
                  </div>
                  <Select
                    value={room ?? NOWHERE}
                    disabled={saving}
                    onValueChange={(next: string | null) => {
                      if (!next || next === (room ?? NOWHERE)) return;
                      if (next === NOWHERE) onUnplace(speaker);
                      else onPlace(speaker, next);
                    }}
                  >
                    <SelectTrigger aria-label={`${speaker.name} room`} className="h-9 w-full sm:w-52">
                      {/* Base UI shows the raw value unless told
                          otherwise, and the value is the slug. */}
                      <SelectValue placeholder="Not in a room">
                        {(value: string) => (value === NOWHERE ? "Not in a room" : humanize(value))}
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={NOWHERE}>Not in a room</SelectItem>
                      {roomChoices(rooms, room).map((choice) => (
                        <SelectItem key={choice} value={choice}>
                          {humanize(choice)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              );
            })}
          </>
        )}
        {error && <p className="text-destructive text-sm">{error}</p>}
      </CardContent>
    </Card>
  );
}
