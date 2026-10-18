/**
 * The add-on library: what Browse lists, and how an installed add-on's
 * panel says where it stands.
 */

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { LibraryEntry, PluginLibrary, PluginSummary } from "@/lib/api";

/**
 * The library, as Browse and the panels show it. Niles keeps the index
 * for six hours; the app asks Niles at most hourly. `enabled` lets a
 * panel without a repo stay quiet.
 */
export function useLibrary(enabled = true) {
  return useQuery({
    queryKey: ["plugin-library"],
    queryFn: () => api.pluginLibrary(),
    staleTime: 60 * 60 * 1000,
    retry: false,
    enabled,
  });
}

/** The same repo whatever the case, a trailing slash or `.git`. */
export function sameRepo(a: string, b: string): boolean {
  const tidy = (s: string) => s.trim().replace(/\/+$/, "").replace(/\.git$/i, "").toLowerCase();
  return tidy(a) === tidy(b);
}

/** The installed add-on that came from `entry`'s repo, when there is one. */
export function installedFrom(
  entry: LibraryEntry,
  plugins: PluginSummary[],
): PluginSummary | undefined {
  return plugins.find((p) => p.repo !== null && sameRepo(p.repo, entry.repo));
}

/** Whether `entry` matches `query`: its name, description, owner or repo
 * path. An empty query matches everything. */
export function entryMatches(entry: LibraryEntry, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const path = entry.repo.replace(/^https?:\/\/(www\.)?github\.com\//i, "");
  return (
    entry.name.toLowerCase().includes(q) ||
    entry.description.toLowerCase().includes(q) ||
    entry.owner.toLowerCase().includes(q) ||
    path.toLowerCase().includes(q)
  );
}

/** Browse's two shelves: Listed first, Unreviewed folded away. Removed
 * ones never appear. Each sorted by name, ignoring case. */
export function shelvesOf(
  library: PluginLibrary,
  query: string,
): { listed: LibraryEntry[]; unreviewed: LibraryEntry[] } {
  const sort = (entries: LibraryEntry[]) =>
    entries.sort((a, b) => a.name.localeCompare(b.name, "en", { sensitivity: "base" }));
  const matching = library.addons.filter((e) => entryMatches(e, query));
  return {
    listed: sort(matching.filter((e) => e.standing === "listed")),
    unreviewed: sort(matching.filter((e) => e.standing === "unreviewed")),
  };
}

/** `34 installs`, or `120 downloads` when it has no install count. */
export function countLine(entry: LibraryEntry): string {
  if (entry.installs !== null) {
    return `${entry.installs.toLocaleString("en")} install${entry.installs === 1 ? "" : "s"}`;
  }
  return `${entry.downloads.toLocaleString("en")} download${entry.downloads === 1 ? "" : "s"}`;
}

/** `1 star`, or `1,234 stars`. */
export function starsLine(n: number): string {
  return n === 1 ? "1 star" : `${n.toLocaleString("en")} stars`;
}

/**
 * Where an installed add-on stands in the library, for its panel.
 * `null` when it has no repo, the library isn't loaded, or its repo
 * isn't in the library (installed by address, or private): Unreviewed
 * means found by the topic, which those weren't. One that was taken off
 * the list says so, with the reason and that it keeps working.
 */
export function standingOf(
  plugin: PluginSummary,
  library: PluginLibrary | undefined,
): { label: string; note: string | null } | null {
  if (plugin.repo === null || library === undefined) return null;
  const repo = plugin.repo;
  const entry = library.addons.find((e) => sameRepo(e.repo, repo));
  if (entry === undefined) return null;
  if (entry.standing === "listed") return { label: "Listed", note: null };
  if (entry.standing === "unreviewed") return { label: "Unreviewed", note: null };
  if (entry.standing === "removed") {
    const reason = entry.removed_reason;
    const sentence =
      reason && !/[.!?]$/.test(reason) ? `${reason}.` : reason;
    return {
      label: "No longer listed",
      note: ["Taken off the add-on library's list.", sentence, "It keeps working."]
        .filter(Boolean)
        .join(" "),
    };
  }
  return null;
}
