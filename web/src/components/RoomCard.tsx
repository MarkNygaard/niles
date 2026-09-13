import { ChevronRight, Droplets, Thermometer, X } from "lucide-react";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { LightRow } from "@/components/LightRow";
import { PowerButton } from "@/components/PowerButton";
import { roomSummary, roomToggle } from "@/lib/rooms";
import type { Room } from "@/lib/rooms";
import type { Device, SetLight } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface RoomCardProps {
  room: Room;
  disabled?: boolean;
  onSetRoom: (body: SetLight) => void;
  onSetLight: (light: Device, body: SetLight) => void;
}

/**
 * A room, at the size you use it at.
 *
 * The card itself is the switch, because "turn the kitchen off" is what
 * anyone opening this page on their way past actually wants, and it
 * should not cost a press to find. Everything finer — which light, how
 * bright, what colour — is one deliberate press away, on its own strip
 * along the bottom rather than behind a long-press nobody would
 * discover and a mouse can't perform.
 */
export function RoomCard({
  room,
  disabled,
  onSetRoom,
  onSetLight,
}: RoomCardProps) {
  const lit = room.on > 0;
  const toggle = roomToggle(room);

  return (
    <Dialog>
      {/* No outline, and no tint when the room is lit. The card is
          separated from the page by its own fill, and the only thing
          carrying colour is the switch — which is the only thing on it
          reporting a state. */}
      <div className="flex flex-col overflow-hidden rounded-xl bg-card text-card-foreground">
        <button
          type="button"
          disabled={disabled}
          aria-label={`${room.label}, ${roomSummary(room)}. Turn all ${toggle.on ? "on" : "off"}.`}
          onClick={() => onSetRoom(toggle)}
          className={cn(
            // Two cards to a row on a phone leaves each about 170px, so
            // the glyph goes above the name rather than stealing a
            // third of the width from it. Side by side from `sm`, where
            // there is room for both.
            "flex flex-1 flex-col items-start gap-2 p-3 text-left transition-colors",
            "sm:flex-row sm:items-center sm:gap-3 sm:p-4",
            "hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
            "disabled:cursor-not-allowed disabled:opacity-60",
          )}
        >
          {/* The whole card is the switch, so the icon is a read-out
              rather than a second target inside the first. */}
          <span
            aria-hidden
            className={cn(
              "flex size-10 shrink-0 items-center justify-center rounded-full transition-colors sm:size-12",
              lit ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground",
            )}
          >
            <PowerGlyph />
          </span>

          <span className="w-full min-w-0 flex-1">
            <span className="font-heading block truncate text-sm leading-snug font-medium sm:text-base">
              {room.label}
            </span>
            <span className="text-muted-foreground block truncate text-xs sm:text-sm">
              {roomSummary(room)}
            </span>
            {(room.temperature !== undefined || room.humidity !== undefined) && (
              <span className="text-muted-foreground/80 mt-1 flex flex-wrap items-center gap-x-3 text-xs">
                {room.temperature !== undefined && (
                  <span className="flex items-center gap-1">
                    <Thermometer aria-hidden className="size-3" />
                    {room.temperature.toFixed(1)}°C
                  </span>
                )}
                {room.humidity !== undefined && (
                  <span className="flex items-center gap-1">
                    <Droplets aria-hidden className="size-3" />
                    {Math.round(room.humidity)}%
                  </span>
                )}
              </span>
            )}
          </span>
        </button>

        <DialogTrigger className="border-border/60 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 flex items-center justify-between gap-1 border-t px-3 py-2.5 text-left text-xs transition-colors focus-visible:outline-none sm:px-4 sm:gap-2">
          <span className="truncate">
            {room.lights.length === 1
              ? "Adjust this light"
              : `Adjust ${room.lights.length} lights`}
          </span>
          <ChevronRight aria-hidden className="size-4 shrink-0" />
        </DialogTrigger>
      </div>

      <DialogContent>
        <DialogHeader>
          <div className="min-w-0">
            <DialogTitle>{room.label}</DialogTitle>
            <DialogDescription>{roomSummary(room)}</DialogDescription>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <PowerButton
              on={lit}
              label={`All lights in ${room.label}`}
              disabled={disabled}
              onToggle={() => onSetRoom(toggle)}
            />
            <DialogClose
              aria-label="Close"
              className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex size-8 items-center justify-center rounded-lg focus-visible:outline-none"
            >
              <X aria-hidden className="size-4" />
            </DialogClose>
          </div>
        </DialogHeader>
        <DialogBody className="divide-border divide-y">
          {room.lights.map((light) => (
            <LightRow
              key={light.id}
              light={light}
              disabled={disabled}
              onSet={(body) => onSetLight(light, body)}
            />
          ))}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

function PowerGlyph() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      className="size-5"
      focusable="false"
    >
      <path d="M12 2v10" />
      <path d="M18.4 6.6a9 9 0 1 1-12.77.04" />
    </svg>
  );
}
