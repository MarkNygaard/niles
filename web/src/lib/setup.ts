import type { SetupGap, SetupReport } from "@/lib/api";

/**
 * The Settings page that answers a gap, by the config path it names.
 *
 * Each gap is shown once, at the top of the page where it is fixed,
 * rather than in a banner above every page: a list of what is missing
 * is only useful beside the thing that fills it in.
 */
export function pageOfGap(path: string): string | null {
  const section = path.split(".")[0];
  switch (section) {
    case "mqtt":
      return "devices";
    case "stt":
    case "llm":
      return "voice";
    case "home":
      return "home";
    default:
      return null;
  }
}

/** The gaps a page answers, worst first as the server sent them. */
export function gapsFor(report: SetupReport | undefined, page: string): SetupGap[] {
  if (!report) return [];
  return report.gaps.filter((gap) => pageOfGap(gap.path) === page);
}

/** How many gaps each page answers, for the count beside it in the list. */
export function todoByPage(report: SetupReport | undefined): Record<string, number> {
  const todo: Record<string, number> = {};
  for (const gap of report?.gaps ?? []) {
    const page = pageOfGap(gap.path);
    if (page) todo[page] = (todo[page] ?? 0) + 1;
  }
  return todo;
}
