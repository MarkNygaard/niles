import { Power, Tv } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { TvInfo } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface TvCardProps {
  tv?: TvInfo;
  busy?: boolean;
  onPower: (on: boolean) => void;
}

/** "Netflix", "On", or "Off". */
export function tvLine(tv: TvInfo): string {
  if (!tv.status?.on) return "Off";
  return tv.status.app ?? "On";
}

/**
 * The TV, when one is paired: whether it is on, what is on it, and the
 * button to change the first. Nothing at all before it is paired — a
 * card that can only say "not set up" belongs in Settings.
 */
export function TvCard({ tv, busy, onPower }: TvCardProps) {
  if (!tv?.configured || !tv.paired) return null;
  const on = Boolean(tv.status?.on);
  return (
    <div className="bg-card flex items-center gap-3 rounded-xl px-4 py-3">
      <Tv
        aria-hidden
        className={cn("size-5 shrink-0", on ? "text-primary" : "text-muted-foreground")}
      />
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">TV</div>
        <div className="text-muted-foreground truncate text-xs">{tvLine(tv)}</div>
      </div>
      <Button
        variant={on ? "default" : "outline"}
        size="icon"
        aria-label={on ? "Turn the TV off" : "Turn the TV on"}
        disabled={busy}
        onClick={() => onPower(!on)}
      >
        <Power aria-hidden />
      </Button>
    </div>
  );
}
