import { useEffect, useState } from "react";
import { ChevronRight, RefreshCw, RotateCcw, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { PluginInboundSection } from "@/components/PluginInbound";
import { PluginLog } from "@/components/PluginLog";
import { PluginLogo, PluginPreviewView, shortRepo } from "@/components/PluginPreview";
import { PluginSettings } from "@/components/PluginSettings";
import { api } from "@/lib/api";
import type { PluginPreview, PluginSummary, SecretsReport } from "@/lib/api";
import type { Status } from "@/lib/integrations";
import { setupLine, widgetPlace, widgetPlaceOptions } from "@/lib/plugins";
import { cn } from "@/lib/utils";

/** An installed plugin on the Connected tab, laid out like a built-in's row. */
export function PluginRow({
  plugin,
  status,
  disabled,
  onOpen,
}: {
  plugin: PluginSummary;
  status: Status;
  disabled?: boolean;
  onOpen: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      aria-label={`Configure ${plugin.name}`}
      onClick={onOpen}
      className="hover:bg-muted focus-visible:bg-muted flex min-h-12 w-full items-center gap-3 px-3 py-2 text-left focus-visible:outline-none disabled:opacity-50"
    >
      <PluginLogo plugin={plugin} />
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm font-medium">{plugin.name}</span>
          {/* Every plugin says it is one: somebody else's code, chosen
              by this house, not a part of Niles. */}
          <Badge variant="outline" className="text-muted-foreground h-4 px-1.5 text-[10px]">
            Custom
          </Badge>
        </span>
        <span className="text-muted-foreground flex items-center gap-1.5 text-xs">
          <span
            aria-hidden
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              status.ok ? "bg-ok" : "bg-muted-foreground/40",
            )}
          />
          <span className="truncate">{status.text}</span>
        </span>
      </span>
      <ChevronRight aria-hidden className="text-muted-foreground size-4 shrink-0" />
    </button>
  );
}

function day(iso: string | null): string | null {
  if (!iso) return null;
  return new Date(iso).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

/**
 * An installed plugin's panel: where it came from, its settings, where
 * its widgets go, its log, and switching it off, updating, rolling back
 * and removing it.
 */
export function PluginPanel({
  plugin,
  rooms = [],
  secrets,
  effective,
  onChanged,
  onSecretsChanged,
  onPlaceWidget,
  onRemoved,
  outcome,
}: {
  plugin: PluginSummary;
  /** What the way back from its sign-in page said, when it just did. */
  outcome?: { ok: boolean; text: string } | null;
  /** The house's rooms, for a room setting and a widget's place. */
  rooms?: string[];
  secrets?: SecretsReport;
  /** The config as it is, for where each widget is now. */
  effective?: unknown;
  /** Something changed; ask for the list again. */
  onChanged: () => void;
  onSecretsChanged?: () => void;
  /** Put a widget on the front page, a room, or nowhere: `[menu.widgets]`. */
  onPlaceWidget?: (widget: string, place: string) => void;
  onRemoved: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [update, setUpdate] = useState<PluginPreview | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);

  async function run(work: () => Promise<void>) {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      await work();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const installed = day(plugin.installed_at);

  // A save restarts it; ask again until it has settled, so the panel
  // says how the new settings went rather than "Starting".
  useEffect(() => {
    if (plugin.status !== "starting") return;
    const again = setTimeout(onChanged, 1_500);
    return () => clearTimeout(again);
  }, [plugin.status, plugin.status_line, onChanged]);

  if (update) {
    return (
      <div className="flex flex-col gap-4">
        <PluginPreviewView preview={update} />
        <div className="flex gap-2">
          <Button
            disabled={busy}
            onClick={() =>
              run(async () => {
                await api.updatePlugin(plugin.id, update);
                setUpdate(null);
                onChanged();
              })
            }
          >
            Update to v{update.version}
          </Button>
          <Button variant="ghost" disabled={busy} onClick={() => setUpdate(null)}>
            Cancel
          </Button>
        </div>
        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 text-sm">
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
        <dt className="text-muted-foreground">Status</dt>
        <dd>{setupLine(plugin)}</dd>
        {plugin.repo && (
          <>
            <dt className="text-muted-foreground">From</dt>
            <dd className="min-w-0 truncate">
              <a
                href={plugin.repo}
                target="_blank"
                rel="noopener noreferrer"
                className="underline underline-offset-2"
              >
                {shortRepo(plugin.repo)}
              </a>
            </dd>
          </>
        )}
        <dt className="text-muted-foreground">Version</dt>
        <dd>{plugin.version}</dd>
        {plugin.authors.length > 0 && (
          <>
            <dt className="text-muted-foreground">By</dt>
            <dd>{plugin.authors.join(", ")}</dd>
          </>
        )}
        {installed && (
          <>
            <dt className="text-muted-foreground">Installed</dt>
            <dd>{installed}</dd>
          </>
        )}
      </dl>
      {plugin.older_interface && (
        <p className="text-muted-foreground">
          Built for an older Niles. It works, and a newer release may be on{" "}
          {plugin.repo ? (
            <a
              href={plugin.repo}
              target="_blank"
              rel="noopener noreferrer"
              className="underline underline-offset-2"
            >
              its page
            </a>
          ) : (
            "its page"
          )}
          .
        </p>
      )}
      {/* What the status line leaves out: why it stopped, or that an
          update was rolled back while it went on working. */}
      {plugin.error && !plugin.status_line.includes(plugin.error) && (
        <p className="text-muted-foreground">{plugin.error}</p>
      )}

      {plugin.update && (
        <div className="bg-muted/50 flex flex-col gap-2 rounded-xl p-3">
          <p className="font-medium">Update available: v{plugin.update.version}</p>
          {plugin.update.added.length > 0 && (
            <p className="text-muted-foreground">
              It asks for more: {plugin.update.added.join("; ")}.
            </p>
          )}
          <div>
            <Button
              size="sm"
              disabled={busy}
              onClick={() =>
                run(async () => setUpdate(await api.previewPluginUpdate(plugin.id)))
              }
            >
              Update…
            </Button>
          </div>
        </div>
      )}

      <PluginSettings
        key={`${plugin.id}-${plugin.updated_at}`}
        plugin={plugin}
        rooms={rooms}
        secrets={secrets}
        onSaved={() => {
          onChanged();
          // The restart a save causes lands a moment later; ask again
          // then, so the status says how the new settings went.
          setTimeout(onChanged, 2_000);
          onSecretsChanged?.();
        }}
      />

      {plugin.inbound && (
        <PluginInboundSection
          plugin={plugin}
          inbound={plugin.inbound}
          outcome={outcome}
          onChanged={onChanged}
        />
      )}

      {plugin.widgets.length > 0 && (
        <section aria-label="Widgets" className="flex flex-col gap-2">
          <h3 className="text-sm font-medium">Widgets</h3>
          {plugin.widgets.map((widget) => {
            const options = widgetPlaceOptions(rooms);
            const place = widgetPlace(plugin.id, widget, effective);
            // A room the house does not have is the front page, as the
            // dashboard draws it.
            const shown = options.some((o) => o.value === place) ? place : "front";
            return (
              <div key={widget.id} className="flex items-center justify-between gap-3">
                <span className="min-w-0 truncate">{widget.title}</span>
                <Select
                  items={options}
                  value={shown}
                  disabled={busy || !onPlaceWidget}
                  onValueChange={(next: string | null) => {
                    if (next) onPlaceWidget?.(widget.id, next);
                  }}
                >
                  <SelectTrigger aria-label={`Where ${widget.title} goes`} className="h-8 w-40">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {options.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            );
          })}
        </section>
      )}

      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() =>
            run(async () => {
              await api.setPluginEnabled(plugin.id, !plugin.enabled);
              onChanged();
            })
          }
        >
          {plugin.enabled ? "Turn off" : "Turn on"}
        </Button>
        {plugin.repo && (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() =>
              run(async () => {
                const checked = await api.checkPluginUpdate(plugin.id);
                if (!checked.update) setNote(`${plugin.version} is the newest release.`);
                onChanged();
              })
            }
          >
            <RefreshCw aria-hidden /> Check for updates
          </Button>
        )}
        {plugin.previous_version && (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() =>
              run(async () => {
                await api.rollBackPlugin(plugin.id);
                onChanged();
              })
            }
          >
            <RotateCcw aria-hidden /> Roll back to v{plugin.previous_version}
          </Button>
        )}
      </div>
      {note && <p className="text-muted-foreground">{note}</p>}
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}

      <PluginLog plugin={plugin.id} />

      <div className="flex flex-col gap-2 border-t pt-3">
        {confirmRemove ? (
          <>
            {/* Said before the click: none of it comes back. */}
            <p>
              Remove {plugin.name}? Its devices leave your rooms, its pages and widgets go,
              and its settings, saved keys and stored data are deleted. Rules that use its devices are kept, and show as
              broken.
            </p>
            <div className="flex gap-2">
              <Button
                variant="destructive"
                disabled={busy}
                onClick={() =>
                  run(async () => {
                    await api.removePlugin(plugin.id);
                    onRemoved();
                  })
                }
              >
                Remove
              </Button>
              <Button variant="ghost" disabled={busy} onClick={() => setConfirmRemove(false)}>
                Keep it
              </Button>
            </div>
          </>
        ) : (
          <div>
            <Button
              variant="ghost"
              className="text-muted-foreground"
              disabled={busy}
              onClick={() => setConfirmRemove(true)}
            >
              <Trash2 aria-hidden /> Remove add-on
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
