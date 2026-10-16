import { useCallback, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { X } from "lucide-react";
import { Popover as PopoverPrimitive } from "@base-ui/react/popover";
import {
  Drawer,
  DrawerClose,
  DrawerContent,
  DrawerDescription,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { cn } from "@/lib/utils";

/** One tab of the panel. A new tab is one more of these in the list. */
export interface PanelTab {
  id: string;
  /** Read out for the icon, and the tooltip over it. */
  label: string;
  icon: LucideIcon;
  render: () => React.ReactNode;
}

export interface NilesPanelProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The avatar it opens from: where it grows from, and where focus goes back to. */
  anchor: React.RefObject<HTMLButtonElement | null>;
  /** One line under the name. */
  status: string;
  tabs: PanelTab[];
}

/**
 * Niles's face: the app's own icon, in a circle.
 *
 * The icon rather than a drawing of its own, so the face in the chat
 * is the one on the home screen.
 */
export function NilesAvatar({ className }: { className?: string }) {
  return (
    <img
      src="/icon-192.png"
      alt=""
      draggable={false}
      className={cn("pointer-events-none shrink-0 rounded-full select-none", className)}
    />
  );
}

/** Ease-out that runs a touch past the end and settles back: a spring,
    without a library to simulate one. The desktop panel uses the same. */
const SPRING = "cubic-bezier(0.34, 1.3, 0.64, 1)";

/**
 * Flies a copy of the chat's avatar up into the panel's header while the
 * panel rises under it, and lands it where the header's own avatar will
 * be once the panel is all the way up.
 *
 * A copy, fixed to the screen, rather than the header's avatar itself:
 * that one rides on the panel, and a transform on it would be fighting
 * the panel's own for the whole of the flight. The landing spot is
 * worked out from where the avatar sits *within* the panel, which the
 * panel's slide does not change, plus where the panel will end up —
 * flush with the bottom of the screen.
 */
function fly(from: HTMLElement | null, to: HTMLElement): (() => void) | undefined {
  const panel = to.closest<HTMLElement>("[data-slot=drawer-popup]");
  if (!from || !panel || typeof to.animate !== "function") return;
  const start = from.getBoundingClientRect();
  const end = to.getBoundingClientRect();
  const box = panel.getBoundingClientRect();
  if (!end.width || !start.width) return;
  const top = window.innerHeight - panel.offsetHeight + (end.top - box.top);
  const left = end.left - box.left;

  const copy = to.cloneNode(true) as HTMLElement;
  Object.assign(copy.style, {
    position: "fixed",
    top: `${top}px`,
    left: `${left}px`,
    width: `${end.width}px`,
    height: `${end.height}px`,
    margin: "0",
    zIndex: "60",
    transformOrigin: "0 0",
    pointerEvents: "none",
  });
  copy.setAttribute("aria-hidden", "true");
  copy.dataset.slot = "avatar-flight";
  document.body.append(copy);
  // Both ends hidden while the copy is between them, or there are two
  // avatars on the screen for a third of a second.
  from.style.visibility = "hidden";
  to.style.visibility = "hidden";

  const flight = copy.animate(
    [
      {
        transform: `translate(${start.left - left}px, ${start.top - top}px) scale(${start.width / end.width})`,
      },
      { transform: "none" },
    ],
    { duration: 340, easing: SPRING },
  );
  const land = () => {
    copy.remove();
    from.style.visibility = "";
    to.style.visibility = "";
  };
  flight.onfinish = land;
  flight.oncancel = land;
  return () => flight.cancel();
}

/**
 * Niles's own panel, opened from the avatar at the top of the chat: who
 * Niles is, how it is doing, and tabs of what it keeps — the
 * conversations first, with more to come.
 *
 * Full screen in a hand, risen from the bottom like any sheet there and
 * dismissed the same way, by swiping it down. On a desk, a tall panel
 * that grows out of the avatar rather than a dialog in the middle of the
 * screen: it belongs to the thing that was clicked.
 */
export function NilesPanel({ open, onOpenChange, anchor, status, tabs }: NilesPanelProps) {
  const phone = useMediaQuery("(max-width: 639px)");
  const still = useMediaQuery("(prefers-reduced-motion: reduce)");
  const [tab, setTab] = useState(tabs[0]?.id);
  const active = tabs.find((t) => t.id === tab) ?? tabs[0];
  // The panel itself takes focus on opening, not its first button: the
  // first button is the X, and a ring round it after a tap reads as
  // something being wrong with it. Keyboard users land at the top of the
  // panel and tab on from there.
  const top = useRef<HTMLDivElement>(null);

  // Stable, or React would call it again on every render — and fly the
  // avatar in again every time the status line changed.
  const land = useCallback(
    (el: HTMLSpanElement | null) => {
      if (!el || !phone || still) return;
      return fly(anchor.current, el);
    },
    [anchor, phone, still],
  );

  const Title = phone ? DrawerTitle : PopoverPrimitive.Title;
  const Description = phone ? DrawerDescription : PopoverPrimitive.Description;
  const Close = phone ? DrawerClose : PopoverPrimitive.Close;

  const inside = (
    <div ref={top} tabIndex={-1} className="flex min-h-0 flex-1 flex-col outline-none">
      <div className="relative flex shrink-0 flex-col items-center gap-0.5 px-12 pt-4 pb-3">
        <Close
          aria-label="Close"
          className={cn(
            "text-muted-foreground hover:text-foreground hover:bg-muted absolute top-2 left-2 flex size-9 items-center justify-center rounded-full transition-colors",
            "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          )}
        >
          <X aria-hidden className="size-5" />
        </Close>
        <span ref={land} className={cn("mb-1.5 inline-flex", phone ? "size-20" : "size-16")}>
          <NilesAvatar className="size-full" />
        </span>
        <Title className="font-wordmark text-foreground text-2xl leading-tight font-medium tracking-wide">
          Niles
        </Title>
        <Description
          role="status"
          className="text-muted-foreground max-w-full truncate text-sm"
        >
          {status}
        </Description>
      </div>

      {tabs.length > 1 ? (
        <Tabs
          value={active?.id}
          onValueChange={(value) => setTab(String(value))}
          className="min-h-0 flex-1 gap-0"
        >
          <div className="shrink-0 px-4 pb-2">
            <TabsList className="h-10 w-full rounded-full p-1">
              {tabs.map((t) => (
                <TabsTrigger
                  key={t.id}
                  value={t.id}
                  aria-label={t.label}
                  title={t.label}
                  className="h-full rounded-full [&_svg:not([class*='size-'])]:size-[18px]"
                >
                  <t.icon aria-hidden />
                </TabsTrigger>
              ))}
            </TabsList>
          </div>
          {tabs.map((t) => (
            <TabsContent
              key={t.id}
              value={t.id}
              className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-2"
            >
              {t.render()}
            </TabsContent>
          ))}
        </Tabs>
      ) : (
        active && (
          <>
            {/* One tab is a heading, not a bar: a segmented control with
                one segment offers a choice there is not, and a screen
                reader reads it out as "tab, 1 of 1". The bar comes with
                the second tab. */}
            <h3 className="text-muted-foreground flex shrink-0 items-center gap-1.5 px-4 pt-1 pb-2 text-xs font-medium tracking-wide uppercase">
              <active.icon aria-hidden className="size-3.5" />
              {active.label}
            </h3>
            <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-2">
              {active.render()}
            </div>
          </>
        )
      )}
    </div>
  );

  if (phone) {
    return (
      <Drawer open={open} onOpenChange={onOpenChange}>
        <DrawerContent
          initialFocus={top}
          finalFocus={anchor}
          className={cn(
            // The whole screen, square at the top, and clear of the
            // notch: a drawer is positioned against the viewport, so the
            // body's own safe-area padding never reaches it.
            "h-dvh max-h-dvh pt-[env(safe-area-inset-top)] data-[swipe-direction=down]:rounded-t-none data-[swipe-direction=down]:border-t-0",
            // Quicker than the component's 450 ms: it is the whole
            // screen moving, and that much screen feels slow at the
            // same speed as a small sheet.
            "duration-[320ms]",
            // Nothing slides when movement is turned down: the closed
            // panel sits where the open one does, and fades.
            still &&
              "data-[swipe-direction=down]:[--closed-transform:translate3d(0,0,0)] data-starting-style:opacity-0 data-ending-style:opacity-0",
          )}
        >
          {inside}
        </DrawerContent>
      </Drawer>
    );
  }

  return (
    // Modal, so focus stays in it and a click outside closes it rather
    // than landing on the chat underneath.
    <PopoverPrimitive.Root open={open} onOpenChange={onOpenChange} modal>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Positioner
          anchor={anchor}
          side="bottom"
          align="center"
          sideOffset={8}
          collisionPadding={16}
          className="z-50"
        >
          <PopoverPrimitive.Popup
            data-slot="niles-panel"
            initialFocus={top}
            finalFocus={anchor}
            className={cn(
              "bg-popover text-popover-foreground ring-foreground/10 flex flex-col overflow-hidden rounded-xl shadow-xl ring-1 outline-none",
              "h-[min(80vh,44rem,var(--available-height))] w-[min(28rem,calc(100vw-2rem))]",
              // Grows out of the avatar: Base UI puts the origin where the
              // anchor is. Springs open, and goes quickly and plainly.
              "origin-(--transform-origin) transition-[transform,opacity] duration-300 ease-[cubic-bezier(0.34,1.3,0.64,1)]",
              "data-ending-style:duration-150 data-ending-style:ease-in",
              still
                ? "data-starting-style:opacity-0 data-ending-style:opacity-0"
                : "data-starting-style:scale-[0.4] data-starting-style:opacity-0 data-ending-style:scale-[0.6] data-ending-style:opacity-0",
            )}
          >
            {inside}
          </PopoverPrimitive.Popup>
        </PopoverPrimitive.Positioner>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
}
