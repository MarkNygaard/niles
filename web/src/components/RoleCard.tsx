import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
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
export function RoleCard({
  role,
  title,
  description,
  providers,
  current,
  model,
  fallbackHost,
  saving,
  onSave,
}: RoleCardProps) {
  const [provider, setProvider] = useState(current ?? "");
  const [draft, setDraft] = useState(model);

  useEffect(() => setProvider(current ?? ""), [current]);
  useEffect(() => setDraft(model), [model]);

  const usable = providers.filter(
    (p) => !p.serves || p.serves.length === 0 || p.serves.includes(role),
  );
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
      <div className="flex flex-col gap-2 sm:flex-row">
        <select
          aria-label={`${title} provider`}
          value={provider}
          disabled={saving}
          onChange={(e) => setProvider(e.target.value)}
          className={cn(
            "border-input bg-background h-9 rounded-lg border px-2 text-sm sm:w-44",
            "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          )}
        >
          {/* An empty choice only while nothing is picked, so the box
              cannot claim a provider that was never chosen. Once one is,
              it stops being offered: unpicking would silently fall back
              to the config file. */}
          {!current && <option value="">Pick one</option>}
          {usable.map((p) => (
            <option key={p.name} value={p.name}>
              {p.name}
            </option>
          ))}
        </select>

        <Input
          value={draft}
          aria-label={`${title} model`}
          spellCheck={false}
          disabled={saving}
          onChange={(e) => setDraft(e.target.value)}
          className="font-mono sm:flex-1"
        />

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
