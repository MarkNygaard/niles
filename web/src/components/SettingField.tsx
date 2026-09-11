import { useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RotateCcw } from "lucide-react";
import { cn } from "@/lib/utils";

export interface SettingFieldProps {
  /** Dotted path, e.g. `lighting.daytime_brightness`. */
  path: string;
  label: string;
  /** What Niles is running right now. */
  value: unknown;
  /** True when this value has been changed away from the config file. */
  overridden: boolean;
  /** False for boot-only sections: the edit is stored but needs a restart. */
  hot: boolean;
  hint?: string;
  saving?: boolean;
  /** Set when the last save of *this* field was refused, verbatim. */
  error?: string;
  onSave: (value: unknown) => void;
  onReset: () => void;
}

/**
 * One editable value.
 *
 * Deliberately not a live-updating control: config writes are
 * persistent, validated, and land on real lights, so each change is an
 * explicit save. A slider that wrote on every pixel of drag would spam
 * the broker and fill the undo history.
 */
export function SettingField({
  path,
  label,
  value,
  overridden,
  hot,
  hint,
  saving,
  error,
  onSave,
  onReset,
}: SettingFieldProps) {
  const serverValue = value === null || value === undefined ? "" : String(value);
  const [draft, setDraft] = useState(serverValue);

  // Adopt the server's value whenever it changes underneath us — which
  // happens for real here, because the same setting can be changed by
  // voice while this page is open.
  useEffect(() => {
    setDraft(serverValue);
  }, [serverValue]);

  const dirty = draft !== serverValue;
  const numeric = typeof value === "number";

  function save() {
    if (!dirty) return;
    onSave(numeric ? Number(draft) : draft);
  }

  return (
    <div className="flex flex-col gap-1.5 py-3">
      <div className="flex items-center gap-2">
        <Label htmlFor={path} className="text-sm font-medium">
          {label}
        </Label>
        {overridden && (
          <Badge variant="secondary" title="Changed away from the config file">
            overridden
          </Badge>
        )}
        {!hot && (
          <Badge
            variant="outline"
            title="Stored, but only read when Niles starts — needs a restart to take effect"
          >
            restart required
          </Badge>
        )}
      </div>

      <div className="flex items-center gap-2">
        <Input
          id={path}
          value={draft}
          inputMode={numeric ? "numeric" : "text"}
          disabled={saving}
          aria-invalid={error ? true : undefined}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") save();
            if (e.key === "Escape") setDraft(serverValue);
          }}
          className={cn("max-w-40 font-mono", dirty && "border-primary")}
        />
        <Button size="sm" onClick={save} disabled={!dirty || saving}>
          Save
        </Button>
        {overridden && (
          <Button
            size="sm"
            variant="ghost"
            onClick={onReset}
            disabled={saving}
            title="Return to the value in the config file"
          >
            <RotateCcw /> Reset
          </Button>
        )}
      </div>

      {/* The server's rejection names the field and the reason; showing
          our own message instead would lose that. */}
      {error ? (
        <p className="text-destructive text-xs">{error}</p>
      ) : hint ? (
        <p className="text-muted-foreground text-xs">{hint}</p>
      ) : null}
      <p className="text-muted-foreground/70 font-mono text-[11px]">{path}</p>
    </div>
  );
}
