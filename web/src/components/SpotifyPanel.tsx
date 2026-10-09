import { SecretField } from "@/components/SecretField";
import { Switch } from "@/components/ui/switch";
import type { Secret } from "@/lib/api";

export interface SpotifyPanelProps {
  enabled: boolean;
  clientId?: Secret;
  clientSecret?: Secret;
  writable: boolean;
  saving?: boolean;
  onEnabled: (enabled: boolean) => void;
  onSecretsChanged: () => void;
}

/**
 * Spotify: the keys of a developer app, for finding what to play.
 *
 * Sonos plays it, through the Spotify account linked in the Sonos app;
 * these keys only search the catalogue, so they need nobody's login.
 */
export function SpotifyPanel({
  enabled,
  clientId,
  clientSecret,
  writable,
  saving,
  onEnabled,
  onSecretsChanged,
}: SpotifyPanelProps) {
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        “Play John Mayer”, “play Gravity by John Mayer”, “play the album
        Continuum” — found on Spotify and played on your Sonos, through the
        Spotify account linked in the Sonos app.
      </p>

      <label className="flex items-center justify-between gap-4">
        <span className="min-w-0">
          <span className="block text-sm font-medium">Use Spotify</span>
          <span className="text-muted-foreground block text-xs">
            Off keeps the keys; favorites and radio still play.
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
          Sign in at developer.spotify.com/dashboard with the Premium account,
          and choose Create app.
        </li>
        <li>
          Any name and description will do. Spotify asks for a redirect URI:
          use <span className="font-mono">http://127.0.0.1:8888/callback</span>{" "}
          — Niles never signs in, so it is never used.
        </li>
        <li>Tick Web API, save, and copy the Client ID and Client secret here.</li>
      </ol>
    </div>
  );
}
