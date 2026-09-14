import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { SecretField } from "@/components/SecretField";
import { TadoCard } from "@/components/TadoCard";
import { LinearCard } from "@/components/LinearCard";
import { cn } from "@/lib/utils";
import type {
  Integration,
  Secret,
  SecretsReport,
  TadoStatus,
} from "@/lib/api";

export interface IntegrationsPageProps {
  integrations: Integration[];
  secrets?: SecretsReport;
  tado?: TadoStatus;
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
 * Only what is set up gets a card; the rest lives behind the add
 * button. That is the difference between this and the list of text
 * boxes it replaces — you could type any name and any URL there and get
 * something that would never work, because whether Niles can talk to a
 * service is a question about the code, not about the config. The list
 * of what is possible comes from the server for that reason, and it
 * carries the endpoint too, so nobody has to look one up.
 *
 * A hundred entries later this is still one card per thing you use.
 */
export function IntegrationsPage({
  integrations,
  secrets,
  tado,
  linear,
  saving,
  error,
  onChange,
  onSecretsChanged,
  onTadoChanged,
}: IntegrationsPageProps) {
  const [adding, setAdding] = useState(false);

  const added = integrations.filter((i) => i.added);
  const available = integrations.filter((i) => !i.added);

  function add(integration: Integration) {
    setAdding(false);
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
            value: [
              ...integrations
                .filter((i) => i.added && i.kind === "provider")
                .map((i) => ({
                  name: i.id,
                  base_url: i.base_url,
                  serves: i.serves,
                })),
              {
                name: integration.id,
                base_url: integration.base_url,
                serves: integration.serves,
              },
            ],
          },
        ]);
    }
  }

  function remove(integration: Integration) {
    switch (integration.id) {
      case "tado":
        return onChange("presence", [{ path: "presence.enabled", value: false }]);
      default:
        return onChange("providers", [
          {
            path: "providers",
            value: integrations
              .filter(
                (i) => i.added && i.kind === "provider" && i.id !== integration.id,
              )
              .map((i) => ({
                name: i.id,
                base_url: i.base_url,
                serves: i.serves,
              })),
          },
        ]);
    }
  }

  function secretFor(key: string | null): Secret | undefined {
    if (!key) return undefined;
    return secrets?.secrets.find((s) => s.key === key);
  }

  return (
    <div className="flex flex-col gap-4">
      {added.length === 0 && !adding && (
        <Card>
          <CardHeader>
            <CardTitle>Nothing connected yet</CardTitle>
            <CardDescription>
              Niles works on its own — lights, the curve, the dashboard. What
              it connects to is what it can do beyond the house: transcribe
              speech, answer a question, know who is home.
            </CardDescription>
          </CardHeader>
        </Card>
      )}

      {added.map((integration) => {
        if (integration.id === "tado") {
          return (
            tado && (
              <TadoCard
                key="tado"
                status={tado}
                saving={saving}
                onToggle={(on) =>
                  onChange("presence", [{ path: "presence.enabled", value: on }])
                }
                onChanged={onTadoChanged}
                onRemove={() => remove(integration)}
              />
            )
          );
        }
        if (integration.id === "linear") {
          return (
            <LinearCard
              key="linear"
              team={linear?.team ?? ""}
              triggerLabel={linear?.trigger_label ?? ""}
              secret={secretFor(integration.secret_key)}
              writable={secrets?.writable ?? false}
              saving={saving}
              onChange={(entries) => onChange("integrations.linear", entries)}
              onSecretsChanged={onSecretsChanged}
            />
          );
        }
        return (
          <Card key={integration.id}>
            <CardHeader>
              <CardTitle>{integration.label}</CardTitle>
              <CardDescription>
                {integration.blurb}
                <span className="mt-1 block font-mono text-xs">
                  {integration.base_url}
                </span>
              </CardDescription>
              <CardAction>
                <Button
                  variant="ghost"
                  aria-label={`Remove ${integration.label}`}
                  disabled={saving}
                  onClick={() => remove(integration)}
                >
                  <Trash2 aria-hidden />
                </Button>
              </CardAction>
            </CardHeader>
            <CardContent className="flex flex-col gap-3">
              {secretFor(integration.secret_key) ? (
                <SecretField
                  secret={secretFor(integration.secret_key)!}
                  writable={secrets?.writable ?? false}
                  onChanged={onSecretsChanged}
                />
              ) : (
                <p className="text-muted-foreground text-sm">
                  Loading its key…
                </p>
              )}
              <p className="text-muted-foreground text-xs">
                Pick it for a job under Speech &amp; language. One account can
                do both.
              </p>
            </CardContent>
          </Card>
        );
      })}

      {adding ? (
        <Card>
          <CardHeader>
            <CardTitle>Add an integration</CardTitle>
            <CardDescription>
              Everything Niles has been taught to talk to. Adding one puts a
              card above with whatever it needs from you.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
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
                className={cn(
                  "hover:bg-muted/60 focus-visible:ring-3 focus-visible:ring-ring/50 rounded-lg px-3 py-2 text-left focus-visible:outline-none",
                )}
              >
                <span className="block text-sm font-medium">
                  {integration.label}
                </span>
                <span className="text-muted-foreground block text-xs">
                  {integration.blurb}
                </span>
              </button>
            ))}
            <div>
              <Button variant="ghost" onClick={() => setAdding(false)}>
                Cancel
              </Button>
            </div>
          </CardContent>
        </Card>
      ) : (
        <div>
          <Button variant="outline" onClick={() => setAdding(true)}>
            <Plus aria-hidden /> Add integration
          </Button>
        </div>
      )}

      {error && <p className="text-destructive text-sm">{error}</p>}
    </div>
  );
}
