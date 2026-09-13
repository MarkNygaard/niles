import { AlertTriangle, Info } from "lucide-react";
import type { SetupReport } from "@/lib/api";

export interface SetupBannerProps {
  report: SetupReport;
}

/**
 * What is still unanswered.
 *
 * Niles starts with no config at all now, which is what lets the app be
 * the way in — and means starting no longer proves it is set up. This
 * is the other half of that trade: the server already knows what is
 * missing and what each gap costs, so the page says it rather than
 * leaving somebody to notice their lights never respond.
 *
 * Nothing here when everything is answered, on purpose. A permanent
 * "all good" banner is a thing people stop reading, and then it is not
 * there when it changes.
 */
export function SetupBanner({ report }: SetupBannerProps) {
  if (report.set_up || report.gaps.length === 0) return null;

  const blocking = report.gaps.filter((g) => g.severity === "blocking");
  const rest = report.gaps.filter((g) => g.severity !== "blocking");

  return (
    <div className="flex flex-col gap-3">
      {blocking.length > 0 && (
        <section className="border-destructive/40 bg-destructive/5 flex gap-3 rounded-xl border px-4 py-3">
          <AlertTriangle
            aria-hidden
            className="text-destructive mt-0.5 size-4 shrink-0"
          />
          <div className="min-w-0">
            <h2 className="text-sm font-medium">
              {blocking.length === 1
                ? "Niles is missing something it needs"
                : `Niles is missing ${blocking.length} things it needs`}
            </h2>
            <ul className="text-muted-foreground mt-1 flex flex-col gap-1 text-sm">
              {blocking.map((gap) => (
                <li key={gap.path}>
                  {gap.consequence}{" "}
                  <code className="text-xs">{gap.path}</code>
                </li>
              ))}
            </ul>
          </div>
        </section>
      )}

      {rest.length > 0 && (
        <section className="bg-muted/40 flex gap-3 rounded-xl px-4 py-3">
          <Info
            aria-hidden
            className="text-muted-foreground mt-0.5 size-4 shrink-0"
          />
          <div className="min-w-0">
            {/* Separate from the blocking list, and quieter. These are
                features that are off, not a house that does not work,
                and running them together would make neither urgent. */}
            <h2 className="text-sm font-medium">
              {rest.length === 1
                ? "One thing is not set up yet"
                : `${rest.length} things are not set up yet`}
            </h2>
            <ul className="text-muted-foreground mt-1 flex flex-col gap-1 text-sm">
              {rest.map((gap) => (
                <li key={gap.path}>
                  {gap.consequence} <code className="text-xs">{gap.path}</code>
                </li>
              ))}
            </ul>
          </div>
        </section>
      )}
    </div>
  );
}
