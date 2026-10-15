import { AlertTriangle, Circle } from "lucide-react";
import type { SetupGap } from "@/lib/api";

export interface SetupChecklistProps {
  /** The gaps this page answers. */
  gaps: SetupGap[];
}

/**
 * What is still unanswered on this page.
 *
 * Niles starts with no config at all, which is what lets the app be the
 * way in — and means starting no longer proves it is set up. The server
 * knows what is missing and what each gap costs, so the page that fixes
 * it says so at the top, rather than leaving somebody to notice their
 * lights never respond.
 *
 * Nothing here when everything is answered, on purpose. A permanent
 * "all good" is a thing people stop reading, and then it is not there
 * when it changes.
 */
export function SetupChecklist({ gaps }: SetupChecklistProps) {
  if (gaps.length === 0) return null;
  const blocking = gaps.some((g) => g.severity === "blocking");

  return (
    <section
      aria-labelledby="setup-checklist"
      className={
        blocking
          ? "border-destructive/40 bg-destructive/5 rounded-xl border px-4 py-3"
          : "bg-muted/40 rounded-xl px-4 py-3"
      }
    >
      <h2 id="setup-checklist" className="text-sm font-medium">
        {blocking
          ? "Niles is missing something it needs"
          : gaps.length === 1
            ? "One thing is not set up yet"
            : `${gaps.length} things are not set up yet`}
      </h2>
      <ul className="text-muted-foreground mt-2 flex flex-col gap-1.5 text-sm">
        {/* Worst first, as the server sends them. A house that does not
            work is marked apart from a feature that is off: run
            together, neither reads as urgent. */}
        {gaps.map((gap) => (
          <li key={gap.path} className="flex gap-2">
            {gap.severity === "blocking" ? (
              <AlertTriangle
                aria-label="Needed"
                className="text-destructive mt-0.5 size-4 shrink-0"
              />
            ) : (
              <Circle aria-label="To do" className="mt-1 size-3 shrink-0" />
            )}
            <span>{gap.consequence}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
