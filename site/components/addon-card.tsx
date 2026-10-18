import Link from "next/link"

import { ownerUrl, pagePath, repoUrl, type Addon } from "@/lib/addons"

const number = new Intl.NumberFormat("en")
// UTC and a fixed locale, so the build and the browser print the same day.
const day = new Intl.DateTimeFormat("en-GB", {
  day: "numeric",
  month: "short",
  year: "numeric",
  timeZone: "UTC",
})

export function plural(n: number, one: string, many: string) {
  return `${number.format(n)} ${n === 1 ? one : many}`
}

export function formatDay(iso: string) {
  return day.format(new Date(iso))
}

/** Installs when the add-on counts them, otherwise downloads, labelled as such. */
export function usage(addon: Addon) {
  return addon.installs !== null
    ? plural(addon.installs, "install", "installs")
    : plural(addon.downloads, "download", "downloads")
}

export function AddonLogo({ addon, size }: { addon: Addon; size: number }) {
  if (addon.logo) {
    return (
      // A data: URI in an <img>: the SVG renders as a picture, with no
      // script and no requests, whatever the file says.
      <img
        src={addon.logo}
        alt=""
        width={size}
        height={size}
        className="shrink-0 rounded-md"
        style={{ width: size, height: size }}
      />
    )
  }
  return (
    <span
      aria-hidden
      className="inline-flex shrink-0 items-center justify-center rounded-md bg-muted font-medium text-muted-foreground"
      style={{ width: size, height: size }}
    >
      {Array.from(addon.name)[0]?.toUpperCase() ?? "?"}
    </span>
  )
}

/**
 * One add-on in the list. `hasPage` says whether the build made a page for
 * it; one the browser found later links to its repo instead.
 */
export function AddonCard({ addon, hasPage }: { addon: Addon; hasPage: boolean }) {
  const href = hasPage ? pagePath(addon) : repoUrl(addon)
  const meta = [
    addon.version ? `v${addon.version}` : null,
    addon.publishedAt ? formatDay(addon.publishedAt) : null,
  ].filter(Boolean)

  return (
    <li className="flex gap-4 rounded-lg border border-border bg-card p-5">
      <AddonLogo addon={addon} size={40} />
      <div className="min-w-0 flex-1">
        <h3 className="font-medium">
          {hasPage ? (
            <Link href={href} className="hover:underline hover:underline-offset-4">
              {addon.name}
            </Link>
          ) : (
            <a href={href} target="_blank" rel="noreferrer" className="hover:underline hover:underline-offset-4">
              {addon.name}
            </a>
          )}
        </h3>
        <p className="mt-0.5 text-xs text-muted-foreground">
          by{" "}
          <a
            href={ownerUrl(addon)}
            target="_blank"
            rel="noreferrer"
            className="underline underline-offset-4 hover:text-foreground"
          >
            {addon.owner}
          </a>
          {meta.length > 0 ? ` · ${meta.join(" · ")}` : null}
        </p>
        {addon.description ? (
          <p className="mt-2 text-sm leading-relaxed text-muted-foreground break-words">{addon.description}</p>
        ) : null}
        <p className="mt-3 text-xs text-muted-foreground">
          {usage(addon)} · {plural(addon.stars, "star", "stars")}
        </p>
      </div>
    </li>
  )
}
