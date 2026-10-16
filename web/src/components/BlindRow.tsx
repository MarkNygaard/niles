import { Blinds } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CommitSlider } from "@/components/LightRow";
import { blindState, humanize, withPosition } from "@/lib/rooms";
import type { Device } from "@/lib/api";

export interface BlindRowProps {
  blind: Device;
  disabled?: boolean;
  /** `0` closed, `100` open, or "open" — the normal open level, all
      the way when the blind is already there (the second press). */
  onSet: (position: number | "open") => void;
}

/**
 * One blind: open, close, or anywhere between.
 *
 * The buttons are for the two answers nearly every press wants; the
 * slider is for the rest. It sends on release like the light sliders,
 * and for more reason: a blind takes seconds to travel, so every
 * position passed through on the way would set the motor off towards
 * each of them in turn.
 */
export function BlindRow({ blind, disabled, onSet }: BlindRowProps) {
  const name = humanize(blind.name);
  return (
    <div className="flex flex-col gap-2 py-3">
      <div className="flex items-center gap-3">
        <span
          aria-hidden
          className="bg-muted text-muted-foreground flex size-10 shrink-0 items-center justify-center rounded-full [&_svg]:size-4"
        >
          <Blinds />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">{name}</div>
          <div className="text-muted-foreground text-xs">{blindState(blind)}</div>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={disabled}
          aria-label={`Open ${name}`}
          onClick={() => onSet("open")}
        >
          Open
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={disabled}
          aria-label={`Close ${name}`}
          onClick={() => onSet(0)}
        >
          Close
        </Button>
      </div>
      <CommitSlider
        label={`${name} position`}
        // A blind that has never said is drawn closed, which is how
        // most of them spend most of their time; the line above still
        // says it has not been heard from.
        value={blind.state.position ?? 0}
        min={0}
        max={100}
        disabled={disabled}
        format={(position) => blindState(withPosition(blind, position))}
        onCommit={onSet}
      />
    </div>
  );
}

export interface AllBlindsProps {
  /** The room's label, for the buttons' names. */
  room: string;
  disabled?: boolean;
  /** `0` closed, `100` open, or "open" — the normal open level, all
      the way the second press. */
  onSet: (position: number | "open") => void;
}

/** Every blind in a room at once, for a room with more than one. */
export function AllBlinds({ room, disabled, onSet }: AllBlindsProps) {
  return (
    <div className="flex items-center gap-3 py-3">
      <span className="text-muted-foreground min-w-0 flex-1 text-xs">All blinds</span>
      <Button
        variant="outline"
        size="sm"
        disabled={disabled}
        aria-label={`Open all blinds in ${room}`}
        onClick={() => onSet("open")}
      >
        Open all
      </Button>
      <Button
        variant="outline"
        size="sm"
        disabled={disabled}
        aria-label={`Close all blinds in ${room}`}
        onClick={() => onSet(0)}
      >
        Close all
      </Button>
    </div>
  );
}
