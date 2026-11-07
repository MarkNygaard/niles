import { useState } from "react";
import { LogIn } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyField } from "@/components/CopyField";
import { api } from "@/lib/api";
import type { PluginInbound as Inbound, PluginSummary } from "@/lib/api";
import { connectHref, lastHookLine } from "@/lib/plugins";

const NO_PUBLIC_ADDRESS = (
  <p className="text-muted-foreground">
    Set Niles's public address under Settings → API access first.
  </p>
);

/**
 * Connecting an account and the webhook, on a plugin's panel
 * (interface 0.3): the addresses to register with the service, Connect,
 * and a new webhook address.
 */
export function PluginInboundSection({
  plugin,
  inbound,
  outcome,
  onChanged,
}: {
  plugin: PluginSummary;
  inbound: Inbound;
  /** What the way back from a sign-in page said, when it just did. */
  outcome?: { ok: boolean; text: string } | null;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  return (
    <div className="flex flex-col gap-4">
      {inbound.callback && (
        <section aria-label="Account" className="flex flex-col gap-2">
          <h3 className="text-sm font-medium">Account</h3>
          {outcome && (
            <p role="status" className={outcome.ok ? "text-ok" : "text-destructive"}>
              {outcome.text}
            </p>
          )}
          {inbound.callback_url ? (
            <>
              <p className="text-muted-foreground">
                Register this as the callback address in {plugin.name}'s developer settings.
              </p>
              <CopyField label="Callback address" value={inbound.callback_url} />
              <div>
                {/* A navigation, not a request: it ends on the service's
                    sign-in page and comes back here. */}
                <Button
                  size="sm"
                  disabled={!plugin.enabled}
                  render={<a href={connectHref(inbound.callback_url)} />}
                >
                  <LogIn aria-hidden /> Connect account
                </Button>
              </div>
            </>
          ) : (
            NO_PUBLIC_ADDRESS
          )}
        </section>
      )}

      {inbound.webhook && (
        <section aria-label="Webhook" className="flex flex-col gap-2">
          <h3 className="text-sm font-medium">Webhook</h3>
          {inbound.hook_url ? (
            <>
              <p className="text-muted-foreground">
                The add-on gives this address to {plugin.name} itself. Only{" "}
                <code className="text-xs">/plugins/*/hook/*</code> has to be reachable from the
                internet.
              </p>
              <CopyField label="Webhook address" value={inbound.hook_url} />
              <p className="text-muted-foreground">{lastHookLine(inbound.last_hook)}</p>
              {confirming ? (
                <div className="flex flex-wrap items-center gap-2">
                  <span>The old address stops working, and the add-on restarts.</span>
                  <Button
                    size="sm"
                    variant="destructive"
                    disabled={busy}
                    onClick={async () => {
                      setBusy(true);
                      setError(null);
                      try {
                        await api.rotatePluginHook(plugin.id);
                        setConfirming(false);
                        onChanged();
                      } catch (e) {
                        setError(e instanceof Error ? e.message : String(e));
                      } finally {
                        setBusy(false);
                      }
                    }}
                  >
                    New address
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>
                    Keep it
                  </Button>
                </div>
              ) : (
                <div>
                  <Button size="sm" variant="outline" onClick={() => setConfirming(true)}>
                    New address…
                  </Button>
                </div>
              )}
            </>
          ) : (
            NO_PUBLIC_ADDRESS
          )}
        </section>
      )}

      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
    </div>
  );
}
