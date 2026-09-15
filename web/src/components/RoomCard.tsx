import { useState } from "react";
import { Droplets, Flame, Lightbulb, X } from "lucide-react";
import {
  Dialog,
  DialogBody,
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
import { HEAT_INK, heatSheet } from "@/lib/heat";
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

/**
 * What a room tile is painted.
 *
 * Measured out of tado's own tiles, and not the vertical gradient it
 * looks like: a light corner at the top left, falling away evenly in
 * every direction. Two pairs give it away — the top middle matches the
 * middle left, and the top right matches the bottom left. Equal values
 * at equal distances from one corner is a circle, not a slope.
 *
 * It reaches the far colour at the anti-diagonal and stays there, so
 * the whole bottom-right half of the tile is one flat colour. That is
 * the `70.7%`: the corner-to-corner distance is 1.414 of a side, and
 * the gradient is finished at 1.0 of one.
 *
 * Subtle enough that you would not name it if asked, and a flat fill
 * beside one looks like a swatch rather than a surface — which is most
 * of why theirs reads as a physical thing.
 *
 * Which one shows is the lights, not the heating. The tile is the light
 * switch, so its colour has to be what pressing it changes.
 *
 * White text on both measures about 2.1–2.6:1, which is below AA. That
 * is what tado ships and what was asked for here; the weights below are
 * heavier than the rest of the app to buy back what legibility a weight
 * can, and the secondary line is `text-sm` rather than `text-xs`
 * because nothing smaller survives this background.
 */
const TILE = {
  on: "radial-gradient(circle at top left, #fd9740 0%, #fd8b2d 70.7%)",
  off: "radial-gradient(circle at top left, #adb7c2 0%, #97a2b0 70.7%)",
};

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
  const hasZone = room.zone !== undefined && onSetZone !== undefined;
  const [open, setOpen] = useState<"lights" | "heating" | null>(null);
  /**
   * What the heating sheet is coloured, following the dial as it moves.
   *
   * Held here rather than in the panel because the colour belongs to
   * the whole sheet, and the panel is only what is inside it. Reset on
   * open so a sheet never flashes the last room's temperature.
   */
  const [draft, setDraft] = useState<number | null>(null);
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
              <DialogDescription>
                {roomSummary(room)}
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

  /**
   * Heating, as a sheet rather than a drawer.
   *
   * A drawer is dismissed by dragging it downwards, which is the same
   * gesture as turning the heating down — so every attempt to reach 5°
   * threw the sheet off the bottom of the screen instead. A dialog has
   * no such gesture, and on a phone it already rises from the bottom
   * edge, so it looks like the drawer it replaces without fighting the
   * one control on it.
   *
   * Full height there too, because the dial wants the room and there is
   * nothing else on this screen to share it with.
   */
  const heatingSheet = room.zone && onSetZone && (
    <Dialog
      open={open === "heating"}
      onOpenChange={(next: boolean) => {
        if (!next) setOpen(null);
      }}
    >
      <DialogContent
        className="inset-0 max-h-none rounded-t-none transition-colors duration-200 sm:inset-x-auto sm:top-1/2 sm:bottom-auto sm:left-1/2 sm:h-auto sm:max-h-[85vh] sm:rounded-xl"
        style={{ backgroundImage: heatSheet(draft), color: HEAT_INK }}
      >
        <div className="flex items-center gap-2 px-3 py-3">
          <DialogClose
            aria-label="Close"
            className="focus-visible:ring-3 focus-visible:ring-ring/50 flex size-9 shrink-0 items-center justify-center rounded-lg hover:bg-black/10 focus-visible:outline-none"
          >
            <X aria-hidden className="size-5" />
          </DialogClose>

          <DialogTitle className="font-heading min-w-0 flex-1 truncate text-center text-base font-semibold">
            {room.label}
          </DialogTitle>
          <DialogDescription className="sr-only">
            Heating for {room.label}
          </DialogDescription>

          {/* Nothing on the right. Off is the bottom of the dial, and a
              second way to reach it up here was one control too many on
              a screen that has exactly one. */}
          <span aria-hidden className="size-9 shrink-0" />
        </div>

        <DialogBody className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto">
          <ClimatePanel
            zone={room.zone}
            saving={disabled}
            onDraft={setDraft}
            onHeat={(celsius) => onSetZone({ action: "heat", celsius })}
            onOff={() => onSetZone({ action: "off" })}
            onResume={() => onSetZone({ action: "resume" })}
          />
        </DialogBody>
      </DialogContent>
    </Dialog>
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
          "flex flex-col overflow-hidden rounded-xl text-white",
          // Square everywhere. It was only square on a phone because two
          // to a row made it so; a wide screen stretching them into
          // letterboxes made the same grid read as a different one.
          "aspect-square",
        )}
        style={{ backgroundImage: lit ? TILE.on : TILE.off }}
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
            "hover:bg-black/5",
          )}
        >
          <span className="flex w-full items-start justify-between gap-2">
            {room.humidity !== undefined && (
              <span className="flex items-center gap-1 rounded-full bg-white/25 px-2 py-0.5 text-xs font-semibold">
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
              <Reading celsius={measured(room)!} />
            )}
            <span className="font-heading mt-1 block truncate text-base leading-snug font-semibold sm:text-lg">
              {room.label}
            </span>
            {/* `text-sm`, not `text-xs`: white on these colours is about
                2.3:1, and nothing smaller than this survives it. Full
                white rather than dimmed, since the weight is already
                doing the work of making it secondary. */}
            <span className="block truncate text-sm font-thin">
              {subtitle(room)}
            </span>
          </span>
        </button>

        {/* Two, so the common one costs nothing. Lights are adjusted
            many times a day and heating rarely, and a single button
            labelled for lights is one nobody finds the thermostat
            behind. */}
        {/* Drawn rather than written. Two words and two chevrons took a
            fifth of the card to say what a bulb and a flame say at a
            glance, and the card's own colour and reading are what that
            space is for. The name is still there for anything not
            reading the picture. */}
        <div className="flex border-t border-white/25">
          <button
            type="button"
            aria-label={`Lights in ${room.label}`}
            title="Lights"
            onClick={() => setOpen("lights")}
            className={cn(
              "flex flex-1 items-center justify-center px-3 py-2.5 transition-colors",
              "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
              "hover:bg-black/10",
            )}
          >
            <Lightbulb aria-hidden className="size-5" />
          </button>
          {hasZone && (
            <button
              type="button"
              aria-label={`Heating in ${room.label}`}
              title="Heating"
              onClick={() => {
                setDraft(room.zone?.on ? (room.zone.target ?? null) : null);
                setOpen("heating");
              }}
              className={cn(
                "flex flex-1 items-center justify-center border-l border-white/25 px-3 py-2.5 transition-colors",
                "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
                "hover:bg-black/10",
              )}
            >
              <Flame aria-hidden className="size-5" />
            </button>
          )}
        </div>
      </div>

      {heatingSheet}

      {phone ? (
        <Drawer
          open={open === "lights"}
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
          <DrawerContent className="data-[swipe-direction=down]:rounded-t-none">
            {contents}
          </DrawerContent>
        </Drawer>
      ) : (
        <Dialog
          open={open === "lights"}
          onOpenChange={(next: boolean) => {
            if (!next) setOpen(null);
          }}
        >
          <DialogContent>{contents}</DialogContent>
        </Dialog>
      )}
    </>
  );
}

/**
 * A temperature, set the way a gauge sets one.
 *
 * The whole degrees carry it and the tenth is a footnote — small enough
 * to read as one, with the degree sign stacked above it so the pair
 * occupies a single column rather than trailing off the end. Which is
 * also how it stays legible at the size a phone gives two cards to a
 * row: the number you actually read is as large as the space allows,
 * and the part you rarely read is not competing with it.
 */
function Reading({ celsius }: { celsius: number }) {
  const [whole, tenth] = celsius.toFixed(1).split(".");
  return (
    <span className="font-heading flex items-start text-4xl leading-none font-semibold tabular-nums sm:text-5xl">
      {whole}
      <span className="ml-0.5 flex flex-col items-center leading-none">
        <span className="text-xl sm:text-2xl">°</span>
        <span className="text-base sm:text-lg">{tenth}</span>
      </span>
    </span>
  );
}
