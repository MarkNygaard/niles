import { useState } from "react";
import { ChevronRight, Droplets, X } from "lucide-react";
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
import { OpeningGlyph, openingLabel } from "@/components/OpeningGlyph";
import { LightRow } from "@/components/LightRow";
import { PowerButton } from "@/components/PowerButton";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { ClimatePanel } from "@/components/ClimatePanel";
import { HEAT_INK, heatColor } from "@/lib/heat";
import { measured, roomSummary, roomToggle, subtitle } from "@/lib/rooms";
import type { Room } from "@/lib/rooms";
import type { Device, SetLight } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface RoomCardProps {
  room: Room;
  disabled?: boolean;
  onSetRoom: (body: SetLight) => void;
  onSetLight: (light: Device, body: SetLight) => void;
  /** Absent when this instance has no tado connection. */
  onSetZone?: (body: SetZone) => void;
}

/** What the drawer can ask of a heating zone. */
export type SetZone =
  | { action: "heat"; celsius: number }
  | { action: "off" }
  | { action: "resume" };

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
  onSetZone,
}: RoomCardProps) {
  const lit = room.on > 0;
  const toggle = roomToggle(room);
  const heating = room.zone !== undefined && onSetZone !== undefined;
  const [open, setOpen] = useState<"lights" | "heating" | null>(null);
  /**
   * What the heating sheet is coloured, following the dial as it moves.
   *
   * Held here rather than in the panel because the colour belongs to
   * the whole sheet, and the panel is only what is inside it. Reset on
   * open so a sheet never flashes the last room's temperature.
   */
  const [draft, setDraft] = useState<number | null>(null);
  const tinted = open === "heating" && room.zone?.reachable;
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
      <div
        className={cn(
          "flex items-start justify-between gap-3 border-b px-4 py-3",
          tinted ? "border-black/15" : "border-border",
        )}
      >
        <div className="min-w-0">
          {phone ? (
            <>
              <DrawerTitle className="font-heading text-base leading-snug font-medium">
                {room.label}
              </DrawerTitle>
              <DrawerDescription
                className={cn("text-sm", tinted ? "opacity-80" : "text-muted-foreground")}
              >
                {open === "heating" ? "Heating" : roomSummary(room)}
              </DrawerDescription>
            </>
          ) : (
            <>
              <DialogTitle>{room.label}</DialogTitle>
              <DialogDescription>
                {open === "heating" ? "Heating" : roomSummary(room)}
              </DialogDescription>
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
        {/* One subject at a time. Both together meant scrolling past a
            thermostat to reach a light, which is the wrong way round:
            lights are adjusted many times a day and heating rarely. */}
        {open === "heating" && room.zone && onSetZone && (
          <ClimatePanel
            zone={room.zone}
            saving={disabled}
            onDraft={setDraft}
            onHeat={(celsius) => onSetZone({ action: "heat", celsius })}
            onOff={() => onSetZone({ action: "off" })}
            onResume={() => onSetZone({ action: "resume" })}
          />
        )}
        {open === "lights" &&
          room.lights.map((light) => (
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
        {open === "lights" && room.unreachable.length > 0 && (
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
      {/* Filled rather than tinted, and by the lights rather than the
          heating: the whole card is the light switch, so the colour has
          to be the thing the press changes. The temperature on it is a
          reading, not a control. */}
      <div
        className={cn(
          "flex flex-col overflow-hidden rounded-xl transition-colors",
          // Square everywhere. It was only square on a phone because two
          // to a row made it so; a wide screen stretching them into
          // letterboxes made the same grid read as a different one.
          "aspect-square",
          lit
            ? "bg-lit text-lit-foreground"
            : "bg-card text-card-foreground",
        )}
      >
        <button
          type="button"
          disabled={disabled}
          aria-label={`${room.label}, ${roomSummary(room)}. Turn all ${toggle.on ? "on" : "off"}.`}
          onClick={() => onSetRoom(toggle)}
          className={cn(
            "flex flex-1 flex-col items-start gap-1 p-3 text-left sm:p-4",
            "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
            "disabled:cursor-not-allowed disabled:opacity-60",
            lit ? "hover:bg-black/5" : "hover:bg-muted/50",
          )}
        >
          <span className="flex w-full items-start justify-between gap-2">
            {room.humidity !== undefined && (
              <span
                className={cn(
                  "flex items-center gap-1 rounded-full px-2 py-0.5 text-xs",
                  lit ? "bg-white/20" : "bg-muted",
                )}
              >
                <Droplets aria-hidden className="size-3" />
                {Math.round(room.humidity)}%
              </span>
            )}
            {/* Open doors and windows sit up here rather than in the
                readings below: at a glance it is the one thing on the
                card you might act on. */}
            <span className="ml-auto flex items-center gap-1.5">
              {room.openings.map((opening) => (
                <span key={opening.kind} title={openingLabel(opening)}>
                  <OpeningGlyph kind={opening.kind} />
                </span>
              ))}
            </span>
          </span>

          <span className="mt-auto w-full min-w-0">
            {measured(room) !== undefined && (
              <span className="font-heading block text-3xl leading-none font-medium tabular-nums sm:text-4xl">
                {measured(room)!.toFixed(1)}
                <span className="align-top text-lg sm:text-xl">°</span>
              </span>
            )}
            <span className="font-heading mt-1 block truncate text-sm leading-snug font-medium sm:text-base">
              {room.label}
            </span>
            <span
              className={cn(
                "block truncate text-xs",
                lit ? "text-lit-foreground/80" : "text-muted-foreground",
              )}
            >
              {subtitle(room)}
            </span>
          </span>
        </button>

        {/* Two, so the common one costs nothing. Lights are adjusted
            many times a day and heating rarely, and a single button
            labelled for lights is one nobody finds the thermostat
            behind. */}
        <div
          className={cn(
            "flex border-t text-xs",
            lit ? "border-black/10" : "border-border/60",
          )}
        >
          <button
            type="button"
            onClick={() => setOpen("lights")}
            className={cn(
              "flex flex-1 items-center justify-between gap-1 px-3 py-2.5 text-left transition-colors sm:px-4",
              "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
              lit
                ? "hover:bg-black/5"
                : "text-muted-foreground hover:bg-muted hover:text-foreground",
            )}
          >
            <span className="truncate">Lights</span>
            <ChevronRight aria-hidden className="size-4 shrink-0" />
          </button>
          {heating && (
            <button
              type="button"
              onClick={() => {
              setDraft(room.zone?.on ? (room.zone.target ?? null) : null);
              setOpen("heating");
            }}
              className={cn(
                "flex flex-1 items-center justify-between gap-1 border-l px-3 py-2.5 text-left transition-colors sm:px-4",
                "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
                lit
                  ? "border-black/10 hover:bg-black/5"
                  : "border-border/60 text-muted-foreground hover:bg-muted hover:text-foreground",
              )}
            >
              <span className="truncate">Heating</span>
              <ChevronRight aria-hidden className="size-4 shrink-0" />
            </button>
          )}
        </div>
      </div>

      {phone ? (
        <Drawer
          open={open !== null}
          onOpenChange={(next: boolean) => {
            if (!next) setOpen(null);
          }}
          showSwipeHandle
        >
          {/* Square across the top. It is already flush to the bottom
              and both sides — the rounded corners are the component's
              default and belong to a sheet that floats, which this one
              does not. Overridden here rather than in the component, so
              `shadcn add drawer` can still update it cleanly. */}
          <DrawerContent
            className={cn(
              "data-[swipe-direction=down]:rounded-t-none transition-colors duration-200",
            )}
            style={
              tinted
                ? { backgroundColor: heatColor(draft), color: HEAT_INK }
                : undefined
            }
          >
            {contents}
          </DrawerContent>
        </Drawer>
      ) : (
        <Dialog
          open={open !== null}
          onOpenChange={(next: boolean) => {
            if (!next) setOpen(null);
          }}
        >
          <DialogContent
            className="transition-colors duration-200"
            style={
              tinted
                ? { backgroundColor: heatColor(draft), color: HEAT_INK }
                : undefined
            }
          >
            {contents}
          </DialogContent>
        </Dialog>
      )}
    </>
  );
}
