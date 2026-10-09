import { Tv } from "lucide-react";
import { PowerButton } from "@/components/PowerButton";
import type { TvInfo } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface TvRowProps {
  tv: TvInfo;
  busy?: boolean;
  onPower: (on: boolean) => void;
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
 * The TV, in its room's view, above the lights: what is on it, and the
 * switch. In the room rather than on the dashboard, where a card of its
 * own pushed the bottom row of rooms off a phone's screen.
 */
export function TvRow({ tv, busy, onPower }: TvRowProps) {
  const on = tvOn(tv);
  return (
    <div className="flex items-center gap-3 py-3">
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
  );
}
