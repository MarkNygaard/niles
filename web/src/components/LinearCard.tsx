import { useState } from "react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { SecretField } from "@/components/SecretField";
import type { Secret } from "@/lib/api";

export interface LinearCardProps {
  team: string;
  triggerLabel: string;
  secret?: Secret;
  writable: boolean;
  saving?: boolean;
  onChange: (entries: { path: string; value: unknown }[]) => void;
  onSecretsChanged: () => void;
}

/**
 * Linear, which needs a team as well as a key.
 *
 * Its own card rather than a row in a list, because a key on its own
 * connects nothing here: Niles has to know which team's issues are
 * meant for it, and which label says an issue is.
 */
export function LinearCard({
  team,
  triggerLabel,
  secret,
  writable,
  saving,
  onChange,
  onSecretsChanged,
}: LinearCardProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Linear</CardTitle>
        <CardDescription>
          An issue in this team, carrying the label below, becomes work Niles
          picks up.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-col gap-2 sm:flex-row">
          <Field
            label="Team"
            value={team}
            placeholder="niles"
            disabled={saving}
            onCommit={(value) =>
              onChange([{ path: "integrations.linear.team", value }])
            }
          />
          <Field
            label="Trigger label"
            value={triggerLabel}
            placeholder="AI Eligible"
            disabled={saving}
            onCommit={(value) =>
              onChange([{ path: "integrations.linear.trigger_label", value }])
            }
          />
        </div>

        {secret && (
          <SecretField
            secret={secret}
            writable={writable}
            onChanged={onSecretsChanged}
          />
        )}

        {!team.trim() && (
          <p className="text-muted-foreground text-xs">
            Niles will not read anything until a team is named.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/** Writes when you leave it, so half a team name is never saved. */
function Field({
  label,
  value,
  placeholder,
  disabled,
  onCommit,
}: {
  label: string;
  value: string;
  placeholder: string;
  disabled?: boolean;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(value);
  }

  return (
    <label className="flex min-w-0 flex-1 flex-col gap-1">
      <span className="text-muted-foreground text-xs">{label}</span>
      <Input
        value={draft}
        aria-label={label}
        placeholder={placeholder}
        disabled={disabled}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          if (draft.trim() && draft !== value) onCommit(draft.trim());
          else setDraft(value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") setDraft(value);
        }}
      />
    </label>
  );
}
