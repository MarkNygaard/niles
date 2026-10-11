import { SecretField } from "@/components/SecretField";
import { Switch } from "@/components/ui/switch";
import type { Secret } from "@/lib/api";

export interface MicrosoftPanelProps {
  enabled: boolean;
  clientId?: Secret;
  clientSecret?: Secret;
  writable: boolean;
  saving?: boolean;
  onEnabled: (enabled: boolean) => void;
  onSecretsChanged: () => void;
}

/** What the app registration must name as its redirect: this server,
    as the browser reaches it. */
export function redirectUri(origin = window.location.origin): string {
  return `${origin}/calendars/microsoft/callback`;
}

/**
 * The household's app registration in Microsoft Entra.
 *
 * Only the keys live here. Each person connects their own calendars,
 * from their profile, and chooses there whether Niles may write to
 * them.
 */
export function MicrosoftPanel({
  enabled,
  clientId,
  clientSecret,
  writable,
  saving,
  onEnabled,
  onSecretsChanged,
}: MicrosoftPanelProps) {
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        “What's on my calendar tomorrow?” — each person's own Outlook.com and work
        calendars, for whoever is asking. Everybody connects theirs from their profile.
      </p>

      <label className="flex items-center justify-between gap-4">
        <span className="min-w-0">
          <span className="block text-sm font-medium">Use Microsoft calendars</span>
          <span className="text-muted-foreground block text-xs">
            Off keeps the keys and everyone's connections.
          </span>
        </span>
        <Switch checked={enabled} disabled={saving} onCheckedChange={(next) => onEnabled(next)} />
      </label>

      {clientId && (
        <SecretField secret={clientId} writable={writable} onChanged={onSecretsChanged} />
      )}
      {clientSecret && (
        <SecretField secret={clientSecret} writable={writable} onChanged={onSecretsChanged} />
      )}

      <ol className="text-muted-foreground list-decimal space-y-1 pl-4 text-xs">
        <li>
          In entra.microsoft.com, App registrations → New registration. Account types:
          any organizational directory and personal Microsoft accounts.
        </li>
        <li>
          Redirect URI, platform Web:{" "}
          <span className="font-mono break-all select-all">{redirectUri()}</span>
        </li>
        <li>
          Certificates & secrets → New client secret. Copy the Application (client) ID and
          the secret's value here.
        </li>
      </ol>
    </div>
  );
}
