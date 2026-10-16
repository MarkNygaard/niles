import { BellRing, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { usePush } from "@/hooks/usePush";
import type { PushDelivery, PushDevice } from "@/lib/api";
import type { PushSupport } from "@/lib/push";
import { deniedHelp } from "@/lib/push";
import { relativeTime } from "@/lib/skills";

export interface NotificationsCardProps {
  support: PushSupport;
  /** Where to turn them back on, for a device that said no. */
  denied: string;
  /** Why the server cannot send, when it cannot. */
  unavailable?: string;
  on: boolean;
  /** This device's endpoint, marked in the list. */
  here?: string | null;
  /** Undefined while they are fetched, or when the server cannot send. */
  devices?: PushDevice[];
  /** The switch can ask at once: the key is here. */
  canAsk: boolean;
  busy?: boolean;
  error?: string;
  tested?: PushDelivery;
  onChange: (on: boolean) => void;
  onRemove: (endpoint: string) => void;
  onTest: () => void;
}

/** What a test did, in a sentence. */
export function testedLine(d: PushDelivery): string {
  if (d.sent === 0 && d.failed === 0) {
    return d.gone > 0
      ? "None of your devices could be reached any more, so they were removed."
      : "There is no device to send to yet.";
  }
  const sent = d.sent === 1 ? "Sent to 1 device." : `Sent to ${d.sent} devices.`;
  const failed =
    d.failed === 0 ? "" : d.failed === 1 ? " 1 didn't take it." : ` ${d.failed} didn't take it.`;
  return `${sent}${failed}`;
}

function deviceLine(device: PushDevice, now: Date): string {
  const added = `Added ${relativeTime(device.created_at, now)}`;
  return device.last_ok_at ? `${added} · last sent ${relativeTime(device.last_ok_at, now)}` : added;
}

/**
 * Whether Niles may tell you things on your devices, and on which.
 *
 * On by default in the only sense that matters: nothing to set but the
 * browser's own question, and every kind of notification comes once
 * it is answered yes. The card is per device because the browser's
 * answer is.
 */
export function NotificationsCard({
  support,
  denied,
  unavailable,
  on,
  here,
  devices,
  canAsk,
  busy,
  error,
  tested,
  onChange,
  onRemove,
  onTest,
}: NotificationsCardProps) {
  const now = new Date();
  return (
    <Card>
      <CardHeader>
        <CardTitle>Notifications</CardTitle>
        <CardDescription>
          Niles tells you here when something it was writing for you is ready.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {unavailable ? (
          <p className="text-muted-foreground text-sm">{unavailable}</p>
        ) : (
          <>
            <ThisDevice
              support={support}
              denied={denied}
              on={on}
              canAsk={canAsk}
              busy={busy}
              onChange={onChange}
            />
            {devices && devices.length > 0 && (
              <ul className="flex flex-col gap-2 border-t pt-3" aria-label="Your devices">
                {devices.map((device) => (
                  <li key={device.endpoint} className="flex items-center justify-between gap-3">
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium">
                        {device.device}
                        {device.endpoint === here && (
                          <span className="text-muted-foreground font-normal"> · this device</span>
                        )}
                      </span>
                      <span className="text-muted-foreground block text-xs">
                        {deviceLine(device, now)}
                      </span>
                    </span>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Remove ${device.device}`}
                      disabled={busy}
                      onClick={() => onRemove(device.endpoint)}
                    >
                      <Trash2 aria-hidden />
                    </Button>
                  </li>
                ))}
              </ul>
            )}
            {devices && devices.length > 0 && (
              <div className="flex flex-wrap items-center gap-3">
                <Button size="sm" variant="outline" disabled={busy} onClick={onTest}>
                  <BellRing aria-hidden />
                  Send a test
                </Button>
                {tested && (
                  <span className="text-muted-foreground text-xs" role="status">
                    {testedLine(tested)}
                  </span>
                )}
              </div>
            )}
          </>
        )}
        {error && <p className="text-destructive text-sm">{error}</p>}
      </CardContent>
    </Card>
  );
}

function ThisDevice({
  support,
  denied,
  on,
  canAsk,
  busy,
  onChange,
}: Pick<NotificationsCardProps, "support" | "denied" | "on" | "canAsk" | "busy" | "onChange">) {
  switch (support) {
    case "needs-home-screen":
      return (
        <div className="flex flex-col gap-1 text-sm">
          <p>On an iPhone or iPad, only Niles on the Home Screen can show notifications.</p>
          <p className="text-muted-foreground">
            In Safari, tap Share, then Add to Home Screen. Open Niles from there and turn them
            on here. It needs iOS 16.4 or later.
          </p>
        </div>
      );
    case "unsupported":
      return (
        <p className="text-muted-foreground text-sm">
          This browser can't show notifications from Niles.
        </p>
      );
    case "denied":
      return <p className="text-muted-foreground text-sm">{denied}</p>;
    case "available":
      return (
        <label className="flex items-center justify-between gap-4">
          <span className="min-w-0">
            <span className="block text-sm font-medium">On this device</span>
            <span className="text-muted-foreground block text-xs">
              {on
                ? "You'll be told here when a document is ready."
                : "Your browser will ask whether Niles may send notifications."}
            </span>
          </span>
          <Switch
            checked={on}
            disabled={busy || (!on && !canAsk)}
            onCheckedChange={(next) => onChange(next)}
          />
        </label>
      );
  }
}

/** The card, with this device and the person's others. */
export function Notifications() {
  const push = usePush();
  return (
    <NotificationsCard
      support={push.support}
      denied={deniedHelp(push.env)}
      unavailable={push.unavailable}
      on={push.on}
      here={push.here}
      devices={push.devices}
      canAsk={push.canAsk}
      busy={push.busy}
      error={push.error}
      tested={push.tested}
      onChange={(next) => (next ? push.enable() : push.disable())}
      onRemove={push.remove}
      onTest={push.test}
    />
  );
}
