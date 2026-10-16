import { useState } from "react";
import { BellRing, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { usePush } from "@/hooks/usePush";
import { deniedHelp, dismissNudge, nudgeDismissed, shouldNudge } from "@/lib/push";

/**
 * "Notify me when it's done", under a document being written.
 *
 * The moment a person is most likely to want a notification is the one
 * where Niles has just said it will take a few minutes, so the offer is
 * made there, once. Dismissed, it is not made on this device again:
 * My profile still has the switch.
 */
export function PushNudge() {
  const push = usePush();
  const [dismissed, setDismissed] = useState(nudgeDismissed);
  const [asked, setAsked] = useState(false);

  if (asked && push.on) {
    return (
      <p className="text-muted-foreground self-start px-1 text-xs" role="status">
        You'll get a notification on this device when it's ready.
      </p>
    );
  }
  if (asked && push.support === "denied") {
    return <p className="text-muted-foreground self-start px-1 text-xs">{deniedHelp(push.env)}</p>;
  }
  if (!shouldNudge({ ...push, dismissed })) return null;
  return (
    <div className="flex items-center gap-1 self-start px-1">
      <Button
        size="sm"
        variant="outline"
        disabled={push.busy || !push.canAsk}
        onClick={() => {
          setAsked(true);
          push.enable();
        }}
      >
        <BellRing aria-hidden />
        Notify me when it's done
      </Button>
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label="Don't offer notifications"
        className="text-muted-foreground"
        onClick={() => {
          dismissNudge();
          setDismissed(true);
        }}
      >
        <X aria-hidden />
      </Button>
      {push.error && <span className="text-destructive text-xs">{push.error}</span>}
    </div>
  );
}
