import { useEffect, useState } from "react";
import { Check, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { DevicePicker } from "@/components/DevicePicker";
import type { DeviceOption } from "@/components/DevicePicker";
import { ZonePairing, UnplacedNotice } from "@/components/ZonePairing";
import { api } from "@/lib/api";
import type { TadoStatus, Zone } from "@/lib/api";

export interface TadoPanelProps {
  status: TadoStatus;
  /** What presence does to the lights, as `[presence]` has it. */
  lights?: { offWhenAway: boolean; onWhenHome: string[] };
  /** Every light in the house, for choosing the ones to come home to. */
  lightOptions?: DeviceOption[];
  /** Writes to `presence.*`, batched as one revision. */
  onLightsChange?: (entries: { path: string; value: unknown }[]) => void;
  /** Start or stop reading presence. Only offered once connected. */
  onToggle: (on: boolean) => void;
  saving?: boolean;
  onChanged: () => void;
  /** The heating zones tado reports, once it is connected. */
  zones?: Zone[];
  /** Rooms Niles knows about, to pair them with. */
  rooms?: string[];
  onPair?: (zoneId: number, room: string) => void;
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
export function TadoPanel({
  status,
  onToggle,
  saving,
  lights,
  lightOptions,
  onLightsChange,
  onChanged,
  zones,
  rooms,
  onPair,
}: TadoPanelProps) {
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
    <div className="flex flex-col gap-4">
      {!status.connectable && (
        <p className="text-muted-foreground text-sm">
          Niles has no database configured, and tado's session has to be kept
          somewhere it survives a restart. Set <code>[database]</code> and this
          becomes available.
        </p>
      )}

      {status.connectable && !status.authorised && !pending && (
        <>
          <p className="text-muted-foreground text-sm">
            Approve Niles once, in a browser. There is no tado password to give
            it — there is no longer one to give.
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

          {/* What presence is for, beyond answering the question. Both
              are off until somebody says otherwise: switching presence
              on to know whether the house is empty should not also
              hand it the light switches. */}
          {status.presence_enabled && lights && onLightsChange && (
            <div className="flex flex-col gap-4 border-t pt-4">
              <label className="flex items-center justify-between gap-4">
                <span className="min-w-0">
                  <span className="block text-sm font-medium">
                    Lights off when everyone leaves
                  </span>
                  <span className="text-muted-foreground block text-xs">
                    Whatever is on at the moment the last person goes.
                    Anything already off is left alone.
                  </span>
                </span>
                <Switch
                  checked={lights.offWhenAway}
                  disabled={saving}
                  onCheckedChange={(on) =>
                    onLightsChange([
                      { path: "presence.lights_off_when_away", value: on },
                    ])
                  }
                />
              </label>

              <div className="flex flex-col gap-1.5">
                <span className="text-sm font-medium">
                  Lights on when somebody comes home
                </span>
                <span className="text-muted-foreground text-xs">
                  The hall and the kitchen, rather than the whole house —
                  which is why this is a list and the one above is a
                  switch. A light already on is left as it is.
                </span>
                <DevicePicker
                  id="lights-on-when-home"
                  aria-label="Lights on when somebody comes home"
                  value={lights.onWhenHome}
                  options={lightOptions ?? []}
                  disabled={saving}
                  emptyMessage="No lights yet."
                  onChange={(value) =>
                    onLightsChange([
                      { path: "presence.lights_on_when_home", value },
                    ])
                  }
                />
              </div>
            </div>
          )}

          {zones && zones.length > 0 && onPair && (
            <div className="flex flex-col gap-3 border-t pt-4">
              <ZonePairing
                zones={zones}
                rooms={rooms ?? []}
                saving={saving}
                onPair={onPair}
              />
              <UnplacedNotice zones={zones} />
            </div>
          )}
        </>
      )}

      {error && <p className="text-destructive text-sm">{error}</p>}
    </div>
  );
}
