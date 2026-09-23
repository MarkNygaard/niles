import { CommitField } from "@/components/CommitField";
import { SecretField } from "@/components/SecretField";
import type { Secret } from "@/lib/api";

export interface UnifiPanelProps {
  host: string;
  secret?: Secret;
  writable: boolean;
  saving?: boolean;
  onHost: (host: string) => void;
  onSecretsChanged: () => void;
}

/**
 * The UniFi console: where it is, and the key it wants.
 *
 * Both are read on every poll, so nothing here needs a restart — the
 * pairing button on the dashboard appears as soon as the console
 * answers.
 */
export function UnifiPanel({
  host,
  secret,
  writable,
  saving,
  onHost,
  onSecretsChanged,
}: UnifiPanelProps) {
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        Niles asks the console which phones are on the Wi-Fi. Pair a phone
        to a person from the dashboard, on that phone, while it is at home.
      </p>

      <CommitField
        label="Console address"
        value={host}
        placeholder="192.168.1.1"
        disabled={saving}
        onCommit={onHost}
      />

      {secret && (
        <SecretField
          secret={secret}
          writable={writable}
          onChanged={onSecretsChanged}
        />
      )}

      <p className="text-muted-foreground text-xs">
        Make a key in UniFi Network under Settings → Control Plane →
        Integrations.
      </p>
    </div>
  );
}
