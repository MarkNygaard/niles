import { CommitField } from "@/components/CommitField";
import { SecretField } from "@/components/SecretField";
import type { Secret } from "@/lib/api";

export interface MqttSettings {
  host?: string;
  port?: number;
  z2m_prefix?: string;
  client_id?: string;
}

export interface MqttPanelProps {
  /** `[mqtt]` as the config has it. */
  mqtt: MqttSettings;
  /** Niles's own values, shown where nothing is set. */
  defaults: MqttSettings;
  username?: Secret;
  password?: Secret;
  writable: boolean;
  saving?: boolean;
  onChange: (entries: { path: string; value: unknown }[]) => void;
  onSecretsChanged: () => void;
}

/**
 * The broker every light, plug and sensor is reached through, as
 * System → Devices sets it up.
 *
 * Niles connects once as it starts, so every value here waits for a
 * restart, and the page says so before anybody wonders why nothing
 * moved.
 */
export function MqttPanel({
  mqtt,
  defaults,
  username,
  password,
  writable,
  saving,
  onChange,
  onSecretsChanged,
}: MqttPanelProps) {
  const set = (path: string, value: unknown) => onChange([{ path, value }]);
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        Where Zigbee2MQTT and the WLED strips publish. Without a broker Niles
        has no lights at all. Changes here take effect when Niles restarts.
      </p>

      <div className="flex gap-3">
        <CommitField
          label="Broker address"
          value={mqtt.host ?? ""}
          placeholder="192.168.1.10"
          disabled={saving}
          onCommit={(host) => set("mqtt.host", host)}
        />
        <div className="w-24">
          <CommitField
            label="Port"
            value={mqtt.port === undefined ? "" : String(mqtt.port)}
            placeholder={String(defaults.port ?? "")}
            allowEmpty
            disabled={saving}
            onCommit={(port) =>
              // Empty is Niles's own port; a null clears the setting.
              set("mqtt.port", port === "" ? null : Number(port))
            }
          />
        </div>
      </div>

      <div className="flex gap-3">
        <CommitField
          label="Zigbee2MQTT prefix"
          value={mqtt.z2m_prefix ?? ""}
          placeholder={defaults.z2m_prefix ?? ""}
          allowEmpty
          disabled={saving}
          onCommit={(prefix) => set("mqtt.z2m_prefix", prefix === "" ? null : prefix)}
        />
        <CommitField
          label="Client id"
          value={mqtt.client_id ?? ""}
          placeholder={defaults.client_id ?? ""}
          allowEmpty
          disabled={saving}
          onCommit={(id) => set("mqtt.client_id", id === "" ? null : id)}
        />
      </div>

      {(username || password) && (
        <div className="flex flex-col gap-3">
          <p className="text-muted-foreground text-xs">
            A login, only if the broker asks for one.
          </p>
          {username && (
            <SecretField secret={username} writable={writable} onChanged={onSecretsChanged} />
          )}
          {password && (
            <SecretField secret={password} writable={writable} onChanged={onSecretsChanged} />
          )}
        </div>
      )}
    </div>
  );
}
