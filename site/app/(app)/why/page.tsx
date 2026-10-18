import type { Metadata } from "next"
import { pages } from "@/.source/server"

import { SiteFooter } from "@/components/site-footer"
import { siteConfig } from "@/lib/config"
import { getMDXComponents } from "@/mdx-components"

export const dynamic = "force-static"

// The collection holds exactly one entry. Frontmatter is flattened onto it
// rather than nested under `data`.
function getPage() {
  const page = pages[0]
  if (!page) throw new Error("content/pages/why.mdx is missing")
  return page
}

export function generateMetadata(): Metadata {
  const page = getPage()
  return {
    title: { absolute: page.title },
    description: page.description,
    alternates: { canonical: "/why" },
    openGraph: {
      type: "article",
      url: "/why",
      title: page.title,
      description: page.description,
      siteName: siteConfig.name,
    },
  }
}

/**
 * Rendered outside the docs shell on purpose. This page is read by people
 * deciding whether to use the product, not by people already using it, so it
 * gets the landing page's chrome and none of the sidebar.
 */
export default function WhyPage() {
  const page = getPage()
  const MDX = page.body
  const canonical = new URL("/why", siteConfig.url).toString()

  const jsonLd = {
    "@context": "https://schema.org",
    "@type": "TechArticle",
    "@id": `${canonical}#article`,
    headline: page.title,
    description: page.description,
    url: canonical,
    isPartOf: { "@id": `${siteConfig.url}/#website` },
    inLanguage: "en",
  }

  return (
    <>
      <article className="mx-auto w-full max-w-3xl px-6 py-16">
        <script
          type="application/ld+json"
          // Trusted, locally constructed values -- no user input reaches this.
          dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
        />
        <h1 className="text-4xl font-semibold tracking-tight text-balance">
          {page.title}
        </h1>
        <p className="mt-4 text-lg leading-relaxed text-muted-foreground">
          {page.description}
        </p>
        <div className="prose mt-12">
          <MDX components={getMDXComponents()} />
        </div>
      </article>
      <SiteFooter />
    </>
  )
}
