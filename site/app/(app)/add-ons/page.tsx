import type { Metadata } from "next"
import Link from "next/link"

import { AddonLibrary } from "@/components/addon-library"
import { SiteFooter } from "@/components/site-footer"
import { buildTimeLibrary } from "@/lib/addons-build"
import { siteConfig } from "@/lib/config"

export const dynamic = "force-static"

const description = `Add-ons other people wrote for ${siteConfig.name}: WebAssembly components installed from a GitHub release, each reaching only what the household approved.`

export const metadata: Metadata = {
  title: "Add-ons",
  description,
  alternates: { canonical: "/add-ons" },
  openGraph: { url: "/add-ons", title: "Add-ons", description, siteName: siteConfig.name },
}

export default async function AddonsPage() {
  const library = await buildTimeLibrary()

  return (
    <>
      <section className="mx-auto w-full max-w-6xl px-6 pt-16 pb-24">
        <h1 className="text-4xl font-semibold tracking-tight">Add-ons</h1>
        <p className="mt-4 max-w-2xl text-lg leading-relaxed text-muted-foreground">
          What other people wrote for {siteConfig.name}, from the add-on
          library. Each is a WebAssembly component in a GitHub release, and it
          runs in a sandbox with no database, no conversations and no other
          add-on&apos;s passwords. {siteConfig.name} shows exactly what an
          add-on may reach, in its own words, before you install it.
        </p>
        <p className="mt-3 max-w-2xl text-sm leading-relaxed text-muted-foreground">
          The same list is in {siteConfig.name} under Settings → Add-ons →
          Browse.{" "}
          <Link
            href="/docs/writing-add-ons"
            className="underline underline-offset-4 hover:text-foreground"
          >
            Writing add-ons
          </Link>{" "}
          is the guide for publishing your own.
        </p>

        <AddonLibrary initial={library} />
      </section>
      <SiteFooter />
    </>
  )
}
