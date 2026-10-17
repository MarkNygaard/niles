import { useState } from "react";
import { ExternalLink } from "lucide-react";
import type { KindViewProps } from "@/lib/artifacts";
import { podcastOf } from "@/lib/podcast";

/**
 * A podcast, open in Niles's panel: how long it runs and who hosts it,
 * the topic, the sources the script drew on, and the script itself,
 * folded away until it is asked for. Read-only — an episode is kept as
 * recorded, and a new one is made by asking, not by editing this one.
 */
export function PodcastView({ content }: KindViewProps) {
  const podcast = podcastOf(content);
  const [scriptOpen, setScriptOpen] = useState(false);

  if (!podcast) {
    return <p className="text-muted-foreground text-sm">This podcast can't be shown.</p>;
  }
  const [first, second] = podcast.hosts;

  return (
    <div className="flex flex-col gap-3">
      <div className="bg-muted/60 flex flex-col gap-1 rounded-xl px-3 py-2.5">
        <span className="text-muted-foreground text-sm">
          About {podcast.minutes} min · {first} and {second}
        </span>
        <span className="font-medium break-words">{podcast.topic}</span>
      </div>

      {podcast.sources.length > 0 && (
        <div className="flex flex-col gap-1">
          <span className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
            Sources
          </span>
          <ol className="flex flex-col gap-1">
            {podcast.sources.map((s, i) => {
              let host = s.url;
              try {
                host = new URL(s.url).host;
              } catch {
                // Kept verbatim: the address as the episode saved it.
              }
              return (
                <li key={`${s.url}-${i}`} className="text-sm">
                  <a
                    href={s.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="hover:underline"
                  >
                    {s.title}
                    <span className="text-muted-foreground"> ({host})</span>
                    <ExternalLink aria-hidden className="mx-1 inline size-3 align-[-0.125em]" />
                  </a>
                </li>
              );
            })}
          </ol>
        </div>
      )}

      <div className="flex flex-col gap-1">
        <button
          type="button"
          aria-expanded={scriptOpen}
          onClick={() => setScriptOpen((open) => !open)}
          className="text-muted-foreground hover:text-foreground self-start rounded-md px-1 text-xs font-medium tracking-wide uppercase focus-visible:outline-none"
        >
          Script {scriptOpen ? "▾" : "▸"}
        </button>
        {scriptOpen && (
          <div className="flex flex-col gap-2">
            {podcast.lines.map((line, i) => (
              <p key={i} className="text-sm">
                <strong>{line.host}</strong>: {line.text}
              </p>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
