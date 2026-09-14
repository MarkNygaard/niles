import { useState } from "react";
import { ChevronRight, Droplets, Thermometer, X } from "lucide-react";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerTitle,
} from "@/components/ui/drawer";
import { BulbGlyph } from "@/components/BulbGlyph";
import { OpeningGlyph, openingLabel } from "@/components/OpeningGlyph";
import { LightRow } from "@/components/LightRow";
import { PowerButton } from "@/components/PowerButton";
import { useMediaQuery } from "@/hooks/useMediaQuery";
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
  const [open, setOpen] = useState(false);
  // A drag handle is meaningless with a mouse and a centred modal is
  // wrong in a hand, so this picks the component rather than restyling
  // one of them. Matches the `sm` breakpoint the card already uses.
  const phone = useMediaQuery("(max-width: 639px)");

  // DialogHeader and DrawerHeader disagree — one is a row with a
  // divider, the other a centred column — and the panel is the same
  // panel either way, so the chrome is written once here and only the
  // parts that carry the accessible name are swapped.
  const contents = (
    <>
      <div className="border-border flex items-start justify-between gap-3 border-b px-4 py-3">
        <div className="min-w-0">
          {phone ? (
            <>
              <DrawerTitle className="font-heading text-base leading-snug font-medium">
                {room.label}
              </DrawerTitle>
              <DrawerDescription className="text-muted-foreground text-sm">
                {roomSummary(room)}
              </DrawerDescription>
            </>
          ) : (
            <>
              <DialogTitle>{room.label}</DialogTitle>
              <DialogDescription>{roomSummary(room)}</DialogDescription>
            </>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <PowerButton
            on={lit}
            label={`All lights in ${room.label}`}
            disabled={disabled}
            onToggle={() => onSetRoom(toggle)}
          />
          {/* No close button on the drawer: it is dismissed by swiping
              it away, which is the gesture the handle is advertising. */}
          {!phone && (
            <DialogClose
              aria-label="Close"
              className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex size-8 items-center justify-center rounded-lg focus-visible:outline-none"
            >
              <X aria-hidden className="size-4" />
            </DialogClose>
          )}
        </div>
      </div>
      <div className="divide-border min-h-0 flex-1 divide-y overflow-y-auto px-4 py-3">
        {room.lights.map((light) => (
          <LightRow
            key={light.id}
            light={light}
            disabled={disabled}
            onSet={(body) => onSetLight(light, body)}
          />
        ))}
        {/* Named rather than simply gone. A control that publishes to
            something not listening looks broken, but a light that
            vanishes when its battery dies is one nobody notices has
            died. */}
        {room.unreachable.length > 0 && (
          <p className="text-muted-foreground py-3 text-xs">
            {room.unreachable.join(", ")}{" "}
            {room.unreachable.length === 1 ? "is" : "are"} not answering.
          </p>
        )}
      </div>
    </>
  );

  return (
    <>
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
              rather than a second target inside the first — which is
              why it is a bulb and no longer a power symbol on a filled
              disc. A lit bulb needs no legend; a disc had to carry the
              colour because a stroked glyph could not hold it, and a
              grid of filled amber discs reads as a warning panel. */}
          <BulbGlyph
            className={cn(
              "size-9 transition-colors sm:size-11",
              lit ? "text-lit" : "text-unlit",
            )}
          />

          <span className="w-full min-w-0 flex-1">
            <span className="font-heading block truncate text-sm leading-snug font-medium sm:text-base">
              {room.label}
            </span>
            <span className="text-muted-foreground block truncate text-xs sm:text-sm">
              {roomSummary(room)}
            </span>
            {(room.temperature !== undefined ||
              room.humidity !== undefined ||
              room.openings.length > 0) && (
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
                {/* Open doors and windows sit with the other things the
                    room is reporting rather than getting a badge of
                    their own: this is another reading, not an alarm. */}
                {room.openings.map((opening) => (
                  <span key={opening.kind} className="flex items-center gap-1">
                    <OpeningGlyph kind={opening.kind} />
                    {openingLabel(opening)}
                  </span>
                ))}
              </span>
            )}
          </span>
        </button>

        <button
          type="button"
          onClick={() => setOpen(true)}
          className="border-border/60 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 flex items-center justify-between gap-1 border-t px-3 py-2.5 text-left text-xs transition-colors focus-visible:outline-none sm:px-4 sm:gap-2">
          <span className="truncate">
            {room.lights.length === 1
              ? "Adjust this light"
              : `Adjust ${room.lights.length} lights`}
          </span>
          <ChevronRight aria-hidden className="size-4 shrink-0" />
        </button>
      </div>

      {phone ? (
        <Drawer open={open} onOpenChange={setOpen} showSwipeHandle>
          {/* Square across the top. It is already flush to the bottom
              and both sides — the rounded corners are the component's
              default and belong to a sheet that floats, which this one
              does not. Overridden here rather than in the component, so
              `shadcn add drawer` can still update it cleanly. */}
          <DrawerContent className="data-[swipe-direction=down]:rounded-t-none">
            {contents}
          </DrawerContent>
        </Drawer>
      ) : (
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent>{contents}</DialogContent>
        </Dialog>
      )}
    </>
  );
}
