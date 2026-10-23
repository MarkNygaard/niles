import type { Metadata } from "next"
import Link from "next/link"
import { notFound } from "next/navigation"

import { AddonLogo, formatDay, plural, usage } from "@/components/addon-card"
import { CopyButton } from "@/components/copy-button"
import { SiteFooter } from "@/components/site-footer"
import { ownerUrl, pagePath, repoUrl, type Addon } from "@/lib/addons"
import { buildTimeLibrary } from "@/lib/addons-build"
import { siteConfig } from "@/lib/config"

type PageProps = { params: Promise<{ owner: string; name: string }> }

export const dynamic = "force-static"
// Only the add-ons the build knew; any other address is the site's 404.
export const dynamicParams = false

/**
 * A static export needs at least one path for a dynamic route, and the
 * library may be empty or out of reach. This placeholder renders as a 404
 * and is linked from nowhere.
 */
const PLACEHOLDER = { owner: "_", name: "_" }

export async function generateStaticParams() {
  const library = await buildTimeLibrary()
  const params = (library?.addons ?? []).map((a) => ({ owner: a.owner, name: a.repo }))
  return params.length > 0 ? params : [PLACEHOLDER]
}

async function find(params: PageProps["params"]): Promise<Addon | undefined> {
  const { owner, name } = await params
  const library = await buildTimeLibrary()
  return library?.addons.find((a) => a.owner === owner && a.repo === name)
}

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const addon = await find(params)
  if (!addon) return { title: "Not found", robots: { index: false } }
  const description = addon.description || `${addon.name}, an add-on for ${siteConfig.name}.`
  return {
    title: `${addon.name}, an add-on`,
    description,
    alternates: { canonical: pagePath(addon) },
    openGraph: { url: pagePath(addon), title: addon.name, description, siteName: siteConfig.name },
  }
}

export default async function AddonPage({ params }: PageProps) {
  const addon = await find(params)
  if (!addon) notFound()

  const address = repoUrl(addon)
  const facts = [
    addon.version ? `Version ${addon.version}` : null,
    addon.publishedAt ? `released ${formatDay(addon.publishedAt)}` : null,
    usage(addon),
    plural(addon.stars, "star", "stars"),
  ].filter(Boolean)

  return (
    <>
      <article className="mx-auto w-full max-w-3xl px-6 pt-16 pb-24">
        <p className="text-sm">
          <Link href="/add-ons" className="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline">
            Add-ons
          </Link>
        </p>

        <div className="mt-6 flex items-start gap-5">
          <AddonLogo addon={addon} size={56} />
          <div className="min-w-0">
            <h1 className="text-3xl font-semibold tracking-tight break-words">{addon.name}</h1>
            <p className="mt-1 text-sm text-muted-foreground">
              by{" "}
              <a
                href={ownerUrl(addon)}
                target="_blank"
                rel="noreferrer"
                className="underline underline-offset-4 hover:text-foreground"
              >
                {addon.owner}
              </a>
            </p>
          </div>
        </div>

        {addon.description ? (
          <p className="mt-6 text-lg leading-relaxed text-muted-foreground break-words">
            {addon.description}
          </p>
        ) : null}

        <p className="mt-4 text-sm text-muted-foreground">{facts.join(" · ")}</p>

        <p className="mt-4 max-w-2xl rounded-md border border-border bg-card px-4 py-3 text-sm leading-relaxed">
          {addon.standing === "listed"
            ? `Listed. It passed ${siteConfig.name}'s checks when it was added to the library, and someone read what it asks for. That is not a review of its code.`
            : "Unreviewed. Nobody has reviewed it. Its author gave the repo the library's GitHub topic, and that is all."}
        </p>

        <section className="mt-10">
          <h2 className="text-xl font-semibold tracking-tight">Install it</h2>
          <div className="mt-4 flex max-w-xl items-center gap-2 rounded-lg border border-border bg-card py-1 pr-1 pl-4">
            <code className="min-w-0 flex-1 truncate font-mono text-sm">{address}</code>
            <CopyButton text={address} label="Copy the address" />
          </div>
          <ol className="mt-5 list-decimal space-y-1.5 pl-5 text-sm leading-relaxed text-muted-foreground">
            <li>In {siteConfig.name}, open Settings → Add-ons.</li>
            <li>On the Available tab, press Add next to Add-on from GitHub.</li>
            <li>Paste the address and press Preview.</li>
          </ol>
          <p className="mt-4 max-w-2xl text-sm leading-relaxed text-muted-foreground">
            Nothing is installed until you press Install. {siteConfig.name} first
            shows exactly what the add-on may reach, in its own words, and the
            same add-on is on the Browse tab too.
          </p>
        </section>

        {addon.readme ? (
          <section className="mt-10">
            <h2 className="text-xl font-semibold tracking-tight">From its README</h2>
            <p className="mt-4 text-sm leading-relaxed whitespace-pre-line text-muted-foreground break-words">
              {addon.readme}
            </p>
          </section>
        ) : null}

        <section className="mt-10 flex flex-wrap gap-3 text-sm">
          <a
            href={address}
            target="_blank"
            rel="noreferrer"
            className="inline-flex h-9 items-center rounded-md border border-border px-4 font-medium transition-colors hover:bg-accent"
          >
            Repository on GitHub
          </a>
          <a
            href={`${address}/releases`}
            target="_blank"
            rel="noreferrer"
            className="inline-flex h-9 items-center rounded-md border border-border px-4 font-medium transition-colors hover:bg-accent"
          >
            Releases
          </a>
        </section>

        <p className="mt-10 text-sm text-muted-foreground">
          Writing your own?{" "}
          <Link href="/docs/writing-add-ons" className="underline underline-offset-4 hover:text-foreground">
            The author guide
          </Link>{" "}
          starts from nothing.
        </p>
      </article>
      <SiteFooter />
    </>
  )
}
