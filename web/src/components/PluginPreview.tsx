import { useQuery } from "@tanstack/react-query";
import { Puzzle } from "lucide-react";
import { api } from "@/lib/api";
import type { PluginPreview, PluginSummary } from "@/lib/api";
import { svgToReact } from "@/lib/svg";

/**
 * A plugin's mark: its own logo, or the generic one.
 *
 * Drawn inline, so its `currentColor` is the text colour around it, in
 * dark mode too, like the built-in marks. Never inlined as markup: the
 * SVG Niles sanitised on install is fetched as text and rebuilt by
 * `svgToReact` from the same allowlist, as React elements, so nothing
 * in it can run. Anything outside that list draws the generic mark.
 *
 * Give it the installed `plugin` (its logo is fetched from
 * `/plugins/{id}/logo`), or, before install, the `svg` the preview
 * carries.
 */
export function PluginLogo({
  plugin,
  svg,
}: {
  plugin?: Pick<PluginSummary, "id" | "logo" | "updated_at">;
  svg?: string | null;
}) {
  if (plugin?.logo) return <InstalledLogo plugin={plugin} />;
  return <LogoMark svg={svg ?? null} />;
}

function InstalledLogo({ plugin }: { plugin: Pick<PluginSummary, "id" | "updated_at"> }) {
  // Keyed by when it was last updated: an update can bring a new logo,
  // and nothing else changes it.
  const logo = useQuery({
    queryKey: ["plugin-logo", plugin.id, plugin.updated_at],
    queryFn: () => api.pluginLogo(plugin.id),
    staleTime: Infinity,
    retry: false,
  });
  return <LogoMark svg={logo.data ?? null} />;
}

function LogoMark({ svg }: { svg: string | null }) {
  const drawn = svg ? svgToReact(svg, "size-6") : null;
  if (!drawn) {
    return (
      <span data-logo="generic" className="flex shrink-0">
        <Puzzle aria-hidden className="size-6" />
      </span>
    );
  }
  return (
    <span data-logo="plugin" className="flex shrink-0">
      {drawn}
    </span>
  );
}

/** `github.com/jane/niles-spotprice`, for a link's text. */
export function shortRepo(url: string): string {
  return url.replace(/^https?:\/\//, "");
}

/**
 * The approval screen: what is being installed, from where, and what it
 * may do, in the words the household is asked to agree to. For an update
 * the difference comes first, since that is what is being approved.
 */
export function PluginPreviewView({ preview }: { preview: PluginPreview }) {
  const updating = preview.current_version !== null;
  const asks = preview.settings.map((s) => (s.required ? `${s.label} (required)` : s.label));
  return (
    <div className="flex flex-col gap-4 text-sm">
      <div className="flex items-start gap-3">
        <PluginLogo svg={preview.logo} />
        <div className="min-w-0">
          <p className="font-medium">
            {preview.name}{" "}
            <span className="text-muted-foreground font-normal">v{preview.version}</span>
          </p>
          <p className="text-muted-foreground text-xs">
            By {preview.authors.join(", ")} ·{" "}
            <a
              href={preview.repo}
              target="_blank"
              rel="noopener noreferrer"
              className="underline underline-offset-2"
            >
              {shortRepo(preview.repo)}
            </a>
          </p>
        </div>
      </div>
      <p>{preview.description}</p>

      {updating && (
        <section aria-label="What changes" className="flex flex-col gap-2">
          <p>
            From v{preview.current_version} to v{preview.version}.
          </p>
          {preview.added.length === 0 && preview.removed.length === 0 ? (
            <p className="text-muted-foreground">What it may do does not change.</p>
          ) : (
            <>
              {preview.added.length > 0 && (
                <div>
                  <p className="font-medium">It would also be able to</p>
                  <ul className="list-disc pl-5">
                    {preview.added.map((line) => (
                      <li key={line}>{line}</li>
                    ))}
                  </ul>
                </div>
              )}
              {preview.removed.length > 0 && (
                <div>
                  <p className="font-medium">It would no longer</p>
                  <ul className="text-muted-foreground list-disc pl-5">
                    {preview.removed.map((line) => (
                      <li key={line}>{line}</li>
                    ))}
                  </ul>
                </div>
              )}
            </>
          )}
          {preview.notes.trim() && (
            <p className="text-muted-foreground line-clamp-6 whitespace-pre-line">
              {preview.notes.trim()}
            </p>
          )}
        </section>
      )}

      <section aria-label="Permissions" className="flex flex-col gap-1">
        <p className="font-medium">{updating ? "Everything it may do" : "It will be able to"}</p>
        {preview.permissions.length > 0 ? (
          <ul className="list-disc pl-5">
            {preview.permissions.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        ) : (
          <p className="text-muted-foreground">Nothing beyond keeping its own settings and data.</p>
        )}
        <p className="text-muted-foreground">{preview.cannot}</p>
        {preview.tools.length > 0 && (
          <details className="text-muted-foreground">
            <summary className="cursor-pointer">What its tools do</summary>
            <ul className="mt-1 flex flex-col gap-1 pl-4">
              {preview.tools.map((tool) => (
                <li key={tool.name}>
                  <span className="text-foreground font-mono text-xs">{tool.name}</span>:{" "}
                  {tool.description}
                </li>
              ))}
            </ul>
          </details>
        )}
      </section>

      {asks.length > 0 && (
        <p className="text-muted-foreground">It will ask for: {asks.join(", ")}.</p>
      )}
      {preview.warnings.length > 0 && (
        <ul className="text-muted-foreground flex flex-col gap-1 text-xs">
          {preview.warnings.map((warning) => (
            <li key={warning}>{warning}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
