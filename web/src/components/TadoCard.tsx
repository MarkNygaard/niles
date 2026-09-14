import { useEffect, useState } from "react";
import { Check, ExternalLink, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import type { TadoStatus } from "@/lib/api";

export interface TadoCardProps {
  status: TadoStatus;
  /** Start or stop reading presence. Only offered once connected. */
  onToggle: (on: boolean) => void;
  saving?: boolean;
  onChanged: () => void;
  /** Disconnect it entirely, from the integrations page. */
  onRemove?: () => void;
}

/**
 * Connecting tado, which cannot be done from a config file.
 *
 * tado dropped password logins in March 2025; what replaced it needs a
 * person to approve a code in a browser. So the code appears here,
 * where there is a person and a browser.
 *
 * Connecting comes first and switching on comes second, because that
 * is the order in which they make sense: authorising something nothing
 * is using yet is harmless, while turning presence on before anything
 * is connected is a feature that cannot work. Asked once — after that
 * Niles keeps the session alive on its own.
 */
export function TadoCard({
  status,
  onToggle,
  saving,
  onChanged,
  onRemove,
}: TadoCardProps) {
  const [pending, setPending] = useState(status.pending);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => setPending(status.pending), [status.pending]);

  // While a code is out, watch for it being approved. The server is
  // polling tado; this only watches for the answer.
  useEffect(() => {
    if (!pending || status.authorised) return;
    const timer = setInterval(async () => {
      try {
        const next = await api.tadoStatus();
        if (next.authorised) {
          setPending(undefined);
          onChanged();
        }
      } catch {
        // A missed poll is not worth reporting; the next is three
        // seconds away.
      }
    }, 3000);
    return () => clearInterval(timer);
  }, [pending, status.authorised, onChanged]);

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
        {/* The header's own slot rather than a flex row of my own:
            CardHeader is a grid, and adding `flex` to it left the
            button stranded in the middle of the card. */}
        {onRemove && (
          <CardAction>
            <Button
              variant="ghost"
              aria-label="Remove tado"
              disabled={saving}
              onClick={onRemove}
            >
              <Trash2 aria-hidden />
            </Button>
          </CardAction>
        )}
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {!status.connectable && (
          <p className="text-muted-foreground text-sm">
            Niles has no database configured, and tado's session has to be kept
            somewhere it survives a restart. Set <code>[database]</code> and
            this becomes available.
          </p>
        )}

        {status.connectable && !status.authorised && !pending && (
          <>
            <p className="text-muted-foreground text-sm">
              Approve Niles once, in a browser. There is no tado password to
              give it — there is no longer one to give.
            </p>
            <Button onClick={connect} disabled={busy} className="self-start">
              {busy ? "Asking tado…" : "Connect tado"}
            </Button>
          </>
        )}

        {status.connectable && !status.authorised && pending && (
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

        {status.authorised && (
          <>
            <p className="text-muted-foreground flex items-center gap-2 text-sm">
              <Check aria-hidden className="text-lit size-4" />
              Connected. Niles keeps the session alive on its own.
            </p>
            <label className="flex items-center justify-between gap-4">
              <span className="min-w-0">
                <span className="block text-sm font-medium">Read presence</span>
                <span className="text-muted-foreground block text-xs">
                  Polls tado every few minutes for who is home. Takes effect
                  straight away.
                </span>
              </span>
              <Switch
                checked={status.presence_enabled}
                disabled={saving}
                onCheckedChange={onToggle}
              />
            </label>
          </>
        )}

        {error && <p className="text-destructive text-sm">{error}</p>}
      </CardContent>
    </Card>
  );
}
