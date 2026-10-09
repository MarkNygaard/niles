import { useEffect, useState } from "react";
import { Music, Pause, Play } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import type { RoomMusic } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface MusicRowProps {
  /** What the row is called: the room, on the Media page. */
  title?: string;
  music: RoomMusic;
  busy?: boolean;
  onPause: () => void;
  onPlay: () => void;
  onVolume: (percent: number) => void;
}

/** What the row says under "Music". */
export function musicLine(music: RoomMusic): string {
  if (!music.playing) return "Paused";
  if (music.kind === "tv") return "The TV's sound";
  if (music.what) return music.what;
  return music.kind === "radio" ? "The radio" : "Playing";
}

/** Whether the room's card should carry the music mark: music or the
    radio, not the TV, which has a mark of its own. */
export function musicPlaying(music?: RoomMusic): boolean {
  return Boolean(music?.playing && music.kind !== "tv");
}

/**
 * The room's Sonos, in its view above the lights: what plays, pause or
 * play, and how loud. Every speaker in the room moves together, the way
 * "louder in the living room" moves them.
 */
export function MusicRow({
  title = "Music",
  music,
  busy,
  onPause,
  onPlay,
  onVolume,
}: MusicRowProps) {
  const [draft, setDraft] = useState(music.volume ?? 0);
  useEffect(() => setDraft(music.volume ?? 0), [music.volume]);

  return (
    <div className="flex flex-col gap-2 py-3">
      <div className="flex items-center gap-3">
        <Music
          aria-hidden
          className={cn(
            "size-5 shrink-0",
            music.playing ? "text-primary" : "text-muted-foreground",
          )}
        />
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium">{title}</div>
          <div className="text-muted-foreground truncate text-xs">{musicLine(music)}</div>
        </div>
        <Button
          variant="outline"
          size="icon"
          aria-label={`${music.playing ? "Pause" : "Play"} ${title === "Music" ? "the music" : title}`}
          disabled={busy}
          onClick={music.playing ? onPause : onPlay}
        >
          {music.playing ? <Pause aria-hidden /> : <Play aria-hidden />}
        </Button>
      </div>
      {music.volume !== null && (
        <Slider
          value={draft}
          min={0}
          max={100}
          step={1}
          thumbLabel={`${title} volume`}
          thumbValueText={`${draft}%`}
          disabled={busy}
          onValueChange={(next, details) => {
            const picked = Array.isArray(next) ? next[0] : next;
            setDraft(picked);
            // A tap on the track never reaches onValueCommitted on a
            // touch screen — the Base UI quirk the light rows hit.
            if (details.reason === "track-press") onVolume(picked);
          }}
          onValueCommitted={(next) => onVolume(Array.isArray(next) ? next[0] : next)}
        />
      )}
    </div>
  );
}
