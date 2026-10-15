import { Minus, Plus, Tv, Volume2, VolumeX } from "lucide-react";
import { CommitSlider } from "@/components/LightRow";
import { PowerButton } from "@/components/PowerButton";
import { Button } from "@/components/ui/button";
import type { TvInfo, TvVolume, TvVolumeChange } from "@/lib/api";
import { cn } from "@/lib/utils";

/** One press of a volume button: the remote's step. */
export const TV_VOLUME_STEP = 1;

export interface TvRowProps {
  tv: TvInfo;
  busy?: boolean;
  onPower: (on: boolean) => void;
  /** Without it the row has no volume controls. */
  onVolume?: (change: TvVolumeChange) => void;
}

/** "Netflix", "On", or "Off". */
export function tvLine(tv: TvInfo): string {
  if (!tv.status?.on) return "Off";
  return tv.status.app ?? "On";
}

/** Whether the TV is on, for the mark on its room's card. */
export function tvOn(tv?: TvInfo): boolean {
  return Boolean(tv?.status?.on);
}

/**
 * The TV's own volume, while it is on, says what it is, and its sound
 * comes out of its own speakers. Sent to a soundbar, the number is the
 * TV's and not what anyone hears. Only newer TVs say where it goes.
 */
export function tvVolume(tv?: TvInfo): TvVolume | null {
  const status = tv?.status;
  if (!status?.on || typeof status.volume !== "number") return null;
  if (status.sound_output && status.sound_output !== "tv_speaker") return null;
  return { volume: status.volume, muted: Boolean(status.muted) };
}

/** Where a change leaves the volume, to draw before the TV answers. */
export function afterChange(now: TvVolume, change: TvVolumeChange): TvVolume {
  if ("mute" in change) return { ...now, muted: change.mute };
  const volume = "volume" in change ? change.volume : now.volume + change.delta;
  return { ...now, volume: Math.min(100, Math.max(0, volume)) };
}

/** `tv` with its volume replaced, as the query keeps it. */
export function withTvVolume(tv: TvInfo | undefined, sound: TvVolume): TvInfo | undefined {
  if (!tv?.status) return tv;
  return { ...tv, status: { ...tv.status, ...sound } };
}

/**
 * The TV, in its room's view, above the lights: what is on it, the
 * switch, and — while it is on — its own speakers. In the room rather
 * than on the dashboard, where a card of its own pushed the bottom row
 * of rooms off a phone's screen.
 */
export function TvRow({ tv, busy, onPower, onVolume }: TvRowProps) {
  const on = tvOn(tv);
  const sound = tvVolume(tv);
  return (
    <div className="flex flex-col gap-2 py-3">
      <div className="flex items-center gap-3">
        <Tv
          aria-hidden
          className={cn("size-5 shrink-0", on ? "text-primary" : "text-muted-foreground")}
        />
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium">TV</div>
          <div className="text-muted-foreground truncate text-xs">{tvLine(tv)}</div>
        </div>
        <PowerButton on={on} label="TV" disabled={busy} onToggle={() => onPower(!on)} />
      </div>
      {sound && onVolume && <TvVolumeControls sound={sound} busy={busy} onVolume={onVolume} />}
    </div>
  );
}

/** Mute, a step down, the slider — sent on release — and a step up. */
export function TvVolumeControls({
  sound,
  busy,
  onVolume,
}: {
  sound: TvVolume;
  busy?: boolean;
  onVolume: (change: TvVolumeChange) => void;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={sound.muted ? "Unmute the TV" : "Mute the TV"}
        aria-pressed={sound.muted}
        disabled={busy}
        onClick={() => onVolume({ mute: !sound.muted })}
      >
        {sound.muted ? <VolumeX aria-hidden /> : <Volume2 aria-hidden />}
      </Button>
      <Button
        variant="outline"
        size="icon-sm"
        aria-label="TV volume down"
        disabled={busy || sound.volume <= 0}
        onClick={() => onVolume({ delta: -TV_VOLUME_STEP })}
      >
        <Minus aria-hidden />
      </Button>
      <div className={cn("min-w-0 flex-1 px-1", sound.muted && "opacity-50")}>
        <CommitSlider
          label="TV volume"
          value={sound.volume}
          min={0}
          max={100}
          disabled={busy}
          format={(volume) => `${volume}`}
          onCommit={(volume) => onVolume({ volume })}
        />
      </div>
      <Button
        variant="outline"
        size="icon-sm"
        aria-label="TV volume up"
        disabled={busy || sound.volume >= 100}
        onClick={() => onVolume({ delta: TV_VOLUME_STEP })}
      >
        <Plus aria-hidden />
      </Button>
    </div>
  );
}
