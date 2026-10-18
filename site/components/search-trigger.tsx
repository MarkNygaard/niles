"use client"

import { useSearchContext } from "fumadocs-ui/contexts/search"
import { SearchIcon } from "lucide-react"

/**
 * Opens fumadocs' search dialog from the site header.
 *
 * The dialog itself lives in the root provider, so this works on the landing
 * page too, not only inside the docs. The keyboard hint is fumadocs' own
 * `hotKey` descriptor rather than a hard-coded "⌘K" -- it already resolves to
 * Ctrl on Windows and Linux after mount, and reusing it means the hint cannot
 * drift from the shortcut that is actually bound.
 */
export function SearchTrigger() {
  const { enabled, hotKey, setOpenSearch } = useSearchContext()

  if (!enabled) return null

  return (
    <button
      type="button"
      onClick={() => setOpenSearch(true)}
      aria-label="Search docs"
      className="inline-flex h-8 items-center gap-2 rounded-md text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground max-sm:size-8 max-sm:justify-center sm:w-64 sm:border sm:border-border sm:bg-muted/50 sm:px-2.5"
    >
      <SearchIcon className="size-4 shrink-0" />
      <span className="hidden truncate sm:inline">Search docs…</span>
      <kbd className="ml-auto hidden items-center gap-1 rounded border border-border bg-background px-1.5 font-mono text-[0.6875rem] text-muted-foreground sm:inline-flex">
        {hotKey.map((key, i) => (
          // A real element per key, not a fragment: `display` renders bare text,
          // and adjacent text nodes collapse into one anonymous flex item, so
          // the gap would have nothing to separate.
          <span key={i}>{key.display}</span>
        ))}
      </kbd>
    </button>
  )
}
