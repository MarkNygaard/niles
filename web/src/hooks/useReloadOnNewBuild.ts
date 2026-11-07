import { useEffect } from "react";
import { noticeBuild } from "@/lib/update";

/** The default way back onto a new build. At module scope, so the
    identity is stable across renders: a per-render closure in an
    effect's deps re-runs the effect every render. */
const reloadPage = () => window.location.reload();

/** A page's script is the build that served it; when another build
    answers (an update, any redeploy) it loads that build's script,
    once. `undefined` means no answer yet and is ignored. */
export function useReloadOnNewBuild(
  commit: string | null | undefined,
  reload: () => void = reloadPage,
) {
  useEffect(() => {
    if (commit !== undefined) noticeBuild(commit, reload);
  }, [commit, reload]);
}
