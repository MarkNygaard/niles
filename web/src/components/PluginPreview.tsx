import { Puzzle } from "lucide-react";
import type { PluginPreview } from "@/lib/api";

/**
 * A plugin's mark: its own logo, or the generic one.
 *
 * Drawn as an image, never inlined into the page: the server rewrote
 * the SVG from an allowlist, and an `<img>` runs nothing even if that
 * ever let something through. `src` is the served logo
 * (`/plugins/{id}/logo`) or, before install, the SVG the preview
 * carries.
 */
export function PluginLogo({ src, svg }: { src?: string | null; svg?: string | null }) {
  const url = src ?? (svg ? `data:image/svg+xml;utf8,${encodeURIComponent(svg)}` : null);
  if (!url) {
    return (
      <span data-logo="generic" className="flex shrink-0">
        <Puzzle aria-hidden className="size-6" />
      </span>
    );
  }
  return (
    <span data-logo="plugin" className="flex shrink-0">
      <img src={url} alt="" className="size-6" />
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
