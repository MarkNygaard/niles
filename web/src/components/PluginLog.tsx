import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { PluginLogLine } from "@/lib/api";
import { cn } from "@/lib/utils";

/** How many of the kept lines the panel shows, newest first. */
export const SHOWN = 200;

const LEVEL: Record<PluginLogLine["level"], string> = {
  debug: "text-muted-foreground",
  info: "text-foreground",
  warn: "text-foreground bg-warn/15",
  error: "text-destructive bg-destructive/10",
};

/** `14:03:27`, in the viewer's own clock. */
export function timeOf(at: string): string {
  return new Date(at).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

/**
 * A plugin's log on its panel: what it wrote, and what Niles noted
 * about it (started, not set up, failing), newest first. Asked for
 * again every few seconds while the panel is open, so a restart can be
 * watched as it happens.
 */
export function PluginLog({ plugin }: { plugin: string }) {
  const log = useQuery({
    queryKey: ["plugin-log", plugin],
    queryFn: () => api.pluginLog(plugin),
    refetchInterval: 5_000,
    retry: false,
  });
  const lines = (log.data ?? []).slice(-SHOWN).reverse();
  return (
    <section aria-label="Log" className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">Log</h3>
      {lines.length === 0 ? (
        <p className="text-muted-foreground text-xs">
          {log.isPending ? "Reading…" : "Nothing since Niles started."}
        </p>
      ) : (
        <ol className="bg-muted/40 max-h-64 overflow-y-auto rounded-lg p-2 font-mono text-[11px] leading-relaxed">
          {lines.map((line, n) => (
            <li
              key={`${line.at}-${n}`}
              data-level={line.level}
              className={cn("flex gap-2 rounded px-1", LEVEL[line.level])}
            >
              <time dateTime={line.at} className="text-muted-foreground shrink-0 tabular-nums">
                {timeOf(line.at)}
              </time>
              <span className="w-10 shrink-0 uppercase">{line.level}</span>
              <span className={cn("min-w-0 break-words", line.niles && "italic")}>
                {line.niles && <span className="text-muted-foreground not-italic">Niles: </span>}
                {line.message}
              </span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
