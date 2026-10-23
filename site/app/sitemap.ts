import type { MetadataRoute } from "next"

import { pagePath } from "@/lib/addons"
import { buildTimeLibrary } from "@/lib/addons-build"
import { siteConfig } from "@/lib/config"
import { source } from "@/lib/source"

// Static export: emitted once at build time.
export const dynamic = "force-static"

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const at = (p: string) => new URL(p, siteConfig.url).toString()

  const docs = source.getPages().map((page) => ({
    url: at(page.url),
    changeFrequency: "weekly" as const,
    priority: 0.8,
  }))

  // Only the add-ons the build knew have pages.
  const library = await buildTimeLibrary()
  const addons = (library?.addons ?? []).map((addon) => ({
    url: at(pagePath(addon)),
    changeFrequency: "weekly" as const,
    priority: 0.5,
  }))

  return [
    { url: siteConfig.url, changeFrequency: "weekly", priority: 1 },
    // Standalone, so they are not in the docs tree the loop above walks.
    { url: at("/why"), changeFrequency: "monthly", priority: 0.9 },
    { url: at("/add-ons"), changeFrequency: "daily", priority: 0.7 },
    ...docs,
    ...addons,
  ]
}
