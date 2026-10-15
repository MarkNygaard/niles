import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { SecretField } from "@/components/SecretField";
import { api } from "@/lib/api";
import type { PluginSetting, PluginSummary, Secret, SecretsReport } from "@/lib/api";
import {
  choiceOptions,
  defaultText,
  draftOf,
  roomOptions,
  settingsPatch,
} from "@/lib/plugins";
import type { Draft } from "@/lib/plugins";

export interface PluginSettingsProps {
  plugin: PluginSummary;
  /** The house's rooms, for a `room` setting. */
  rooms: string[];
  /** The secret store's report, for whether each secret is set. */
  secrets?: SecretsReport;
  /** Saved: ask for the plugin, its status and the secrets again. */
  onSaved: () => void;
}

/**
 * A plugin's setup panel, generated from its manifest's settings.
 *
 * Values go into the config under `[plugins.<id>]` with `PATCH
 * /config`, which restarts the plugin with them; secrets into the
 * encrypted store under `plugins.<id>.<key>`, one field each, written
 * on their own and never shown again. A field left empty takes its
 * default, which is written beside it rather than into it, so what the
 * household entered and what the plugin assumed stay told apart.
 */
export function PluginSettings({ plugin, rooms, secrets, onSaved }: PluginSettingsProps) {
  const [draft, setDraft] = useState<Draft>(() => draftOf(plugin.settings));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const plain = plugin.settings.filter((s) => s.type !== "secret");
  const secret = plugin.settings.filter((s) => s.type === "secret");
  if (plugin.settings.length === 0) return null;

  async function save() {
    const { patch, errors } = settingsPatch(plugin.id, plugin.settings, draft);
    setErrors(errors);
    setFailure(null);
    setSaved(false);
    if (Object.keys(errors).length > 0 || !patch) return;
    setBusy(true);
    try {
      await api.patchConfig(patch);
      setSaved(true);
      onSaved();
    } catch (e) {
      setFailure(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const set = (key: string, value: string | boolean) => {
    setSaved(false);
    setDraft((d) => ({ ...d, [key]: value }));
  };

  return (
    <section aria-label="Settings" className="flex flex-col gap-4">
      {plain.length > 0 && (
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            save();
          }}
        >
          {plain.map((setting) => (
            <Field key={setting.key} setting={setting} error={errors[setting.key]}>
              <Control
                setting={setting}
                value={draft[setting.key]}
                rooms={rooms}
                disabled={busy}
                onChange={(value) => set(setting.key, value)}
              />
            </Field>
          ))}
          <div className="flex items-center gap-3">
            <Button type="submit" size="sm" disabled={busy}>
              Save
            </Button>
            {saved && <span className="text-muted-foreground text-xs">Saved; it restarts with them.</span>}
          </div>
          {failure && (
            <p role="alert" className="text-destructive text-sm">
              {failure}
            </p>
          )}
        </form>
      )}
      {secret.map((setting) => (
        <div key={setting.key} className="flex flex-col gap-1">
          <SecretField
            secret={secretFor(plugin, setting, secrets)}
            writable={secrets?.writable ?? true}
            onChanged={onSaved}
          />
          <Notes setting={setting} />
        </div>
      ))}
    </section>
  );
}

/** The store's own entry for a secret setting, or one standing in for
    it, from the plugin's own word on whether it is set, until the
    store's report arrives. */
function secretFor(
  plugin: PluginSummary,
  setting: PluginSetting,
  secrets?: SecretsReport,
): Secret {
  const key = `plugins.${plugin.id}.${setting.key}`;
  const known = secrets?.secrets.find((s) => s.key === key);
  const label = setting.required ? `${setting.label} (required)` : setting.label;
  return known
    ? { ...known, label }
    : { key, label, source: setting.set ? "stored" : "unset" };
}

function Field({
  setting,
  error,
  children,
}: {
  setting: PluginSetting;
  error?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      {setting.type !== "boolean" && (
        <span className="text-sm font-medium">
          {setting.label}
          {setting.required && (
            <span className="text-muted-foreground ml-1.5 text-xs font-normal">required</span>
          )}
        </span>
      )}
      {children}
      <Notes setting={setting} />
      {error && (
        <p role="alert" className="text-destructive text-xs">
          {error}
        </p>
      )}
    </div>
  );
}

function Notes({ setting }: { setting: PluginSetting }) {
  const fallback = defaultText(setting);
  if (!setting.help && !fallback) return null;
  return (
    <p className="text-muted-foreground text-xs">
      {setting.help}
      {setting.help && fallback && " "}
      {fallback && `Default: ${fallback}.`}
    </p>
  );
}

function Control({
  setting,
  value,
  rooms,
  disabled,
  onChange,
}: {
  setting: PluginSetting;
  value: string | boolean | undefined;
  rooms: string[];
  disabled: boolean;
  onChange: (value: string | boolean) => void;
}) {
  switch (setting.type) {
    case "boolean":
      return (
        <div className="flex min-h-9 items-center justify-between gap-3 text-sm font-medium">
          <span>
            {setting.label}
            {setting.required && (
              <span className="text-muted-foreground ml-1.5 text-xs font-normal">required</span>
            )}
          </span>
          <Switch
            aria-label={setting.label}
            checked={value === true}
            disabled={disabled}
            onCheckedChange={(on: boolean) => onChange(on)}
          />
        </div>
      );
    case "choice":
    case "room": {
      const items =
        setting.type === "choice"
          ? choiceOptions(setting)
          : roomOptions(rooms, typeof value === "string" ? value : undefined);
      return (
        <Select
          items={items}
          value={typeof value === "string" && value !== "" ? value : null}
          disabled={disabled}
          onValueChange={(next: string | null) => onChange(next ?? "")}
        >
          <SelectTrigger aria-label={setting.label} className="w-full">
            <SelectValue placeholder={defaultText(setting) ?? "Choose…"} />
          </SelectTrigger>
          <SelectContent>
            {items.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }
    default:
      return (
        <Input
          aria-label={setting.label}
          autoComplete="off"
          inputMode={setting.type === "number" ? "decimal" : undefined}
          placeholder={defaultText(setting) ?? undefined}
          value={typeof value === "string" ? value : ""}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        />
      );
  }
}
