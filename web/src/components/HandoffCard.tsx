import { Hand } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { Handoff } from "@/lib/api";

/**
 * Niles handed the person its browser in this turn: a CAPTCHA, or a
 * step only a person can do. While the browser is still theirs for it,
 * the card opens the live view; once given back, it says so.
 */
export function HandoffCard({
  handoff,
  waiting,
  onOpen,
}: {
  handoff: Handoff;
  /** The browser is still the person's for this handoff. */
  waiting: boolean;
  onOpen?: () => void;
}) {
  return (
    <section
      aria-label="Niles needs you in the browser"
      data-handoff={handoff.id}
      className="bg-muted/60 flex w-full max-w-xl flex-col gap-3 self-start rounded-2xl p-4 ring-1 ring-foreground/10"
    >
      <div className="flex items-start gap-3">
        <Hand aria-hidden className="mt-0.5 size-5 shrink-0" />
        <div className="min-w-0">
          <h3 className="text-base leading-snug font-medium">Niles needs you in the browser</h3>
          <p className="text-muted-foreground font-mono text-sm break-all">{handoff.site}</p>
        </div>
      </div>
      <p className="text-sm">{handoff.why}</p>
      {waiting ? (
        <div>
          <Button size="sm" onClick={onOpen}>
            Open the browser
          </Button>
        </div>
      ) : (
        <p className="text-muted-foreground text-sm">The browser went back to Niles.</p>
      )}
    </section>
  );
}
