import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { MqttPanel } from "@/components/MqttPanel";
import type { MqttSettings } from "@/components/MqttPanel";
import { SetupChecklist } from "@/components/SetupChecklist";
import type { Secret, SetupGap } from "@/lib/api";

export interface DevicesPageProps {
  /** `[mqtt]` as the config has it, and Niles's own values for it. */
  mqtt: MqttSettings;
  defaults: MqttSettings;
  username?: Secret;
  password?: Secret;
  writable: boolean;
  /** How many devices Niles knows, once it has asked. */
  devices?: number;
  /** The setup gaps this page answers. */
  gaps: SetupGap[];
  saving?: boolean;
  error?: string;
  onChange: (entries: { path: string; value: unknown }[]) => void;
  onSecretsChanged: () => void;
}

/**
 * System → Devices: the broker every light, plug and sensor is reached
 * through.
 *
 * Its own page because it is not an add-on: without it Niles has no
 * lights at all, and a new install should not have to find it in a
 * list of optional extras.
 */
export function DevicesPage({
  mqtt,
  defaults,
  username,
  password,
  writable,
  devices,
  gaps,
  saving,
  error,
  onChange,
  onSecretsChanged,
}: DevicesPageProps) {
  return (
    <div className="flex flex-col gap-4">
      <SetupChecklist gaps={gaps} />
      <Card>
        <CardHeader>
          <CardTitle>Zigbee &amp; MQTT</CardTitle>
          <CardDescription>
            <BrokerStatus host={mqtt.host} devices={devices} />
          </CardDescription>
        </CardHeader>
        <CardContent>
          <MqttPanel
            mqtt={mqtt}
            defaults={defaults}
            username={username}
            password={password}
            writable={writable}
            saving={saving}
            onChange={onChange}
            onSecretsChanged={onSecretsChanged}
          />
          {error && <p className="text-destructive mt-3 text-sm">{error}</p>}
        </CardContent>
      </Card>
    </div>
  );
}

/**
 * How the broker is doing, as far as the app can tell.
 *
 * Niles does not report the connection itself, so this says what it
 * can see: whether there is a broker at all, and how many devices have
 * come through it. None after a restart is the sign something is wrong.
 */
export function BrokerStatus({ host, devices }: { host?: string; devices?: number }) {
  if (!host?.trim()) return <>No broker yet. Niles has no lights until it has one.</>;
  if (devices === undefined) return <>Asking Niles what it has found…</>;
  if (devices === 0) {
    return (
      <>
        Set to {host}, and nothing has come through it yet. If Niles has
        restarted since it was set, check the address and the login.
      </>
    );
  }
  return (
    <>
      Set to {host}. Niles knows {devices} {devices === 1 ? "device" : "devices"}.
    </>
  );
}
