import { useState } from "react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { CommitField } from "@/components/CommitField";
import { humanize } from "@/lib/rooms";
import type { BlindOverride, Device } from "@/lib/api";
import type { Setting } from "@/components/SettingRow";

export interface BlindsCardProps {
  /** Every registered blind. The card draws nothing without one. */
  blinds: Device[];
  /** The house level in force, for each row's placeholder. */
  houseLevel: number;
  overrides: BlindOverride[];
  saving?: boolean;
  error?: string;
  onChange: (overrides: BlindOverride[]) => void;
  /** Rendered by the panel, which owns saving and undo for a row. */
  row: (spec: {
    label: string;
    description: string;
    settings: Setting[];
  }) => React.ReactNode;
}

/**
 * The house level and the per-blind overrides, as the Settings page
 * edits them. Kept in Postgres through the config store — no
 * `niles.toml` section is needed to turn this on — and read from the
 * live snapshot on every command, so a change applies to the next
 * press, the next utterance and the next rule run.
 */
const BLINDS_ROWS: Setting[] = [
  { path: "blinds.normal_open", kind: "number", caption: "opens to (%)", width: "w-24" },
];

export function BlindsCard({
  blinds,
  houseLevel,
  overrides,
  saving,
  error,
  onChange,
  row,
}: BlindsCardProps) {
  const [problem, setProblem] = useState<string | null>(null);

  if (blinds.length === 0) return null;

  // Room then name, so a house's blinds read the way the house does.
  const sorted = [...blinds].sort(
    (a, b) => a.room.localeCompare(b.room) || a.name.localeCompare(b.name),
  );

  function commit(blind: Device, raw: string) {
    const existing = overrides.find((o) => o.device === blind.id);
    if (raw === "") {
      setProblem(null);
      if (existing) onChange(overrides.filter((o) => o.device !== blind.id));
      return;
    }
    const n = Number(raw);
    if (!Number.isInteger(n) || n < 1 || n > 100) {
      setProblem("A blind opens to 1–100%.");
      return;
    }
    setProblem(null);
    const next = { device: blind.id, normal_open: n };
    onChange(
      existing
        ? overrides.map((o) => (o.device === blind.id ? next : o))
        : [...overrides, next],
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Blinds</CardTitle>
        <CardDescription>
          How far "open" opens. A blind goes to its normal level — the
          house's, or its own below — and all the way when it is already
          there, which is what a second press means. A whole room only
          goes all the way when every blind in it is already at its own
          level. Applies at once: every command reads the live setting.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {row({
          label: "Every blind",
          description: "Where a plain Open takes a blind that has no level of its own.",
          settings: BLINDS_ROWS,
        })}
        <div className="divide-border divide-y">
          {sorted.map((blind) => {
            const name = humanize(blind.name);
            const override = overrides.find((o) => o.device === blind.id);
            return (
              <div key={blind.id} className="flex items-center gap-3 py-2.5">
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">{name}</span>
                  <span className="text-muted-foreground block truncate text-xs">
                    {humanize(blind.room)}
                  </span>
                </span>
                <CommitField
                  allowEmpty
                  label={`${name} opens to`}
                  placeholder={String(houseLevel)}
                  value={override ? String(override.normal_open) : ""}
                  disabled={saving}
                  onCommit={(raw) => commit(blind, raw)}
                />
              </div>
            );
          })}
        </div>
        {(problem || error) && (
          <p className="text-destructive text-sm">{problem ?? error}</p>
        )}
      </CardContent>
    </Card>
  );
}
