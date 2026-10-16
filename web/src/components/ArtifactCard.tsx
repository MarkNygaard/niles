import { LoaderCircle, TriangleAlert } from "lucide-react";
import type { ArtifactRef } from "@/lib/api";
import { kindOf } from "@/lib/artifacts";
import { cn } from "@/lib/utils";

/** How many of the preview's lines a card shows. */
const LINES = 3;

/**
 * An artifact under the reply that made or changed it: what it is, and
 * its first lines. Tapping it opens it in Niles's panel.
 */
export function ArtifactCard({
  card,
  onOpen,
}: {
  card: ArtifactRef;
  onOpen?: (id: number) => void;
}) {
  const kind = kindOf(card.kind);
  return (
    <button
      type="button"
      onClick={() => onOpen?.(card.id)}
      aria-label={`Open ${card.title}`}
      className={cn(
        "bg-card border-border flex w-full max-w-[85%] items-start gap-2.5 self-start rounded-2xl border px-3.5 py-2.5 text-left transition-colors",
        "hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
      )}
    >
      <kind.icon aria-hidden className="text-muted-foreground mt-0.5 size-4 shrink-0" />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-sm font-medium">{card.title}</span>
        {card.summary && (
          <span className="text-muted-foreground truncate text-xs">{card.summary}</span>
        )}
        {card.status === "working" ? (
          <span className="text-muted-foreground flex items-center gap-1 text-xs">
            <LoaderCircle aria-hidden className="size-3 animate-spin" />
            being written…
          </span>
        ) : card.status === "failed" ? (
          <span className="text-muted-foreground flex items-center gap-1 text-xs">
            <TriangleAlert aria-hidden className="text-destructive size-3" />
            didn't finish
          </span>
        ) : (
          card.preview.lines.length > 0 && (
            <span
              data-slot="artifact-preview"
              className="text-muted-foreground mt-1 flex flex-col border-l-2 pl-2 text-xs"
            >
              {card.preview.lines.slice(0, LINES).map((line, i) => (
                <span
                  key={i}
                  className={cn(
                    "truncate",
                    line.style === "heading" && "text-foreground font-medium",
                  )}
                >
                  {line.style === "bullet" && "• "}
                  {line.style === "check" && (line.checked ? "☑ " : "☐ ")}
                  {line.text}
                </span>
              ))}
            </span>
          )
        )}
      </span>
    </button>
  );
}
