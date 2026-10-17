/**
 * A podcast's content, as crates/niles-artifacts/src/podcast.rs keeps
 * it: the topic it researched, its target length, the two hosts, the
 * script line by line, and the sources the script drew on.
 *
 * A podcast is made only by its job and never edited afterwards, so
 * this is read-only: the view shows it, Copy copies it, and nothing
 * here writes back.
 */

export interface Podcast {
  topic: string;
  /** The target length the script was written to. */
  minutes: number;
  hosts: [string, string];
  lines: { host: string; text: string }[];
  sources: { title: string; url: string }[];
}

/** The podcast in `content`, or null when it is not one. Podcasts are
    validated before they are saved, so a malformed one means a newer
    shape this app has not learned; the view says so rather than
    showing half of it. */
export function podcastOf(content: unknown): Podcast | null {
  if (!content || typeof content !== "object") return null;
  const c = content as Partial<Podcast>;
  if (
    typeof c.topic !== "string" ||
    typeof c.minutes !== "number" ||
    !Array.isArray(c.hosts) ||
    c.hosts.length !== 2 ||
    typeof c.hosts[0] !== "string" ||
    typeof c.hosts[1] !== "string" ||
    !Array.isArray(c.lines) ||
    !Array.isArray(c.sources)
  ) {
    return null;
  }
  if (
    !c.lines.every(
      (l) =>
        l !== null &&
        typeof l === "object" &&
        typeof (l as { host?: unknown }).host === "string" &&
        typeof (l as { text?: unknown }).text === "string",
    ) ||
    !c.sources.every(
      (s) =>
        s !== null &&
        typeof s === "object" &&
        typeof (s as { title?: unknown }).title === "string" &&
        typeof (s as { url?: unknown }).url === "string",
    )
  ) {
    return null;
  }
  return c as Podcast;
}

/** The podcast as text, for Copy: the topic, its length and hosts, the
    script, then the sources. */
export function podcastText(p: Podcast): string {
  return [
    p.topic,
    `${p.minutes} min · ${p.hosts[0]} and ${p.hosts[1]}`,
    "",
    ...p.lines.map((l) => `${l.host}: ${l.text}`),
    "",
    "Sources:",
    ...p.sources.map((s) => `- ${s.title} (${s.url})`),
  ].join("\n");
}
