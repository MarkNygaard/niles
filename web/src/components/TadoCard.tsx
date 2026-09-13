import { useEffect, useState } from "react";
import { Check, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { api } from "@/lib/api";
import type { TadoStatus } from "@/lib/api";

export interface TadoCardProps {
  status: TadoStatus;
  /** Turn `[presence]` on. A restart is needed before it takes. */
  onEnable: () => void;
  enabling?: boolean;
  onConnected: () => void;
}

/**
 * Connecting tado, which cannot be done from a config file.
 *
 * tado dropped password logins in March 2025; what replaced it needs a
 * person to approve a code in a browser. So the code comes here, where
 * there is a person and a browser, rather than to a log line somebody
 * would have to go looking for.
 *
 * Asked once. After that Niles refreshes the session on its own, and
 * this card is a green tick.
 */
export function TadoCard({
  status,
  onEnable,
  enabling,
  onConnected,
}: TadoCardProps) {
  const [pending, setPending] = useState(status.pending);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => setPending(status.pending), [status.pending]);

  // While a code is out, ask whether it has been approved. The server
  // is polling tado anyway; this is only watching for the answer.
  useEffect(() => {
    if (!pending || status.authorised) return;
    const timer = setInterval(async () => {
      try {
        const next = await api.tadoStatus();
        if (next.authorised) {
          setPending(undefined);
          onConnected();
        }
      } catch {
        // A missed poll is not worth saying anything about; the next
        // one is three seconds away.
      }
    }, 3000);
    return () => clearInterval(timer);
  }, [pending, status.authorised, onConnected]);

  async function connect() {
    setError(null);
    setBusy(true);
    try {
      setPending(await api.tadoConnect());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>tado</CardTitle>
        <CardDescription>
          Who is home, from tado's geofencing. Niles only reads it.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {!status.configured && (
          <>
            <p className="text-muted-foreground text-sm">
              Presence is off. Turning it on stores the setting now, but Niles
              only reads this section at startup — so it takes effect on the
              next restart.
            </p>
            <Button onClick={onEnable} disabled={enabling} className="self-start">
              {enabling ? "Turning on…" : "Turn presence on"}
            </Button>
          </>
        )}

        {status.configured && status.authorised && (
          <p className="text-muted-foreground flex items-center gap-2 text-sm">
            <Check aria-hidden className="text-lit size-4" />
            Connected. Niles keeps the session alive on its own.
          </p>
        )}

        {status.configured && !status.authorised && !pending && (
          <>
            <p className="text-muted-foreground text-sm">
              tado needs approving once, in a browser. Niles never sees your
              tado password — there is no longer one to give it.
            </p>
            <Button onClick={connect} disabled={busy} className="self-start">
              {busy ? "Asking tado…" : "Connect tado"}
            </Button>
          </>
        )}

        {status.configured && !status.authorised && pending && (
          <>
            <p className="text-muted-foreground text-sm">
              Open the link and check the code matches. This page notices when
              you are done.
            </p>
            <div className="bg-muted/40 rounded-lg px-4 py-3">
              <div className="text-muted-foreground mb-1 text-xs">Code</div>
              <div className="font-mono text-2xl tracking-widest tabular-nums">
                {pending.user_code}
              </div>
            </div>
            <Button
              render={
                <a
                  href={pending.verification_uri}
                  target="_blank"
                  rel="noreferrer noopener"
                />
              }
              className="gap-2 self-start"
            >
              <ExternalLink aria-hidden /> Approve at tado
            </Button>
          </>
        )}

        {error && <p className="text-destructive text-sm">{error}</p>}
      </CardContent>
    </Card>
  );
}
