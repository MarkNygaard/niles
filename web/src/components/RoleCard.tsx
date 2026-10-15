import { useEffect, useState } from "react";
import { Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { BrandMark } from "@/components/BrandMark";
import { cn } from "@/lib/utils";
import type { Integration, SecretsReport } from "@/lib/api";

export type Role = "stt" | "llm";

/** What one press of Save asks for. */
export interface RoleChange {
  /** The provider picked; absent keeps a role on its own endpoint. */
  provider?: string;
  model: string;
  effort?: string | null;
  /** A key typed in beside the provider, to save with it. */
  key?: string;
}

export interface RoleCardProps {
  role: Role;
  title: string;
  description: string;
  /** The providers Niles ships, from the catalogue. Only the ones that
      do this job are offered. */
  providers: Integration[];
  /** Which provider this role names, if any. */
  current?: string;
  model: string;
  /** What Niles runs when nothing is written down. Shown in the box as
      a placeholder, because that is what an unset field actually
      means here — not "empty", which is what it used to look like. */
  defaultModel?: string;
  /** The host of the role's own `base_url`, when it has one and names
      no provider — so the page can say what is answering today. */
  fallbackHost?: string;
  /** How hard this role's model should think, when it has been set. */
  effort?: string;
  /** Where each provider's key comes from. */
  secrets?: SecretsReport;
  saving?: boolean;
  onSave: (change: RoleChange) => void;
}

/** The sentinel for "let me type one", which is not a model name. */
const OTHER = " other";

/** The sentinel for "whatever the provider does", which is not a value. */
const PROVIDER_DEFAULT = " default";

/**
 * How hard to think before answering, as the page offers it.
 *
 * Only on the thinking role, because it is the only one it means
 * anything to. `none` is left out deliberately: two models accept it
 * and the rest reject it, and "low" already covers wanting an answer
 * rather than an essay.
 *
 * Unset first, and unset unless somebody chooses. The gpt-oss models
 * think at `medium` unless told otherwise — which for a question asked
 * out loud is tokens generated while somebody stands in a dark hall —
 * but choosing on behalf of a provider nobody asked about is how you
 * get a 400 from a model that does not take the field at all.
 */
export const EFFORTS: { value: string; label: string }[] = [
  { value: PROVIDER_DEFAULT, label: "Provider default" },
  { value: "low", label: "Low — answer, do not deliberate" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High — for the tier that escalates" },
];

/** What the select shows for a configured value. */
export function effortValue(effort?: string): string {
  return effort?.trim() ? effort : PROVIDER_DEFAULT;
}

/** What to save for a chosen option. The sentinel means "unset". */
export function effortToSave(picked: string): string | null {
  return picked === PROVIDER_DEFAULT ? null : picked;
}

/**
 * The models to offer, with whatever is already configured included.
 *
 * A shipped list goes stale the week a provider adds something, and a
 * dropdown that cannot express the value already in the config would
 * silently offer to change it.
 */
export function choices(offered?: string[], current?: string): string[] {
  const list = offered ?? [];
  if (!current || list.includes(current)) return list;
  return [...list, current];
}

/** What each provider offers for this role, by its id. */
export function modelsOf(providers: Integration[], role: Role): Record<string, string[]> {
  return Object.fromEntries(
    providers
      .map((p) => [p.id, p.models?.[role] ?? []] as const)
      .filter(([, list]) => list.length > 0),
  );
}

/**
 * What to put in the model box when the provider changes.
 *
 * A model name is not portable between providers — which is the whole
 * reason this card saves the two together — so carrying the old one
 * across a switch produces exactly the request that fails at somebody
 * else's server. Picking the new provider's first model instead means
 * the pair is always one a provider can actually serve.
 *
 * Empty for a provider nothing is known about, which drops the box
 * back to being typed in rather than filling it with a guess.
 */
export function modelForProvider(
  models: Record<string, string[]> | undefined,
  provider: string,
): string {
  return models?.[provider]?.[0] ?? "";
}

/**
 * The models to show for the provider now selected.
 *
 * `configured` is only honoured while the provider is still the one it
 * was configured against. It exists so a model already in the config
 * stays visible even when the shipped list has gone stale — but held
 * across a provider switch it put one provider's models in another's
 * dropdown, which is the opposite of what this card is for.
 */
export function offeredFor(
  models: Record<string, string[]> | undefined,
  provider: string,
  configuredProvider: string,
  configured: string,
): string[] {
  const known = models?.[provider] ?? [];
  return provider === configuredProvider ? choices(known, configured) : known;
}

/**
 * The providers that can do this job.
 *
 * A function rather than a line inside the component because it is the
 * one rule here worth testing on its own. Speech-to-text is offered only
 * what can hear — Cerebras has no transcription endpoint, and offering
 * it would turn a 404 from somebody else's server into the way you find
 * that out.
 */
export function usableFor(providers: Integration[], role: Role): Integration[] {
  return providers.filter((p) => p.kind === "provider" && p.serves.includes(role));
}

/**
 * The provider shown as chosen when the card opens.
 *
 * The one the role names, when Niles still ships it. A role that names
 * none uses its default provider — ElevenLabs hears, Cerebras thinks —
 * so that is what is shown, ready for its key. A role on an endpoint of
 * its own has picked nothing, and is shown as such.
 */
export function startingProvider(
  usable: Integration[],
  role: Role,
  current: string | undefined,
  ownEndpoint: boolean,
): string {
  if (current && usable.some((p) => p.id === current)) return current;
  if (ownEndpoint && !current) return "";
  return usable.find((p) => p.default_for.includes(role))?.id ?? "";
}

/**
 * One job, and who does it.
 *
 * Picking a provider *is* setting it up: its key sits beside it, and
 * the one Save declares the provider, points the role at it, and keeps
 * the key. There is no separate "add a provider" step to find first.
 *
 * The provider and the model sit together because they change
 * together: a model name is not portable between providers, so picking
 * one without the other produces a request that fails at somebody
 * else's server rather than an error on this page. Saving them as one
 * edit also makes it one revision and one undo.
 *
 * The provider is a row of buttons rather than a dropdown: there are
 * one or two per job, and a list that has to be opened to see the
 * choice is a step for nothing.
 */
export function RoleCard({
  role,
  title,
  description,
  providers,
  current,
  model,
  effort,
  defaultModel,
  fallbackHost,
  secrets,
  saving,
  onSave,
}: RoleCardProps) {
  const usable = usableFor(providers, role);
  const models = modelsOf(usable, role);
  const initialProvider = startingProvider(usable, role, current, Boolean(fallbackHost));
  // An unset model under a provider is that provider's first: shown as
  // chosen, so typing only a key is enough to save.
  const initialModel =
    model || (initialProvider ? modelForProvider(models, initialProvider) : "");

  const [provider, setProvider] = useState(initialProvider);
  const [draft, setDraft] = useState(initialModel);
  const [thinking, setThinking] = useState(effortValue(effort));
  const [key, setKey] = useState("");
  const [typing, setTyping] = useState(false);

  useEffect(() => setProvider(initialProvider), [initialProvider]);
  useEffect(() => setDraft(initialModel), [initialModel]);
  useEffect(() => setThinking(effortValue(effort)), [effort]);
  // A saved key is gone from the box once it is saved.
  useEffect(() => setKey(""), [secrets]);

  const picked = usable.find((p) => p.id === provider);
  const secret = picked?.secret_key
    ? secrets?.secrets.find((s) => s.key === picked.secret_key)
    : undefined;
  const dirty =
    provider !== initialProvider ||
    draft !== initialModel ||
    effortValue(effort) !== thinking ||
    key.trim() !== "";
  const known = models[provider] ?? [];
  // `defaultModel` is deliberately not in here. It is what Niles ships
  // with, which belongs to the provider Niles ships with — offering it
  // under a different one is the leak this replaced.
  const offered = offeredFor(models, provider, initialProvider, draft);
  // A list Niles ships will go stale the week a provider adds
  // something, so typing one stays possible — just not the first thing
  // you are asked to do.
  const pickingModel = known.length > 0 && !typing;

  function pick(next: string) {
    if (next === provider) return;
    setProvider(next);
    // The model follows. Leaving the old one selected under a new
    // provider is a pair that fails at the far end.
    setTyping(false);
    setDraft(next === initialProvider ? initialModel : modelForProvider(models, next));
    setKey("");
  }

  return (
    <div className="flex flex-col gap-3">
      <div>
        <h3 className="text-sm font-medium">{title}</h3>
        <p className="text-muted-foreground text-xs">{description}</p>
      </div>

      {usable.length === 0 ? (
        <p className="text-muted-foreground text-sm">
          Niles has no provider for this yet.
        </p>
      ) : (
        <>
          <div
            role="radiogroup"
            aria-label={`${title} provider`}
            className="bg-muted/60 flex w-fit flex-wrap gap-1 rounded-lg p-1"
          >
            {usable.map((p) => {
              const active = p.id === provider;
              return (
                <button
                  key={p.id}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  disabled={saving}
                  onClick={() => pick(p.id)}
                  className={cn(
                    "flex items-center gap-2 rounded-md px-3 py-1.5 text-sm transition-colors",
                    "focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
                    active
                      ? "bg-background text-foreground shadow-sm"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <BrandMark id={p.id} label={p.label} className="size-4" />
                  {p.label}
                </button>
              );
            })}
          </div>

          {picked && <p className="text-muted-foreground text-xs">{picked.needs}</p>}

          <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
            <label className="flex min-w-0 flex-1 flex-col gap-1">
              <span className="text-muted-foreground text-xs">Model</span>
              {pickingModel ? (
                <Select
                  value={draft || null}
                  disabled={saving}
                  onValueChange={(next: string | null) => {
                    if (next === OTHER) {
                      setTyping(true);
                      setDraft("");
                      return;
                    }
                    setDraft(next ?? "");
                  }}
                >
                  <SelectTrigger aria-label={`${title} model`} className="h-9 w-full">
                    <SelectValue placeholder={defaultModel ?? "Pick one"} />
                  </SelectTrigger>
                  <SelectContent>
                    {offered.map((name) => (
                      <SelectItem key={name} value={name}>
                        {name}
                      </SelectItem>
                    ))}
                    <SelectItem value={OTHER}>Something else…</SelectItem>
                  </SelectContent>
                </Select>
              ) : (
                <Input
                  value={draft}
                  aria-label={`${title} model`}
                  spellCheck={false}
                  disabled={saving}
                  // An unset model is not an empty one — Niles ships a
                  // working default, and a blank box made the page look
                  // like it was asking for something it already had.
                  placeholder={defaultModel}
                  onChange={(e) => setDraft(e.target.value)}
                  className="font-mono"
                />
              )}
            </label>

            {/* Only here. Hearing has nothing to think about, and a
                control that does nothing is worse than one that is absent. */}
            {role === "llm" && (
              <label className="flex flex-col gap-1 sm:w-56">
                <span className="text-muted-foreground text-xs">Reasoning effort</span>
                <Select
                  value={thinking}
                  disabled={saving}
                  onValueChange={(next: string | null) =>
                    setThinking(next ?? PROVIDER_DEFAULT)
                  }
                >
                  <SelectTrigger
                    aria-label={`${title} reasoning effort`}
                    className="h-9 w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {EFFORTS.map((e) => (
                      <SelectItem key={e.value} value={e.value}>
                        {e.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
            )}
          </div>

          {picked && secret && (
            <KeyField
              label={secret.label}
              source={secret.source}
              writable={secrets?.writable ?? false}
              value={key}
              disabled={saving}
              onChange={setKey}
            />
          )}

          <div>
            <Button
              variant="outline"
              disabled={saving || !dirty || !draft.trim()}
              onClick={() =>
                onSave({
                  provider: provider || undefined,
                  model: draft.trim(),
                  effort: effortToSave(thinking),
                  key: key.trim() || undefined,
                })
              }
            >
              Save
            </Button>
          </div>
        </>
      )}

      {/* An unset model is not an empty one. Saying which one is
          running beats a blank box that looks like it wants something. */}
      {!model && defaultModel && !provider && (
        <p className="text-muted-foreground text-xs">
          Using <span className="font-mono">{defaultModel}</span>, which is what
          Niles ships with.
        </p>
      )}

      {/* Said rather than offered. Somebody whose config still carries
          its own endpoint is not misconfigured, and telling them
          nothing is chosen while it works would be worse than saying
          nothing at all. */}
      {!current && fallbackHost && (
        <p className="text-muted-foreground text-xs">
          Using <span className="font-mono">{fallbackHost}</span> directly,
          without a provider. Picking one here replaces it.
        </p>
      )}
    </div>
  );
}

/**
 * The picked provider's key, typed in beside it and saved with the rest.
 *
 * Write-only, like every key: the page can say whether one is set and
 * never what it is. One already coming from an environment variable is
 * left alone — offering to replace it from here would quietly move
 * where it lives.
 */
function KeyField({
  label,
  source,
  writable,
  value,
  disabled,
  onChange,
}: {
  label: string;
  source: "environment" | "stored" | "unset";
  writable: boolean;
  value: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-muted-foreground text-xs">API key</span>
        {source !== "unset" && (
          <span className="text-muted-foreground flex items-center gap-1 text-xs">
            <Check aria-hidden className="text-lit size-3" />
            {source === "stored" ? "saved here" : "from the environment"}
          </span>
        )}
      </div>
      {source === "environment" ? (
        <p className="text-muted-foreground text-xs">
          Coming from an environment variable. Remove it there to manage it here
          instead.
        </p>
      ) : (
        <Input
          type="password"
          // A key being set, never a login: "off" alone leaves Chrome
          // pairing it with whatever text box sits nearest.
          autoComplete="new-password"
          aria-label={label}
          placeholder={source === "stored" ? "Replace it…" : "Not set"}
          value={value}
          disabled={!writable || disabled}
          onChange={(e) => onChange(e.target.value)}
        />
      )}
    </div>
  );
}
