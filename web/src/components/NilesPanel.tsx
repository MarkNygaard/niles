import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { X } from "lucide-react";
import {
  Drawer,
  DrawerClose,
  DrawerContent,
  DrawerDescription,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { badgeText } from "@/lib/approvals";
import { cn } from "@/lib/utils";

/** One tab of the panel. A new tab is one more of these in the list. */
export interface PanelTab {
  id: string;
  /** Read out for the icon, and the tooltip over it. */
  label: string;
  icon: LucideIcon;
  /** How many things there wait for you, shown on the icon while any do. */
  badge?: number;
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
  /** The tab shown, when whoever opens the panel decides: a card in
      the chat opens it on Saved. Without it the panel keeps its own. */
  tab?: string;
  onTabChange?: (id: string) => void;
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
  // Scrolled away up a long conversation, the avatar would fly in from
  // above the screen; the header's own then simply rides up with the
  // panel.
  if (!end.width || !start.width || start.bottom < 0 || start.top > window.innerHeight) return;
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
  // Landed at once if the panel goes mid-flight, rather than when the
  // cancel event gets round to it: the avatar should be back in the
  // chat before the panel has finished going.
  return () => {
    flight.cancel();
    land();
  };
}

/**
 * Niles's own panel, opened from the avatar at the top of the chat: who
 * Niles is, how it is doing, and tabs of what it keeps — the
 * conversations first, with more to come.
 *
 * Full screen in a hand, risen from the bottom like any sheet there and
 * dismissed the same way, by swiping it down. On a desk, the right half
 * of the screen: it slides in while the page moves over into the left
 * half, and the chat there stays live, so a plan or a trip can be read
 * on one side while Niles is asked about it on the other.
 */
export function NilesPanel({
  open,
  onOpenChange,
  anchor,
  status,
  tabs,
  tab: chosenTab,
  onTabChange,
}: NilesPanelProps) {
  // A half of a narrower screen is too little for either side.
  const phone = useMediaQuery("(max-width: 767px)");
  const still = useMediaQuery("(prefers-reduced-motion: reduce)");
  const [ownTab, setOwnTab] = useState(tabs[0]?.id);
  const tab = chosenTab ?? ownTab;
  const setTab = (id: string) => {
    setOwnTab(id);
    onTabChange?.(id);
  };
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

  const titleId = useId();
  const Title = phone ? DrawerTitle : "h2";
  const Description = phone ? DrawerDescription : "p";
  const closeClass = cn(
    "text-muted-foreground hover:text-foreground hover:bg-muted absolute top-2 left-2 flex size-9 items-center justify-center rounded-full transition-colors",
    "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
  );

  const inside = (
    <div ref={top} tabIndex={-1} className="flex min-h-0 flex-1 flex-col outline-none">
      <div className="relative flex shrink-0 flex-col items-center gap-0.5 px-12 pt-4 pb-3">
        {phone ? (
          <DrawerClose aria-label="Close" className={closeClass}>
            <X aria-hidden className="size-5" />
          </DrawerClose>
        ) : (
          <button
            type="button"
            aria-label="Close"
            onClick={() => onOpenChange(false)}
            className={closeClass}
          >
            <X aria-hidden className="size-5" />
          </button>
        )}
        <span ref={land} className={cn("mb-1.5 inline-flex", phone ? "size-20" : "size-16")}>
          <NilesAvatar className="size-full" />
        </span>
        <Title id={phone ? undefined : titleId} className="font-wordmark text-foreground text-2xl leading-tight font-medium tracking-wide">
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
          className="min-h-0 flex-1 flex-col gap-0"
        >
          <div className="shrink-0 px-4 pb-2">
            <TabsList className="h-10 w-full rounded-full p-1">
              {tabs.map((t) => (
                <TabsTrigger
                  key={t.id}
                  value={t.id}
                  aria-label={t.badge ? `${t.label}, ${t.badge} waiting` : t.label}
                  title={t.label}
                  className="relative h-full rounded-full [&_svg:not([class*='size-'])]:size-[18px]"
                >
                  <t.icon aria-hidden />
                  {/* On the icon's corner, as a home screen counts what
                      waits: the bar has no room for a word. */}
                  {Boolean(t.badge) && (
                    <span
                      aria-hidden
                      data-slot="tab-badge"
                      className="bg-primary text-primary-foreground absolute top-0 left-1/2 ml-1.5 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] leading-none font-medium tabular-nums"
                    >
                      {badgeText(t.badge ?? 0)}
                    </span>
                  )}
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
    <SidePane
      open={open}
      onOpenChange={onOpenChange}
      anchor={anchor}
      top={top}
      still={still}
      titleId={titleId}
    >
      {inside}
    </SidePane>
  );
}

/** The page's share of the screen while the pane is open, set on the
    root so the page and the composer pinned over it can both follow. */
const SPLIT = "panel";

/**
 * The pane on a desk: fixed to the right half of the screen, with the
 * page moved over into the left half for as long as it is open (see
 * `[data-panel]` in globals.css).
 *
 * Not modal: the chat beside it is the point. It is there or not, and
 * while it slides out it keeps what it showed, then lets it go, so an
 * open page's frame is not kept running off screen.
 */
function SidePane({
  open,
  onOpenChange,
  anchor,
  top,
  still,
  titleId,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  anchor: React.RefObject<HTMLButtonElement | null>;
  top: React.RefObject<HTMLDivElement | null>;
  still: boolean;
  titleId: string;
  children: React.ReactNode;
}) {
  const pane = useRef<HTMLElement>(null);
  const [kept, setKept] = useState(open);
  if (open && !kept) setKept(true);

  useEffect(() => {
    if (!open) return;
    const root = document.documentElement;
    root.dataset[SPLIT] = "";
    top.current?.focus({ preventScroll: true });
    return () => {
      delete root.dataset[SPLIT];
      // Back to the avatar only from inside the pane: closing it from the
      // avatar, or while typing in the chat, leaves focus where it is.
      if (pane.current?.contains(document.activeElement)) anchor.current?.focus();
    };
  }, [open, top, anchor]);

  return (
    <aside
      ref={pane}
      role="dialog"
      aria-modal={false}
      aria-labelledby={titleId}
      aria-hidden={!open}
      inert={!open}
      data-slot="niles-panel"
      data-open={open ? "" : undefined}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onOpenChange(false);
        }
      }}
      onTransitionEnd={(e) => {
        if (e.target === e.currentTarget && !open) setKept(false);
      }}
      className={cn(
        "bg-background text-foreground border-border fixed inset-y-0 right-0 z-40 flex w-1/2 flex-col border-l shadow-xl outline-none",
        "pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)]",
        open ? "translate-x-0" : "translate-x-full",
        // The same speed and curve as the page making room for it.
        still ? "transition-none" : "transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)]",
      )}
    >
      {(open || kept) && children}
    </aside>
  );
}
