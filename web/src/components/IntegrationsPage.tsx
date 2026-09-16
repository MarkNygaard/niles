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
import { LinearPanel } from "@/components/LinearPanel";
import { cn } from "@/lib/utils";
import type {
  Integration,
  Secret,
  SecretsReport,
  TadoStatus,
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
  saving?: boolean;
  error?: string;
  /** Config writes, batched as one revision per action. */
  onChange: (row: string, entries: { path: string; value: unknown }[]) => void;
  onSecretsChanged: () => void;
  onTadoChanged: () => void;
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
 * work.
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
  saving,
  error,
  onChange,
  onSecretsChanged,
  onTadoChanged,
}: IntegrationsPageProps) {
  const [adding, setAdding] = useState(false);
  const [open, setOpen] = useState<string | null>(null);

  const added = integrations.filter((i) => i.added);
  const available = integrations.filter((i) => !i.added);
  const current = integrations.find((i) => i.id === open);

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
    if (integration.id === "tado") return Boolean(tado?.authorised);
    if (integration.id === "linear") return Boolean(linear?.team?.trim());
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

  function remove(integration: Integration) {
    setOpen(null);
    switch (integration.id) {
      case "tado":
        return onChange("presence", [
          { path: "presence.enabled", value: false },
        ]);
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
            What Niles can reach beyond the house: who transcribes speech, who
            answers a question, who knows whether anyone is home.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {added.length === 0 ? (
            <p className="text-muted-foreground text-sm">
              Nothing yet. Niles runs the house without any of these — the
              lights, the curve, the dashboard — so what goes here is what it
              can do beyond it.
            </p>
          ) : (
            <div className="divide-border/60 -mx-1 divide-y">
              {added.map((integration) => (
                <IntegrationRow
                  key={integration.id}
                  integration={integration}
                  ready={ready(integration)}
                  disabled={saving}
                  onOpen={() => setOpen(integration.id)}
                />
              ))}
            </div>
          )}

          {adding ? (
            <div className="flex flex-col gap-1 border-t pt-3">
              <p className="text-muted-foreground mb-1 text-xs">
                Everything Niles has been taught to talk to.
              </p>
              {available.length === 0 && (
                <p className="text-muted-foreground text-sm">
                  All of them are already set up.
                </p>
              )}
              {available.map((integration) => (
                <button
                  key={integration.id}
                  type="button"
                  disabled={saving}
                  onClick={() => add(integration)}
                  className="hover:bg-muted/60 focus-visible:ring-3 focus-visible:ring-ring/50 flex items-center gap-3 rounded-lg px-1 py-2 text-left focus-visible:outline-none"
                >
                  <BrandMark
                    id={integration.id}
                    label={integration.label}
                    className="text-muted-foreground"
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">
                      {integration.label}
                    </span>
                    <span className="text-muted-foreground block text-xs">
                      {integration.blurb}
                    </span>
                  </span>
                  <span className="text-muted-foreground text-xs">Connect</span>
                </button>
              ))}
              <div>
                <Button variant="ghost" onClick={() => setAdding(false)}>
                  Cancel
                </Button>
              </div>
            </div>
          ) : (
            <div>
              <Button variant="outline" onClick={() => setAdding(true)}>
                <Plus aria-hidden /> Add integration
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      {error && <p className="text-destructive text-sm">{error}</p>}

      <Dialog
        open={current !== undefined}
        onOpenChange={(next: boolean) => {
          if (!next) setOpen(null);
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

                {current.kind === "provider" && (
                  <>
                    <p className="text-muted-foreground text-sm">
                      {current.blurb} Pick it for a job under Speech &amp;
                      language — one account can do both.
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

                <div className="border-t pt-3">
                  <Button
                    variant="ghost"
                    className="text-muted-foreground"
                    disabled={saving}
                    onClick={() => remove(current)}
                  >
                    <Trash2 aria-hidden /> Remove {current.label}
                  </Button>
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
  onOpen: () => void;
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
      <Button
        variant="outline"
        size="sm"
        disabled={disabled}
        aria-label={`Configure ${integration.label}`}
        onClick={onOpen}
      >
        Configure
      </Button>
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
