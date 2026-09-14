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
  saving?: boolean;
  onSave: (change: { provider?: string; model: string }) => void;
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
  current,
  model,
  defaultModel,
  fallbackHost,
  saving,
  onSave,
}: RoleCardProps) {
  const [provider, setProvider] = useState(current ?? "");
  const [draft, setDraft] = useState(model);

  useEffect(() => setProvider(current ?? ""), [current]);
  useEffect(() => setDraft(model), [model]);

  const usable = usableFor(providers, role);
  const dirty = (current ?? "") !== provider || model !== draft;

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
            onValueChange={(next: string | null) => setProvider(next ?? "")}
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
        </label>

        <Button
          variant="outline"
          disabled={saving || !dirty || !draft.trim()}
          onClick={() =>
            onSave({
              provider: provider || undefined,
              model: draft.trim(),
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
