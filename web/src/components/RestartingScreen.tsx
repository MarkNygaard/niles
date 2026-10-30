import { useEffect, useState } from "react";
import { api } from "@/lib/api";
import { logHint, useRestarting, waitForNewBoot } from "@/lib/restart";
import { Button } from "@/components/ui/button";

/** The full-screen answer to a restart: wait for a new Niles, reload,
    and after two minutes say it has not come back and where to look. */
export function RestartingScreen({
  onBack = () => window.location.reload(),
  wait = waitForNewBoot,
}: {
  onBack?: () => void;
  wait?: typeof waitForNewBoot;
}) {
  const waiting = useRestarting();
  const [gaveUp, setGaveUp] = useState(false);
  const [round, setRound] = useState(0);

  useEffect(() => {
    if (!waiting) return;
    const controller = new AbortController();
    void wait(waiting.before, {
      status: () => api.restartStatus(),
      signal: controller.signal,
    }).then((outcome) => {
      if (outcome === "back") onBack();
      if (outcome === "gave_up") setGaveUp(true);
    });
    return () => controller.abort();
  }, [waiting, round, wait, onBack]);

  if (!waiting) return null;
  const hint = logHint(waiting.supervisor);
  return (
    <div
      role="status"
      aria-live="polite"
      className="bg-background/95 fixed inset-0 z-50 flex flex-col items-center justify-center gap-3 p-6 text-center"
    >
      {!gaveUp ? (
        <>
          <h1 className="text-lg font-semibold">Restarting…</h1>
          <p className="text-muted-foreground text-sm">
            Niles will be back in about half a minute.
          </p>
        </>
      ) : (
        <>
          <h1 className="text-lg font-semibold">Niles hasn&apos;t come back after two minutes.</h1>
          <p className="text-muted-foreground text-sm">
            {hint.text}
            {hint.command && <code className="ml-1">{hint.command}</code>}
          </p>
          <div className="flex gap-2">
            <Button
              variant="outline"
              onClick={() => {
                setGaveUp(false);
                setRound((r) => r + 1);
              }}
            >
              Keep waiting
            </Button>
            <Button onClick={onBack}>Reload</Button>
          </div>
        </>
      )}
    </div>
  );
}
