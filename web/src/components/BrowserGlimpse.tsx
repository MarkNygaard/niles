import { Globe } from "lucide-react";
import { useLivePicture } from "@/hooks/useLivePicture";
import type { LiveView } from "@/lib/api";
import { siteOf } from "@/lib/browser";

/**
 * Niles's browser, small and live, in the bubble of the turn being
 * waited for: the person sees Niles at work without opening anything,
 * and a tap opens the Browser view to watch it big or take it over.
 */
export function BrowserGlimpse({ live, onOpen }: { live: LiveView; onOpen?: () => void }) {
  const { picture } = useLivePicture(true);
  const site = siteOf(live.address);
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
export function BrowserStrip({ live, onOpen }: { live: LiveView; onOpen?: () => void }) {
  const site = siteOf(live.address);
  return (
    <div className="mx-auto w-full max-w-5xl px-4 pt-2 sm:px-6">
      <button
        type="button"
        onClick={onOpen}
        className="bg-muted/60 text-muted-foreground hover:text-foreground flex w-full items-center gap-2 rounded-full px-3 py-1.5 text-left text-xs ring-1 ring-foreground/10 transition-colors"
      >
        <Globe aria-hidden className="size-3.5 shrink-0" />
        <span className="truncate">
          {live.control === "person" ? "You have Niles's browser" : "Niles's browser is open"} ·{" "}
          {site}
        </span>
      </button>
    </div>
  );
}
