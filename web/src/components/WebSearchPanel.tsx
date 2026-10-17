import { useCallback, useEffect, useState } from "react";
import { CommitField } from "@/components/CommitField";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import type { WebSearchOutcome, WebSearchTest } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface WebSearchPanelProps {
  /** The SearXNG address as the config has it, empty until one is typed. */
  address: string;
  saving?: boolean;
  onAddress: (address: string) => void;
}

/** Where a test is: not run, running, answered, or the request failed. */
export type Probe =
  | { state: "idle" }
  | { state: "running" }
  | { state: "done"; result: WebSearchTest }
  | { state: "failed"; message: string };

/**
 * The address as SearXNG answers searches at it: a scheme when none was
 * typed, and its search path when only the host was. A pasted
 * `search.example.com` is the commonest way to get it wrong.
 */
export function searxngAddress(typed: string): string {
  const trimmed = typed.trim();
  if (!trimmed) return "";
  const withScheme = /^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
  try {
    const url = new URL(withScheme);
    if (url.pathname === "" || url.pathname === "/") {
      url.pathname = "/search";
      return url.toString();
    }
  } catch {
    // Not one a browser reads: the server says what is wrong with it.
  }
  return withScheme;
}

export type Tone = "off" | "pending" | "ok" | "bad";

/** The status line: not added, in use, or not reachable and why. */
export function statusLine(address: string, probe: Probe): { tone: Tone; text: string } {
  if (!address.trim()) return { tone: "off", text: "Not added." };
  switch (probe.state) {
    case "idle":
    case "running":
      return { tone: "pending", text: "Checking…" };
    case "failed":
      return { tone: "bad", text: `Not reachable: ${probe.message}` };
    case "done": {
      const web = probe.result.web;
      if (!probe.result.configured || !web) return { tone: "off", text: "Not added." };
      return web.ok
        ? { tone: "ok", text: "In use: Claude and ChatGPT search the web through it." }
        : { tone: "bad", text: `Not reachable: ${web.error ?? "it did not answer."}` };
    }
  }
}

const TONE_CLASS: Record<Tone, string> = {
  off: "text-muted-foreground",
  pending: "text-muted-foreground",
  ok: "",
  bad: "text-destructive",
};

function count(n: number): string {
  return n === 1 ? "1 result" : `${n} results`;
}

/** What each of the two searches found, a line each. */
export function testLines(result: WebSearchTest): string[] {
  const line = (label: string, outcome: WebSearchOutcome | null, none: string) => {
    if (!outcome) return null;
    if (!outcome.ok) return `${label}: ${outcome.error ?? "it did not answer."}`;
    return outcome.results > 0 ? `${label}: ${count(outcome.results)}.` : `${label}: ${none}`;
  };
  return [
    line("Web", result.web, "no results. SearXNG answered, but its engines found nothing."),
    line(
      "Pictures",
      result.images,
      "none. This SearXNG has no image engines on, and pictures for artifacts need them: switch one on in its settings (Wikimedia Commons or Bing Images, say).",
    ),
  ].filter((l): l is string => l !== null);
}

/**
 * Web search: the house's own SearXNG, for both assistants.
 *
 * Without it Claude searches with Claude Code's own search, and ChatGPT
 * cannot search at all; with it, both search through it and Claude's
 * own is off. The address is read for every search, so changing it or
 * removing the add-on takes hold without a restart.
 *
 * Tested as it opens and on the button, through the server: what the
 * browser can reach says nothing about what Niles can.
 */
export function WebSearchPanel({ address, saving, onAddress }: WebSearchPanelProps) {
  const [probe, setProbe] = useState<Probe>({ state: "idle" });

  const run = useCallback(() => {
    setProbe({ state: "running" });
    api
      .testWebSearch()
      .then((result) => setProbe({ state: "done", result }))
      .catch((e: unknown) =>
        setProbe({ state: "failed", message: e instanceof Error ? e.message : String(e) }),
      );
  }, []);

  // Again for a new address: the last one's answer says nothing of it.
  useEffect(() => {
    if (address.trim()) run();
  }, [address, run]);

  const added = address.trim() !== "";
  const status = statusLine(address, probe);

  return (
    <div className="flex flex-col gap-4">
      {added ? (
        <p className="text-muted-foreground text-sm">
          Claude and ChatGPT search the web through your SearXNG, and find pictures for what
          they save with its image engines. Claude's own search is off while it is here;
          remove it and Claude goes back to its own.
        </p>
      ) : (
        <ul className="text-muted-foreground list-disc space-y-1 pl-4 text-sm">
          <li>Claude searches the web with its own built-in search.</li>
          <li>ChatGPT can't search the web without SearXNG.</li>
          <li>Pictures for artifacts need SearXNG.</li>
        </ul>
      )}

      <CommitField
        label="SearXNG address"
        value={address}
        placeholder="https://search.example.com/search"
        disabled={saving}
        onCommit={(typed) => onAddress(searxngAddress(typed))}
      />

      {added && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between gap-3">
            <p role="status" className={cn("min-w-0 text-sm", TONE_CLASS[status.tone])}>
              {status.text}
            </p>
            <Button
              variant="outline"
              size="sm"
              disabled={saving || probe.state === "running"}
              onClick={run}
            >
              {probe.state === "running" ? "Testing…" : "Test"}
            </Button>
          </div>
          {probe.state === "done" && (
            <ul aria-label="Test results" className="text-muted-foreground space-y-1 text-xs">
              {testLines(probe.result).map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          )}
        </div>
      )}

      <p className="text-muted-foreground text-xs">
        Niles asks SearXNG for JSON, which is off in a new install: in its settings.yml, add
        json under search → formats. Pages a search finds are read by Niles itself, on the
        public internet only.
      </p>
    </div>
  );
}
