import { Globe, PanelRightClose } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useLivePicture } from "@/hooks/useLivePicture";
import type { LiveView } from "@/lib/api";
import { siteOf } from "@/lib/browser";

/**
 * Niles's browser, small and live, in the bubble of the turn being
 * waited for: the person sees Niles at work without opening anything,
 * and a tap opens the Browser view to watch it big or take it over.
 */
export function BrowserGlimpse({
  live,
  beside,
  onOpen,
  onClose,
}: {
  live: LiveView;
  /** The Browser tab shows the page beside the chat: this one steps
      aside, streaming nothing, rather than show it twice. */
  beside?: boolean;
  onOpen?: () => void;
  onClose?: () => void;
}) {
  const { picture } = useLivePicture(!beside);
  const site = siteOf(live.address);
  if (beside) {
    return (
      <div
        role="group"
        aria-label={`The browser on ${site} is open beside the chat`}
        className="bg-muted text-muted-foreground flex aspect-[1280/900] w-full max-w-sm flex-col items-center justify-center gap-2 rounded-md p-4 text-center text-sm"
      >
        <span>Open beside the chat</span>
        <Button variant="outline" size="sm" onClick={onClose}>
          <PanelRightClose aria-hidden />
          Close
        </Button>
      </div>
    );
  }
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label={`Niles is on ${site}. Open the browser`}
      className="flex w-full max-w-sm flex-col gap-1.5 text-left"
    >
      <span className="border-border block aspect-[1280/900] w-full overflow-hidden rounded-md border bg-white">
        {picture ? (
          <img
            src={picture}
            alt={`What ${site} shows`}
            draggable={false}
            className="h-full w-full object-contain"
          />
        ) : (
          <span className="bg-muted block h-full w-full motion-safe:animate-pulse" />
        )}
      </span>
      <span className="text-muted-foreground flex items-center gap-1.5 text-xs">
        <Globe aria-hidden className="size-3.5 shrink-0" />
        <span className="truncate">
          {live.control === "person" ? "You have the browser on" : "Niles is on"} {site}
        </span>
      </span>
    </button>
  );
}

/**
 * A strip above the message field while Niles's browser stays open, so
 * the way back to it is never only behind the avatar.
 */
export function BrowserStrip({
  live,
  beside,
  onOpen,
  onClose,
}: {
  live: LiveView;
  /** The Browser tab shows the page beside the chat. */
  beside?: boolean;
  onOpen?: () => void;
  onClose?: () => void;
}) {
  const site = siteOf(live.address);
  return (
    <div className="mx-auto w-full max-w-5xl px-4 pt-2 sm:px-6">
      <button
        type="button"
        onClick={beside ? onClose : onOpen}
        className="bg-muted/60 text-muted-foreground hover:text-foreground flex w-full items-center gap-2 rounded-full px-3 py-1.5 text-left text-xs ring-1 ring-foreground/10 transition-colors"
      >
        <Globe aria-hidden className="size-3.5 shrink-0" />
        <span className="truncate">
          {beside
            ? "Showing beside the chat"
            : live.control === "person"
              ? "You have Niles's browser"
              : "Niles's browser is open"}{" "}
          · {site}
        </span>
        {beside && <span className="text-foreground ml-auto shrink-0">Close</span>}
      </button>
    </div>
  );
}
