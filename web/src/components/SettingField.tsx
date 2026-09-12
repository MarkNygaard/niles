import { useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RotateCcw } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * What a field holds, declared per field rather than sniffed from the
 * current value. A setting that isn't configured yet has no value to
 * sniff, and those are exactly the ones a user comes here to set.
 */
export type FieldKind = "number" | "text" | "list";

export interface SettingFieldProps {
  /** Dotted path, e.g. `lighting.daytime_brightness`. */
  path: string;
  label: string;
  kind: FieldKind;
  /** What Niles is running right now; `undefined` when unset. */
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
  kind,
  value,
  overridden,
  hot,
  hint,
  saving,
  error,
  onSave,
  onReset,
}: SettingFieldProps) {
  const serverValue = format(value, kind);
  const [draft, setDraft] = useState(serverValue);

  // Adopt the server's value whenever it changes underneath us — which
  // happens for real here, because the same setting can be changed by
  // voice while this page is open.
  useEffect(() => {
    setDraft(serverValue);
  }, [serverValue]);

  const dirty = draft !== serverValue;
  // An empty list means "no lights", which is a value worth saving. An
  // empty number or time is not a value at all — `Number("")` is 0, and
  // silently writing 0 is worse than refusing. A number that isn't one
  // would reach the server as JSON `null` and come back as a parse error
  // about the request body rather than about the field.
  const blank = draft.trim() === "" && kind !== "list";
  const unusable = blank || (kind === "number" && Number.isNaN(Number(draft)));

  function save() {
    if (!dirty || unusable) return;
    onSave(parse(draft, kind));
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <Label htmlFor={path} className="text-sm font-medium">
          {label}
        </Label>
        {overridden && (
          <Badge variant="secondary" title="Changed away from the config file">
            overridden
          </Badge>
        )}
        {value === undefined && !overridden && (
          <Badge variant="outline" title="Nothing is configured for this setting">
            not set
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
          inputMode={kind === "number" ? "numeric" : "text"}
          placeholder={value === undefined ? "not set" : undefined}
          disabled={saving}
          aria-invalid={error ? true : undefined}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") save();
            if (e.key === "Escape") setDraft(serverValue);
          }}
          className={cn(
            "font-mono",
            kind === "list" ? "max-w-full" : "max-w-40",
            dirty && !unusable && "border-primary",
          )}
        />
        <Button onClick={save} disabled={!dirty || unusable || saving}>
          Save
        </Button>
        {overridden && (
          <Button
            variant="ghost"
            onClick={onReset}
            disabled={saving}
            title="Return to the value in the config file"
          >
            <RotateCcw /> Reset
          </Button>
        )}
      </div>

      {/* The hint and the path share a line: the path matters (it is what
          the API and voice use) but it is reference, not instruction. */}
      <div className="flex items-baseline justify-between gap-3">
        {/* The server's rejection names the field and the reason; showing
            our own message instead would lose that. */}
        {error ? (
          <p className="text-destructive text-xs">{error}</p>
        ) : (
          <p className="text-muted-foreground text-xs">{hint}</p>
        )}
        <p className="text-muted-foreground/60 shrink-0 font-mono text-[11px]">
          {path}
        </p>
      </div>
    </div>
  );
}

/** The value as the user should see and edit it. */
function format(value: unknown, kind: FieldKind): string {
  if (value === null || value === undefined) return "";
  if (kind === "list") return Array.isArray(value) ? value.join(", ") : String(value);
  return String(value);
}

/** The typed value to send back, from what the user typed. */
function parse(draft: string, kind: FieldKind): unknown {
  switch (kind) {
    case "number":
      return Number(draft);
    case "list":
      return draft
        .split(",")
        .map((entry) => entry.trim())
        .filter(Boolean);
    default:
      return draft;
  }
}
