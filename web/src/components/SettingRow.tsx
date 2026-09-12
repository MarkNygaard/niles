import { Fragment, useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RotateCcw } from "lucide-react";
import { DevicePicker } from "@/components/DevicePicker";
import type { DeviceOption } from "@/components/DevicePicker";
import { cn } from "@/lib/utils";

/**
 * What a setting holds, declared rather than sniffed from the current
 * value. A setting that isn't configured yet has no value to sniff, and
 * those are exactly the ones someone opens this page to set.
 */
export type FieldKind = "number" | "text" | "devices";

export interface Setting {
  /** Dotted path, e.g. `lighting.morning_start`. */
  path: string;
  kind: FieldKind;
  /** Names this input within the row — "starts", "night". */
  caption: string;
  /** Width class. Times and percentages don't need the same room as ids. */
  width?: string;
  /** `devices` only: what can be picked. Filled in by the page. */
  options?: DeviceOption[];
  /** `devices` only: what to say when there is nothing to pick. */
  optionsEmpty?: string;
}

export interface SettingRowProps {
  /** The thing being configured, not the field: "Morning ramp". */
  label: string;
  /** One line on what it does. */
  description: string;
  /** One or two inputs. Two when the pair is one idea — a span, a range. */
  settings: Setting[];
  /** Drawn between two inputs: `→` for a span, nothing for a pair. */
  joiner?: string;
  /** Effective value per path; `undefined` for a setting that is unset. */
  values: Record<string, unknown>;
  /** The subset of paths with an override in force. */
  overridden: string[];
  /** False for boot-only sections: the edit is stored but needs a restart. */
  hot: boolean;
  saving?: boolean;
  /** The server's refusal of the last save of *this* row, verbatim. */
  error?: string;
  onSave: (entries: Array<{ path: string; value: unknown }>) => void;
  onReset: (paths: string[]) => void;
}

/**
 * One row of the config: a thing, what it does, and the one or two
 * values that define it.
 *
 * A ramp's start and its end are not two settings that happen to sit
 * near each other — they are the two ends of one span, and reading
 * either without the other tells you nothing. So the row is the unit:
 * one label, one save, one revision in the history.
 *
 * Deliberately not live-updating. Config writes are persistent,
 * validated, and land on real lights, so each change is an explicit
 * save. A slider that wrote on every pixel of drag would spam the
 * broker and fill the undo history.
 */
export function SettingRow({
  label,
  description,
  settings,
  joiner,
  values,
  overridden,
  hot,
  saving,
  error,
  onSave,
  onReset,
}: SettingRowProps) {
  const server = Object.fromEntries(
    settings.map((s) => [s.path, format(values[s.path], s.kind)]),
  );
  const [drafts, setDrafts] = useState(server);

  // Adopt the server's values whenever they change underneath us —
  // which happens for real here, because the same setting can be
  // changed by voice while this page is open.
  const signature = settings.map((s) => server[s.path]).join("|");
  // Keyed off the signature rather than `server`, which is a fresh
  // object every render and would re-run this forever.
  useEffect(() => {
    setDrafts(server);
  }, [signature]);

  // Picking a light from a list is already a deliberate act, so that
  // row writes on the spot and has no Save button at all. Typing into a
  // box isn't: half a time is a value, and it must not reach the lights.
  const instant = settings.every((s) => s.kind === "devices");
  const changed = settings.filter((s) => drafts[s.path] !== server[s.path]);
  const blocked = changed.some((s) => !usable(drafts[s.path], s.kind));
  const unset = settings.filter((s) => values[s.path] === undefined);

  function save() {
    if (changed.length === 0 || blocked || saving) return;
    onSave(changed.map((s) => ({ path: s.path, value: parse(drafts[s.path], s.kind) })));
  }

  return (
    <div className="grid gap-x-8 gap-y-3 py-4 sm:grid-cols-[minmax(190px,240px)_minmax(0,1fr)]">
      <div className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <Label htmlFor={settings[0].path} className="text-sm font-medium">
            {label}
          </Label>
          {overridden.length > 0 && (
            <Badge variant="secondary" title="Changed away from the config file">
              overridden
            </Badge>
          )}
          {unset.length === settings.length &&
            overridden.length === 0 &&
            !settings.some((s) => s.kind === "devices") && (
              <Badge variant="outline" title="Nothing is configured here">
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
        <p className="text-muted-foreground text-xs">{description}</p>
      </div>

      <div className="flex flex-wrap items-start gap-2">
        {settings.map((setting, index) => (
          <Fragment key={setting.path}>
            {index > 0 && joiner && (
              <span className="text-muted-foreground/60 h-8 leading-8 text-sm select-none">
                {joiner}
              </span>
            )}
            <div className={cn("flex flex-col gap-1", setting.width ?? "w-28")}>
              {setting.kind === "devices" ? (
                <DevicePicker
                  id={setting.path}
                  aria-label={`${label} ${setting.caption}`}
                  value={split(drafts[setting.path] ?? "")}
                  options={setting.options ?? []}
                  disabled={saving}
                  emptyMessage={setting.optionsEmpty ?? "No lights to pick from."}
                  onChange={(next) => {
                    setDrafts({ ...drafts, [setting.path]: next.join(", ") });
                    onSave([{ path: setting.path, value: next }]);
                  }}
                />
              ) : (
              <Input
                id={setting.path}
                aria-label={`${label} ${setting.caption}`}
                title={setting.path}
                value={drafts[setting.path] ?? ""}
                inputMode={setting.kind === "number" ? "numeric" : "text"}
                placeholder={values[setting.path] === undefined ? "not set" : undefined}
                disabled={saving}
                aria-invalid={error ? true : undefined}
                onChange={(e) =>
                  setDrafts({ ...drafts, [setting.path]: e.target.value })
                }
                onKeyDown={(e) => {
                  if (e.key === "Enter") save();
                  if (e.key === "Escape") setDrafts(server);
                }}
                className={cn(
                  "font-mono",
                  drafts[setting.path] !== server[setting.path] &&
                    usable(drafts[setting.path], setting.kind) &&
                    "border-primary",
                )}
              />
              )}
              <span className="text-muted-foreground/70 pl-0.5 text-[11px]">
                {setting.caption}
              </span>
            </div>
          </Fragment>
        ))}

        <div className="ml-auto flex items-center gap-1">
          {/* Kept in the layout when there is nothing to save, so a row
              doesn't jump sideways the moment you type in it. */}
          {!instant && (
            <Button
              onClick={save}
              disabled={changed.length === 0 || blocked || saving}
              className={cn(changed.length === 0 && "invisible")}
            >
              Save
            </Button>
          )}
          {overridden.length > 0 && (
            <Button
              size="icon"
              variant="ghost"
              onClick={() => onReset(overridden)}
              disabled={saving}
              aria-label="Reset to the config file"
              title="Return to the value in the config file"
            >
              <RotateCcw />
            </Button>
          )}
        </div>
      </div>

      {/* The server's rejection names the field and the reason; showing
          our own message instead would lose that. */}
      {error && (
        <p className="text-destructive text-xs sm:col-start-2">{error}</p>
      )}
    </div>
  );
}

/** The value as the user should see and edit it. */
function format(value: unknown, kind: FieldKind): string {
  if (value === null || value === undefined) return "";
  if (kind === "devices") {
    return Array.isArray(value) ? value.join(", ") : String(value);
  }
  return String(value);
}

/** A comma-separated draft as the list it stands for. */
function split(draft: string): string[] {
  return draft
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);
}

/** The typed value to send back, from what the user typed. */
function parse(draft: string, kind: FieldKind): unknown {
  switch (kind) {
    case "number":
      return Number(draft);
    case "devices":
      return split(draft);
    default:
      return draft;
  }
}

/**
 * Whether what's typed is worth sending.
 *
 * An empty device list means "none", which is a value. An empty number is not
 * one — `Number("")` is 0, and quietly writing 0 to a brightness is
 * worse than refusing. Nor is a number that isn't one: that would reach
 * the server as JSON `null` and come back as an error about the request
 * body rather than about the setting.
 */
function usable(draft: string, kind: FieldKind): boolean {
  if (kind === "devices") return true;
  if (draft.trim() === "") return false;
  return kind !== "number" || !Number.isNaN(Number(draft));
}
