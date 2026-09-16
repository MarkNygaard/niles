import { houseSummary } from "@/lib/rooms";
import type { Room } from "@/lib/rooms";
import { BulbGlyph } from "@/components/BulbGlyph";
import { cn } from "@/lib/utils";

export interface HouseBarProps {
  rooms: Room[];
  disabled?: boolean;
  onToggle: () => void;
}

/**
 * The whole house, above the rooms.
 *
 * Almost always pressed for one thing — turning everything off on the
 * way to bed or out of the door — so it sits where you reach first and
 * says what it will do before you press it.
 *
 * A toggle rather than an off-only button, on the same rule as a room:
 * anything on means off. That keeps a press undoable, which an
 * off-only button would not be.
 *
 * Wide and short rather than another card, so it reads as a heading for
 * the grid rather than a room called "Everything".
 */
export function HouseBar({ rooms, disabled, onToggle }: HouseBarProps) {
  const lit = rooms.some((room) => room.on > 0);
  const summary = houseSummary(rooms);

  return (
    <button
      type="button"
      disabled={disabled}
      aria-label={`The whole house, ${summary}. Turn everything ${lit ? "off" : "on"}.`}
      onClick={onToggle}
      className={cn(
        "flex w-full items-center gap-3 rounded-xl bg-card px-3 py-2.5 text-left transition-colors sm:px-4",
        // Muted at night the way the room cards are, and by the same
        // amount, so the row above the grid reads as part of it rather
        // than as the one thing still at full strength.
        "dark:text-white/85",
        "hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-60",
      )}
    >
      {/* The same read-out as a room card's, at the same weight: the
          house is a row of rooms, and it should not speak louder than
          one of them just because it sits above them. */}
      <BulbGlyph
        className={cn(
          "size-8 transition-colors",
          // The bulb takes the tiles' own dimming rather than the
          // ink's: it is the same statement they make, in the same
          // colours, so it should sit as far back as they do. The
          // token carries it, which is why there is no `dark:` here —
          // in the light theme it is the colour itself.
          lit ? "text-lit-dim" : "text-unlit-dim",
        )}
      />

      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {lit ? "Turn everything off" : "Turn everything on"}
        </span>
        <span className="text-muted-foreground block truncate text-xs">
          {summary}
        </span>
      </span>
    </button>
  );
}
