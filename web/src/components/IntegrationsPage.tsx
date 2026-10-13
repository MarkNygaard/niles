import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { BrandMark } from "@/components/BrandMark";
import type { DeviceOption } from "@/components/DevicePicker";
import { SecretField } from "@/components/SecretField";
import { TadoPanel } from "@/components/TadoPanel";
import { ClaudeCodePanel } from "@/components/ClaudeCodePanel";
import { NemligPanel } from "@/components/NemligPanel";
import { LinearPanel } from "@/components/LinearPanel";
import { SonosPanel } from "@/components/SonosPanel";
import { CalendarAppPanel, redirectUri } from "@/components/CalendarAppPanel";
import { SpotifyPanel } from "@/components/SpotifyPanel";
import { TvPanel } from "@/components/TvPanel";
import { UnifiPanel } from "@/components/UnifiPanel";
import { IntegrationGallery, groupsOf } from "@/components/IntegrationGallery";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import type {
  Integration,
  Secret,
  SecretsReport,
  SpeakersReport,
  TadoStatus,
  TvInfo,
  Zone,
} from "@/lib/api";

export interface IntegrationsPageProps {
  integrations: Integration[];
  secrets?: SecretsReport;
  tado?: TadoStatus;
  /** tado's heating zones, and the rooms they can be paired with. */
  zones?: Zone[];
  rooms?: string[];
  /** What presence does to the lights, and the lights to choose from. */
  presenceLights?: { offWhenAway: boolean; onWhenHome: string[] };
  lightOptions?: DeviceOption[];
  /** The Linear section as the config has it, if there is one. */
  linear?: { team?: string; trigger_label?: string };
  /** The Claude Code section as the config has it, if there is one. */
  claudeCode?: { enabled?: boolean; model?: string };
  /** The nemlig.com section as the config has it, if there is one. */
  nemlig?: { enabled?: boolean };
  /** The Spotify section as the config has it, if there is one. */
  spotify?: { enabled?: boolean };
  /** The Microsoft section as the config has it, if there is one. */
  microsoft?: { enabled?: boolean };
  /** The Google section as the config has it, if there is one. */
  google?: { enabled?: boolean };
  /** The UniFi console's address, empty until one is typed in. */
  unifiHost?: string;
  /** `[speakers]` as the config has it: Sonos's switch and address. */
  sonos?: { enabled?: boolean; host?: string };
  /** What the speaker at that address described. */
  sonosFound?: SpeakersReport;
  /** `[tv]` as the config has it. */
  tvConfig?: { host?: string; room?: string; show_announcements?: boolean };
  tv?: TvInfo;
  onTvChanged?: () => void;
  saving?: boolean;
  error?: string;
  /** Config writes, batched as one revision per action. */
  onChange: (row: string, entries: { path: string; value: unknown }[]) => void;
  /** Drop what was set, returning those paths to Niles's defaults. */
  onReset: (row: string, paths: string[]) => void;
  onSecretsChanged: () => void;
  onTadoChanged: () => void;
  /** Go to a Settings section, for what is set up there rather than
      here: the broker. */
  onOpenSettings?: (section: string) => void;
}

/**
 * What Niles is connected to.
 *
 * A list of services rather than a stack of cards. An endpoint, a key
 * and a model are all setup detail — true, and worth nothing at a
 * glance. What this page is for is seeing which things are connected
 * and getting into the one you came to change, and both of those fit
 * on a row.
 *
 * Only what is set up is listed; the rest is behind the add button.
 * That is the difference from the pair of text boxes this replaces,
 * where any name and any URL bought you a provider that would never
 * work. Both are under the same groups, so a thing is found in the same
 * place whether or not it is set up yet.
 */
export function IntegrationsPage({
  integrations,
  secrets,
  tado,
  zones,
  rooms,
  presenceLights,
  lightOptions,
  linear,
  claudeCode,
  nemlig,
  spotify,
  microsoft,
  google,
  unifiHost,
  sonos,
  sonosFound,
  tvConfig,
  tv,
  onTvChanged,
  saving,
  error,
  onChange,
  onReset,
  onSecretsChanged,
  onTadoChanged,
  onOpenSettings,
}: IntegrationsPageProps) {
  const [adding, setAdding] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);

  const added = integrations.filter((i) => i.added);
  const current = integrations.find((i) => i.id === open);

  function openSettings(section: string) {
    setAdding(false);
    onOpenSettings?.(section);
  }

  function secretFor(key: string | null): Secret | undefined {
    if (!key) return undefined;
    return secrets?.secrets.find((s) => s.key === key);
  }

  /**
   * Whether it is finished, not merely added.
   *
   * Worth its own dot: a provider with no key is a row that looks done
   * and answers nothing.
   */
  function ready(integration: Integration): boolean {
    // Set up on its own page, which says what is missing there.
    if (integration.kind === "setting") return integration.added;
    if (integration.id === "tado") return Boolean(tado?.authorised);
    if (integration.id === "linear") return Boolean(linear?.team?.trim());
    if (integration.id === "nemlig")
      return (
        nemlig?.enabled !== false &&
        secretFor("integrations.nemlig.username")?.source !== "unset" &&
        secretFor("integrations.nemlig.password")?.source !== "unset"
      );
    if (integration.id === "claude_code")
      return (
        claudeCode?.enabled !== false &&
        secretFor(integration.secret_key)?.source !== "unset"
      );
    if (integration.id === "spotify")
      return (
        spotify?.enabled !== false &&
        secretFor("integrations.spotify.client_id")?.source !== "unset" &&
        secretFor("integrations.spotify.client_secret")?.source !== "unset"
      );
    if (integration.id === "microsoft")
      return (
        microsoft?.enabled !== false &&
        secretFor("integrations.microsoft.client_id")?.source !== "unset" &&
        secretFor("integrations.microsoft.client_secret")?.source !== "unset"
      );
    if (integration.id === "google")
      return (
        google?.enabled !== false &&
        secretFor("integrations.google.client_id")?.source !== "unset" &&
        secretFor("integrations.google.client_secret")?.source !== "unset"
      );
    if (integration.id === "lg_tv") return Boolean(tv?.paired);
    if (integration.id === "sonos")
      return sonos?.enabled !== false && Boolean(sonos?.host?.trim());
    if (integration.id === "unifi")
      return (
        Boolean(unifiHost?.trim()) &&
        secretFor(integration.secret_key)?.source !== "unset"
      );
    return secretFor(integration.secret_key)?.source !== "unset";
  }

  function add(integration: Integration) {
    setAdding(false);
    // Opened straight away: adding one is never the whole job, and a
    // row that appears with a grey dot and no prompt is something you
    // have to work out you are not finished with.
    setOpen(integration.id);
    switch (integration.id) {
      case "tado":
        return onChange("presence", [
          { path: "presence.enabled", value: true },
          { path: "presence.tado", value: {} },
        ]);
      case "linear":
        return onChange("integrations.linear", [
          { path: "integrations.linear", value: { team: "" } },
        ]);
      case "claude_code":
        return onChange("integrations.claude_code", [
          { path: "integrations.claude_code.enabled", value: true },
        ]);
      case "nemlig":
        return onChange("integrations.nemlig", [
          { path: "integrations.nemlig.enabled", value: true },
        ]);
      case "spotify":
        return onChange("integrations.spotify", [
          { path: "integrations.spotify.enabled", value: true },
        ]);
      // The redirect as this browser reaches Niles, written down rather
      // than worked out from forwarded headers on every sign-in:
      // Microsoft compares it letter for letter.
      case "microsoft":
        return onChange("integrations.microsoft", [
          { path: "integrations.microsoft.enabled", value: true },
          { path: "integrations.microsoft.redirect_uri", value: redirectUri("microsoft") },
        ]);
      case "google":
        return onChange("integrations.google", [
          { path: "integrations.google.enabled", value: true },
          { path: "integrations.google.redirect_uri", value: redirectUri("google") },
        ]);
      case "unifi":
      case "sonos":
      case "lg_tv":
        // Nothing to write yet: the address is what makes it added, and
        // that is the first thing the dialog asks for.
        return;
      default:
        // A provider is its endpoint and the roles it serves, both of
        // which Niles already knows. Appending rather than replacing:
        // adding one must not drop the others.
        return onChange("providers", [
          {
            path: "providers",
            value: [...providerRows(integrations), asRow(integration)],
          },
        ]);
    }
  }

  /** The keys it keeps that were typed in here, as against ones an
      environment variable provides, which are not this page's to delete. */
  function storedKeys(integration: Integration): string[] {
    return integration.secret_keys.filter(
      (key) => secretFor(key)?.source === "stored",
    );
  }

  async function remove(integration: Integration) {
    // Its keys go with it. A key shows only on its integration's card,
    // so one left behind is one nobody can see or delete. Cleared
    // before the config changes, because the server accepts a
    // provider's key only while that provider is declared.
    const keys = storedKeys(integration);
    if (keys.length > 0) {
      setRemoveError(null);
      setRemoving(true);
      try {
        for (const key of keys) await api.clearSecret(key);
      } catch (e) {
        // Stopped rather than carried on: removing it anyway would
        // strand whatever is left with nothing to show it.
        setRemoveError(e instanceof Error ? e.message : String(e));
        return;
      } finally {
        setRemoving(false);
        onSecretsChanged();
      }
    }
    setOpen(null);
    switch (integration.id) {
      case "tado":
        return onChange("presence", [
          { path: "presence.enabled", value: false },
        ]);
      case "unifi":
        return onChange("presence.unifi", [
          { path: "presence.unifi.host", value: "" },
        ]);
      // The pairing key is gone with it: it belongs to one TV, so a TV
      // added again pairs again.
      case "lg_tv":
        return onChange("tv", [{ path: "tv.host", value: "" }]);
      // The rooms each Sonos is placed in stay, for when it comes back.
      case "sonos":
        return onChange("speakers", [{ path: "speakers.host", value: "" }]);
      // Gone, not switched off — the card's own switch is for that, and
      // keeps the token. Removing it deletes the token too.
      case "claude_code":
        return onReset("integrations.claude_code", ["integrations.claude_code"]);
      // Like Claude Code: gone from the config, and the login with it.
      case "nemlig":
        return onReset("integrations.nemlig", ["integrations.nemlig"]);
      case "spotify":
        return onReset("integrations.spotify", ["integrations.spotify"]);
      // The app's keys are deleted; everyone's own connections stay in
      // the database for when it comes back with new ones.
      case "microsoft":
        return onReset("integrations.microsoft", ["integrations.microsoft"]);
      case "google":
        return onReset("integrations.google", ["integrations.google"]);
      default:
        return onChange("providers", [
          {
            path: "providers",
            value: providerRows(integrations).filter(
              (p) => p.name !== integration.id,
            ),
          },
        ]);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Connected services</CardTitle>
          <CardDescription>
            What Niles is connected to: the broker your lights talk through,
            who transcribes speech, who answers a question, who knows whether
            anyone is home.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {added.length === 0 ? (
            <p className="text-muted-foreground text-sm">
              Nothing yet. Start with Add integration: the ones marked
              Required are what Niles needs to reach your lights, hear you and
              answer.
            </p>
          ) : (
            groupsOf(added).map(({ group, items }) => (
              <section
                key={group.id}
                aria-labelledby={`added-group-${group.id}`}
                className="flex flex-col"
              >
                <h3
                  id={`added-group-${group.id}`}
                  className="text-muted-foreground text-xs font-medium tracking-wide uppercase"
                >
                  {group.label}
                </h3>
                <div className="divide-border/60 -mx-1 divide-y">
                  {items.map((integration) => (
                    <IntegrationRow
                      key={integration.id}
                      integration={integration}
                      ready={ready(integration)}
                      disabled={saving}
                      onOpen={
                        integration.kind === "setting"
                          ? integration.settings_page && onOpenSettings
                            ? () => openSettings(integration.settings_page!.id)
                            : undefined
                          : () => setOpen(integration.id)
                      }
                    />
                  ))}
                </div>
              </section>
            ))
          )}

          <div>
            <Button variant="outline" onClick={() => setAdding(true)}>
              <Plus aria-hidden /> Add integration
            </Button>
          </div>
        </CardContent>
      </Card>

      {error && <p className="text-destructive text-sm">{error}</p>}

      <IntegrationGallery
        open={adding}
        onOpenChange={setAdding}
        integrations={integrations}
        disabled={saving}
        onOpenSettings={onOpenSettings ? openSettings : undefined}
        onPick={(integration) => {
          if (integration.added) {
            setAdding(false);
            setOpen(integration.id);
          } else {
            add(integration);
          }
        }}
      />

      <Dialog
        open={current !== undefined}
        onOpenChange={(next: boolean) => {
          if (!next) {
            setOpen(null);
            setRemoveError(null);
          }
        }}
      >
        <DialogContent className="sm:max-w-lg">
          {current && (
            <>
              <DialogHeader>
                <DialogTitle className="flex items-center gap-2">
                  <BrandMark id={current.id} label={current.label} />
                  {current.label}
                  {ready(current) && (
                    <Badge variant="secondary">configured</Badge>
                  )}
                </DialogTitle>
              </DialogHeader>
              <DialogBody className="flex flex-col gap-4">
                {current.id === "tado" && tado && (
                  <TadoPanel
                    status={tado}
                    saving={saving}
                    onToggle={(on) =>
                      onChange("presence", [
                        { path: "presence.enabled", value: on },
                      ])
                    }
                    onChanged={onTadoChanged}
                    lights={presenceLights}
                    lightOptions={lightOptions}
                    onLightsChange={(entries) => onChange("presence", entries)}
                    zones={zones}
                    rooms={rooms}
                    onPair={(zoneId, room) =>
                      onChange("presence", [
                        {
                          path: `presence.tado.rooms.${zoneId}`,
                          value: room,
                        },
                      ])
                    }
                  />
                )}

                {current.id === "linear" && (
                  <LinearPanel
                    team={linear?.team ?? ""}
                    triggerLabel={linear?.trigger_label ?? ""}
                    secret={secretFor(current.secret_key)}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onChange={(entries) =>
                      onChange("integrations.linear", entries)
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "claude_code" && (
                  <ClaudeCodePanel
                    enabled={claudeCode?.enabled !== false}
                    model={claudeCode?.model ?? "sonnet"}
                    secret={secretFor(current.secret_key)}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.claude_code", [
                        { path: "integrations.claude_code.enabled", value: enabled },
                      ])
                    }
                    onModel={(model) =>
                      onChange("integrations.claude_code", [
                        { path: "integrations.claude_code.model", value: model },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "nemlig" && (
                  <NemligPanel
                    enabled={nemlig?.enabled !== false}
                    username={secretFor("integrations.nemlig.username")}
                    password={secretFor("integrations.nemlig.password")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.nemlig", [
                        { path: "integrations.nemlig.enabled", value: enabled },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "spotify" && (
                  <SpotifyPanel
                    enabled={spotify?.enabled !== false}
                    clientId={secretFor("integrations.spotify.client_id")}
                    clientSecret={secretFor("integrations.spotify.client_secret")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.spotify", [
                        { path: "integrations.spotify.enabled", value: enabled },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "microsoft" && (
                  <CalendarAppPanel
                    provider="microsoft"
                    enabled={microsoft?.enabled !== false}
                    clientId={secretFor("integrations.microsoft.client_id")}
                    clientSecret={secretFor("integrations.microsoft.client_secret")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.microsoft", [
                        { path: "integrations.microsoft.enabled", value: enabled },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "google" && (
                  <CalendarAppPanel
                    provider="google"
                    enabled={google?.enabled !== false}
                    clientId={secretFor("integrations.google.client_id")}
                    clientSecret={secretFor("integrations.google.client_secret")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.google", [
                        { path: "integrations.google.enabled", value: enabled },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "lg_tv" && (
                  <TvPanel
                    host={tvConfig?.host ?? ""}
                    room={tvConfig?.room ?? ""}
                    showAnnouncements={tvConfig?.show_announcements !== false}
                    tv={tv}
                    rooms={rooms ?? []}
                    saving={saving}
                    onChange={(entries) => onChange("tv", entries)}
                    onPaired={() => {
                      onSecretsChanged();
                      onTvChanged?.();
                    }}
                  />
                )}

                {current.id === "sonos" && (
                  <SonosPanel
                    enabled={sonos?.enabled !== false}
                    host={sonos?.host ?? ""}
                    found={sonosFound}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("speakers", [{ path: "speakers.enabled", value: enabled }])
                    }
                    onHost={(host) =>
                      onChange("speakers", [{ path: "speakers.host", value: host }])
                    }
                  />
                )}

                {current.id === "unifi" && (
                  <UnifiPanel
                    host={unifiHost ?? ""}
                    secret={secretFor(current.secret_key)}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onHost={(host) =>
                      onChange("presence.unifi", [
                        { path: "presence.unifi.host", value: host },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.kind === "provider" && (
                  <>
                    <p className="text-muted-foreground text-sm">
                      {current.blurb} Choose what it does under Speech &amp;
                      language.
                    </p>
                    {secretFor(current.secret_key) && (
                      <SecretField
                        // The endpoint is named once below. Repeating it
                        // beside the field, inside this provider's own
                        // dialog, says the same thing twice.
                        secret={{
                          ...secretFor(current.secret_key)!,
                          hint: undefined,
                        }}
                        writable={secrets?.writable ?? false}
                        onChanged={onSecretsChanged}
                      />
                    )}
                    <p className="text-muted-foreground font-mono text-xs">
                      {current.base_url}
                    </p>
                  </>
                )}

                <div className="flex flex-col gap-1 border-t pt-3">
                  <div>
                    <Button
                      variant="ghost"
                      className="text-muted-foreground"
                      disabled={saving || removing}
                      onClick={() => remove(current)}
                    >
                      <Trash2 aria-hidden /> Remove {current.label}
                    </Button>
                  </div>
                  {/* Said before the click rather than after: there is
                      nowhere to get a deleted key back from. */}
                  {storedKeys(current).length > 0 && (
                    <p className="text-muted-foreground px-3 text-xs">
                      Its saved keys are deleted too.
                    </p>
                  )}
                  {removeError && (
                    <p className="text-destructive px-3 text-sm">{removeError}</p>
                  )}
                </div>
              </DialogBody>
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** One service, as a row. */
function IntegrationRow({
  integration,
  ready,
  disabled,
  onOpen,
}: {
  integration: Integration;
  ready: boolean;
  disabled?: boolean;
  /** Absent for one set up on a Settings page nobody can take us to. */
  onOpen?: () => void;
}) {
  return (
    <div className="flex items-center gap-3 px-1 py-2.5">
      <span
        aria-hidden
        className={cn(
          "size-2 shrink-0 rounded-full",
          ready ? "bg-lit" : "bg-muted-foreground/40",
        )}
      />
      <BrandMark id={integration.id} label={integration.label} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {integration.label}
        </span>
        {!ready && (
          <span className="text-muted-foreground block text-xs">
            Added, but not finished.
          </span>
        )}
      </span>
      {onOpen ? (
        <Button
          variant="outline"
          size="sm"
          disabled={disabled}
          aria-label={`Configure ${integration.label}`}
          onClick={onOpen}
        >
          Configure
        </Button>
      ) : (
        integration.settings_page && (
          <span className="text-muted-foreground text-xs">
            In Settings → {integration.settings_page.label}
          </span>
        )
      )}
    </div>
  );
}

/** The `[[providers]]` array as it stands, ready to be written back. */
function providerRows(integrations: Integration[]) {
  return integrations
    .filter((i) => i.added && i.kind === "provider")
    .map(asRow);
}

function asRow(integration: Integration) {
  return {
    name: integration.id,
    base_url: integration.base_url,
    serves: integration.serves,
  };
}
