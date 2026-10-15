import { SecretField } from "@/components/SecretField";
import { Switch } from "@/components/ui/switch";
import type { Secret } from "@/lib/api";

export type CalendarApp = "microsoft" | "google";

export interface CalendarAppPanelProps {
  provider: CalendarApp;
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
export function redirectUri(provider: CalendarApp, origin = window.location.origin): string {
  return `${origin}/calendars/${provider}/callback`;
}

const COPY: Record<CalendarApp, { what: string; use: string; steps: React.ReactNode[] }> = {
  microsoft: {
    what: "each person's own Outlook.com and work calendars and mail",
    use: "Use Microsoft calendars",
    steps: [
      <>
        In entra.microsoft.com, App registrations → New registration. Account types: any
        organizational directory and personal Microsoft accounts.
      </>,
      "redirect",
      <>
        Certificates & secrets → New client secret. Copy the Application (client) ID and the
        secret's value here.
      </>,
    ],
  },
  google: {
    what: "each person's own Gmail calendar and mail",
    use: "Use Google calendars",
    steps: [
      <>
        In console.cloud.google.com, make a project and enable the Google Calendar API, and the
        Gmail API for mail.
      </>,
      <>
        Google Auth Platform: name the app, choose External, and publish it — left in testing,
        Google ends every sign-in after a week. Add the scopes calendar.readonly,
        calendar.events, gmail.readonly and gmail.send. Reading Gmail is one of Google's
        restricted scopes: while the app is unverified it may hold it for up to 100 people,
        which a household is well under.
      </>,
      "redirect",
      <>Clients → Create client → Web application. Copy its client ID and secret here.</>,
    ],
  },
};

/**
 * A household's app registration with Microsoft or Google.
 *
 * Only the keys live here. Each person connects their own calendars,
 * from their profile, and chooses there whether Niles may write to
 * them.
 */
export function CalendarAppPanel({
  provider,
  enabled,
  clientId,
  clientSecret,
  writable,
  saving,
  onEnabled,
  onSecretsChanged,
}: CalendarAppPanelProps) {
  const copy = COPY[provider];
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        “What's on my calendar tomorrow?” — {copy.what}, for whoever is asking. Everybody connects
        theirs from their profile.
      </p>

      <label className="flex items-center justify-between gap-4">
        <span className="min-w-0">
          <span className="block text-sm font-medium">{copy.use}</span>
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
        {copy.steps.map((step, i) => (
          <li key={i}>
            {step === "redirect" ? (
              <>
                Redirect URI, as a web client:{" "}
                <span className="font-mono break-all select-all">{redirectUri(provider)}</span>
              </>
            ) : (
              step
            )}
          </li>
        ))}
      </ol>
    </div>
  );
}
