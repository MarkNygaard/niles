import { useState } from "react";
import { ChevronRight, RefreshCw, RotateCcw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PluginLogo, PluginPreviewView, shortRepo } from "@/components/PluginPreview";
import { api } from "@/lib/api";
import type { PluginPreview, PluginSummary } from "@/lib/api";
import type { Status } from "@/lib/integrations";
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
      <PluginLogo src={plugin.logo} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{plugin.name}</span>
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
 * An installed plugin's panel: where it came from, and switching it
 * off, updating, rolling back and removing it. Its settings form is a
 * section to come above the actions; the actions do not depend on it.
 */
export function PluginPanel({
  plugin,
  onChanged,
  onRemoved,
}: {
  plugin: PluginSummary;
  /** Something changed; ask for the list again. */
  onChanged: () => void;
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
        <dd>{plugin.status_line}</dd>
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

      <div className="flex flex-col gap-2 border-t pt-3">
        {confirmRemove ? (
          <>
            {/* Said before the click: none of it comes back. */}
            <p>
              Remove {plugin.name}? Its devices leave your rooms, and its settings, saved keys
              and stored data are deleted. Rules that use its devices are kept, and show as
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
              <Trash2 aria-hidden /> Remove {plugin.name}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
