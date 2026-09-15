import { CalendarClock } from "lucide-react";
import { FlameGlyph } from "@/components/FlameGlyph";
import { cn } from "@/lib/utils";

export interface BoostButtonProps {
  /** True while a boost is still running, which makes this the way out. */
  boosting: boolean;
  disabled?: boolean;
  /** True while a request is in flight. */
  pending?: boolean;
  onBoost: () => void;
  onResume: () => void;
}

/**
 * Warm the whole house for a while — and then stop.
 *
 * tado's own button, and worth borrowing for the reason theirs exists:
 * what somebody wants on a cold evening is not a temperature, it is
 * *more*, now, without deciding anything. One press, no dialog.
 *
 * The same button ends it, because the two are one thought and a
 * second control that only means something for half an hour is a
 * control that is wrong most of the time. It goes back by itself when
 * the timer runs out: the boost is the only thing Niles writes with an
 * end time, so a timer still running is a boost still running, and
 * nothing here has to remember that a press happened.
 *
 * It sits beside the house switch because it is the same kind of
 * statement — about the whole house, made from the top of the page.
 */
export function BoostButton({
  boosting,
  disabled,
  pending,
  onBoost,
  onResume,
}: BoostButtonProps) {
  return (
    <button
      type="button"
      disabled={disabled || pending}
      aria-label={
        boosting
          ? "Stop the boost and put every room back on its schedule"
          : "Boost the heating everywhere for half an hour"
      }
      title={boosting ? "Resume schedule" : "Boost heating"}
      onClick={boosting ? onResume : onBoost}
      className={cn(
        "bg-card flex shrink-0 items-center gap-2 rounded-xl px-3 transition-colors sm:px-4",
        "hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-60",
        // Lit while it is doing something, which is the state worth
        // seeing across the room.
        boosting ? "text-lit" : "text-muted-foreground hover:text-foreground",
        // The press and the answer are a round trip per room apart, and
        // a button that looks untouched for that long reads as one that
        // missed.
        pending && "animate-pulse",
      )}
    >
      {boosting ? (
        <>
          <CalendarClock aria-hidden className="size-5" />
          <span className="text-sm font-medium">Resume</span>
        </>
      ) : (
        <FlameGlyph className="size-6" />
      )}
    </button>
  );
}
