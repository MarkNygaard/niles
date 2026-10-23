"use client"

import Link from "next/link"
import { useEffect, useState } from "react"

import { AddonCard, formatDay } from "@/components/addon-card"
import { readLibrary, type Library } from "@/lib/addons"
import { siteConfig } from "@/lib/config"

const MAX_INDEX = 8 * 1024 * 1024

type State =
  | { kind: "ready"; library: Library; fromBuild: boolean }
  | { kind: "loading" }
  | { kind: "failed" }

/**
 * The list. With the library from the build it renders at once, and every
 * add-on has its own page. Without one (the build could not reach the
 * index) it loads the index here, and those add-ons link to their repos.
 */
export function AddonLibrary({ initial }: { initial: Library | null }) {
  const [state, setState] = useState<State>(
    initial ? { kind: "ready", library: initial, fromBuild: true } : { kind: "loading" }
  )

  useEffect(() => {
    if (initial) return
    let cancelled = false
    ;(async () => {
      try {
        const response = await fetch(siteConfig.addons.index, { signal: AbortSignal.timeout(15_000) })
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        const text = await response.text()
        if (text.length > MAX_INDEX) throw new Error("too large")
        const library = readLibrary(JSON.parse(text))
        if (!library) throw new Error("not a format 1 index")
        if (!cancelled) setState({ kind: "ready", library, fromBuild: false })
      } catch {
        if (!cancelled) setState({ kind: "failed" })
      }
    })()
    return () => {
      cancelled = true
    }
  }, [initial])

  if (state.kind === "loading") {
    return <p className="mt-10 text-sm text-muted-foreground">Loading the add-on library…</p>
  }

  if (state.kind === "failed") {
    return (
      <p className="mt-10 max-w-2xl text-sm leading-relaxed text-muted-foreground">
        The add-on library could not be loaded. It is one file,{" "}
        <a
          href={siteConfig.addons.index}
          className="underline underline-offset-4 hover:text-foreground"
        >
          {siteConfig.addons.index}
        </a>
        , the same one {siteConfig.name} reads for Settings → Add-ons → Browse. Installing
        by a repository&apos;s address works without it.
      </p>
    )
  }

  const { library, fromBuild } = state
  const listed = library.addons.filter((a) => a.standing === "listed")
  const unreviewed = library.addons.filter((a) => a.standing === "unreviewed")

  if (library.addons.length === 0) {
    return (
      <div className="mt-10 rounded-lg border border-dashed border-border p-8 text-sm text-muted-foreground">
        <p className="text-base text-foreground">
          No add-ons yet. Publish one:{" "}
          <Link href="/docs/writing-add-ons" className="font-medium underline underline-offset-4">
            the author guide
          </Link>
          .
        </p>
        <p className="mt-2">
          A public repo with the GitHub topic{" "}
          <code className="rounded bg-muted px-1.5 py-0.5">{siteConfig.addons.topic}</code> and a
          release appears here within a day.{" "}
          <Link
            href="/docs/writing-add-ons/publishing#the-add-on-library"
            className="underline underline-offset-4 hover:text-foreground"
          >
            Publishing
          </Link>{" "}
          says how it gets listed.
        </p>
      </div>
    )
  }

  return (
    <>
      <section className="mt-10">
        <h2 className="text-xl font-semibold tracking-tight">Listed</h2>
        <p className="mt-2 max-w-2xl text-sm leading-relaxed text-muted-foreground">
          These passed {siteConfig.name}&apos;s checks when they were added to the library,
          and someone read what they ask for. That is not a review of their
          code, and later releases are not reviewed again.
        </p>
        {listed.length > 0 ? (
          <ul className="mt-6 grid gap-4 sm:grid-cols-2">
            {listed.map((addon) => (
              <AddonCard key={`${addon.owner}/${addon.repo}`} addon={addon} hasPage={fromBuild} />
            ))}
          </ul>
        ) : (
          <p className="mt-6 text-sm text-muted-foreground">None listed yet.</p>
        )}
      </section>

      {unreviewed.length > 0 ? (
        <details className="group mt-12">
          <summary className="cursor-pointer text-xl font-semibold tracking-tight select-none">
            Unreviewed ({unreviewed.length})
          </summary>
          <p className="mt-2 max-w-2xl text-sm leading-relaxed text-muted-foreground">
            Nobody has reviewed these. Their authors gave the repo the GitHub
            topic <code className="rounded bg-muted px-1.5 py-0.5">{siteConfig.addons.topic}</code>,
            and that is all.
          </p>
          <ul className="mt-6 grid gap-4 sm:grid-cols-2">
            {unreviewed.map((addon) => (
              <AddonCard key={`${addon.owner}/${addon.repo}`} addon={addon} hasPage={fromBuild} />
            ))}
          </ul>
        </details>
      ) : null}

      {library.generatedAt ? (
        <p className="mt-12 text-xs text-muted-foreground">
          The library was built on {formatDay(library.generatedAt)}.
        </p>
      ) : null}
    </>
  )
}
