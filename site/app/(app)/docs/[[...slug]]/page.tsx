import { notFound } from "next/navigation"
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
} from "fumadocs-ui/page"
import type { Metadata } from "next"

import { siteConfig } from "@/lib/config"
import { source } from "@/lib/source"
import { getMDXComponents } from "@/mdx-components"

type PageProps = { params: Promise<{ slug?: string[] }> }

export default async function Page({ params }: PageProps) {
  const { slug } = await params
  const page = source.getPage(slug)
  if (!page) notFound()

  const MDX = page.data.body
  const canonical = new URL(page.url, siteConfig.url).toString()

  const jsonLd = {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "TechArticle",
        "@id": `${canonical}#article`,
        headline: page.data.title,
        description: page.data.description,
        url: canonical,
        isPartOf: { "@id": `${siteConfig.url}/#website` },
        inLanguage: "en",
      },
      {
        "@type": "BreadcrumbList",
        itemListElement: [
          { "@type": "ListItem", position: 1, name: "Documentation", item: `${siteConfig.url}/docs` },
          { "@type": "ListItem", position: 2, name: page.data.title, item: canonical },
        ],
      },
    ],
  }

  return (
    <DocsPage toc={page.data.toc} full={page.data.full}>
      <script
        type="application/ld+json"
        // Trusted, locally constructed values -- no user input reaches this.
        dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
      />
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      <DocsBody>
        <MDX components={getMDXComponents()} />
      </DocsBody>
    </DocsPage>
  )
}

export function generateStaticParams() {
  return source.generateParams()
}

export async function generateMetadata({
  params,
}: PageProps): Promise<Metadata> {
  const { slug } = await params
  const page = source.getPage(slug)
  if (!page) notFound()

  const description = page.data.description ?? siteConfig.description

  return {
    title: page.data.title,
    description,
    // Every page needs its own canonical: without one, a page reachable at both
    // `/docs/x` and `/docs/x/` is two documents competing with each other.
    alternates: { canonical: page.url },
    openGraph: {
      type: "article",
      url: page.url,
      title: page.data.title,
      description,
      siteName: siteConfig.name,
    },
  }
}
