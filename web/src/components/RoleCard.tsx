import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { Provider } from "@/lib/api";

export interface RoleCardProps {
  /** `stt`, `llm`, `llm.tier2`. */
  role: "stt" | "llm";
  title: string;
  description: string;
  providers: Provider[];
  /** What each provider offers for this role, keyed by provider name. */
  models?: Record<string, string[]>;
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
  saving?: boolean;
  onSave: (change: {
    provider?: string;
    model: string;
    effort?: string | null;
  }) => void;
}

/**
 * One job, and who does it.
 *
 * The provider and the model sit together because they change
 * together: a model name is not portable between providers, so picking
 * one without the other produces a request that fails at somebody
 * else's server rather than an error on this page. Saving them as one
 * edit also makes it one revision and one undo.
 *
 * Only providers that say they serve this role are offered — a
 * language-only provider has no speech endpoint, and offering it would
 * turn a 404 into the way you find that out.
 *
 * With none added there is no dropdown at all, only what to do about
 * it. The list used to carry an option called "From the config",
 * meaning the endpoint and key written into this role's own section.
 * That is still honoured, and is still how a config written before
 * integrations existed works — but it is not something to *choose*, so
 * it is reported as the current state rather than offered as an option.
 */
/**
 * The providers that can do this job.
 *
 * A function rather than a line inside the component because it is the
 * one rule here worth testing and the popup it feeds cannot be opened
 * in jsdom — Base UI's Select hangs it. Testing the rule directly beats
 * testing nothing.
 *
 * A provider that says nothing about what it serves is assumed able:
 * refusing to use something because nobody wrote down what it does is
 * worse than letting it fail with the provider's own error.
 */
/** The sentinel for "let me type one", which is not a model name. */
const OTHER = " other";

/** The sentinel for "whatever the provider does", which is not a value. */
const PROVIDER_DEFAULT = " default";

/**
 * How hard to think before answering, as the page offers it.
 *
 * Only on the language model, because it is the only role it means
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
 * across a provider switch it put Groq's models in Cerebras's
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

export function usableFor(providers: Provider[], role: "stt" | "llm") {
  return providers.filter(
    (p) => !p.serves || p.serves.length === 0 || p.serves.includes(role),
  );
}

export function RoleCard({
  role,
  title,
  description,
  providers,
  models,
  current,
  model,
  effort,
  defaultModel,
  fallbackHost,
  saving,
  onSave,
}: RoleCardProps) {
  const [provider, setProvider] = useState(current ?? "");
  const [draft, setDraft] = useState(model);
  const [thinking, setThinking] = useState(effortValue(effort));

  useEffect(() => setProvider(current ?? ""), [current]);
  useEffect(() => setDraft(model), [model]);
  useEffect(() => setThinking(effortValue(effort)), [effort]);

  const usable = usableFor(providers, role);
  const dirty =
    (current ?? "") !== provider ||
    model !== draft ||
    effortValue(effort) !== thinking;
  const known = models?.[provider] ?? [];
  // `defaultModel` is deliberately not in here. It is what Niles ships
  // with, which belongs to the provider Niles ships with — offering it
  // under a different one is the leak this replaced.
  const offered = offeredFor(models, provider, current ?? "", draft);
  // A list Niles ships will go stale the week a provider adds
  // something, so typing one stays possible — just not the first thing
  // you are asked to do. Decided by what the *provider* offers, not by
  // `offered`: a provider nothing is known about would otherwise get a
  // dropdown holding only the value it already had.
  const [typing, setTyping] = useState(false);
  const picking = known.length > 0 && !typing;

  return (
    <div className="flex flex-col gap-3">
      <div>
        <h3 className="text-sm font-medium">{title}</h3>
        <p className="text-muted-foreground text-xs">{description}</p>
      </div>

      {usable.length === 0 ? (
        <p className="text-muted-foreground text-sm">
          Nothing set up can do this yet. Add one under Integrations.
        </p>
      ) : (
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
        <label className="flex flex-col gap-1 sm:w-44">
          <span className="text-muted-foreground text-xs">Provider</span>
          <Select
            value={provider || null}
            disabled={saving}
            onValueChange={(next: string | null) => {
              const picked = next ?? "";
              setProvider(picked);
              // The model follows. Leaving the old one selected under a
              // new provider is a pair that fails at the far end.
              if (picked !== provider) {
                setTyping(false);
                setDraft(modelForProvider(models, picked));
              }
            }}
          >
            <SelectTrigger
              aria-label={`${title} provider`}
              className="h-9 w-full"
            >
              <SelectValue placeholder="Pick one" />
            </SelectTrigger>
            <SelectContent>
              {usable.map((p) => (
                <SelectItem key={p.name} value={p.name}>
                  {p.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>

        <label className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="text-muted-foreground text-xs">Model</span>
          {picking ? (
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
              <SelectTrigger
                aria-label={`${title} model`}
                className="h-9 w-full"
              >
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

        {/* Only here. Speech-to-text has nothing to think about, and a
            control that does nothing is worse than one that is absent. */}
        {role === "llm" && (
          <label className="flex flex-col gap-1 sm:w-56">
            <span className="text-muted-foreground text-xs">Thinking</span>
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

        <Button
          variant="outline"
          disabled={saving || !dirty || !draft.trim()}
          onClick={() =>
            onSave({
              provider: provider || undefined,
              model: draft.trim(),
              effort: effortToSave(thinking),
            })
          }
        >
          Save
        </Button>
      </div>
      )}

      {/* An unset model is not an empty one. Saying which one is
          running beats a blank box that looks like it wants something. */}
      {!model && defaultModel && (
        <p className="text-muted-foreground text-xs">
          Using <span className="font-mono">{defaultModel}</span>, which is
          what Niles ships with. Type a name here to pin a different one.
        </p>
      )}

      {/* Said rather than offered. Somebody whose config still carries
          its own endpoint is not misconfigured, and telling them
          nothing is available while speech works would be worse than
          saying nothing at all. */}
      {!current && fallbackHost && (
        <p className="text-muted-foreground text-xs">
          Using <span className="font-mono">{fallbackHost}</span> from the
          config file. Picking something here replaces it.
        </p>
      )}
    </div>
  );
}
