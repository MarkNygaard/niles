import { SecretField } from "@/components/SecretField";
import type { Secret } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface ClaudeCodePanelProps {
  model: string;
  secret?: Secret;
  writable: boolean;
  saving?: boolean;
  onModel: (model: string) => void;
  onSecretsChanged: () => void;
}

/** What Claude Code is asked for. Its own short names, which follow the
    newest model of each size without Niles having to be told. */
export const MODELS = [
  { id: "haiku", label: "Haiku", hint: "Quickest" },
  { id: "sonnet", label: "Sonnet", hint: "Balanced" },
  { id: "opus", label: "Opus", hint: "Most capable" },
];

/**
 * Claude Code, answering the chat on the household's own subscription.
 *
 * Only the chat: a spoken command still goes to the voice model, which
 * answers in under a second where this takes several. Without a token,
 * or if Claude Code fails, the chat falls back to that model too.
 */
export function ClaudeCodePanel({
  model,
  secret,
  writable,
  saving,
  onModel,
  onSecretsChanged,
}: ClaudeCodePanelProps) {
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        Answers the chat with Claude, using Niles's own tools. Spoken commands
        keep the faster voice model.
      </p>

      <div>
        <div className="text-muted-foreground mb-1.5 text-xs">Model</div>
        {/* Buttons, not a select: three choices fit on a row, and the
            hint under each is the reason to pick it. */}
        <div className="bg-muted/60 flex gap-1 rounded-lg p-1" role="radiogroup" aria-label="Model">
          {MODELS.map((m) => {
            const active = m.id === model;
            return (
              <button
                key={m.id}
                type="button"
                role="radio"
                aria-checked={active}
                disabled={saving}
                onClick={() => !active && onModel(m.id)}
                className={cn(
                  "flex flex-1 flex-col items-center rounded-md px-1 py-1.5 text-sm transition-colors",
                  "focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
                  active
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {m.label}
                <span className="text-muted-foreground text-[11px]">{m.hint}</span>
              </button>
            );
          })}
        </div>
      </div>

      {secret && (
        <SecretField secret={secret} writable={writable} onChanged={onSecretsChanged} />
      )}

      <p className="text-muted-foreground text-xs">
        Run <code className="font-mono">claude setup-token</code> on a computer
        signed in to your Claude account, and paste the token it prints. It lasts
        a year.
      </p>
    </div>
  );
}
