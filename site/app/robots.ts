import type { MetadataRoute } from "next"

import { siteConfig } from "@/lib/config"

export const dynamic = "force-static"

/**
 * Everything is public documentation, so everything is crawlable.
 *
 * The AI crawlers are named explicitly rather than left to the wildcard. Several
 * of them are opt-*out* by default but are judged by operators on whether a site
 * states its position, and a few (Google-Extended, Applebot-Extended) only ever
 * read a directive addressed to them by name — a bare `User-agent: *` says
 * nothing to those. For a project that wants to be found by people asking an
 * assistant for a self-hosted home assistant, being citable is the point.
 */
const AI_CRAWLERS = [
  "GPTBot",
  "OAI-SearchBot",
  "ChatGPT-User",
  "ClaudeBot",
  "Claude-User",
  "Claude-SearchBot",
  "PerplexityBot",
  "Perplexity-User",
  "Google-Extended",
  "Applebot-Extended",
  "CCBot",
  "meta-externalagent",
  "Bytespider",
  "cohere-ai",
]

export default function robots(): MetadataRoute.Robots {
  return {
    rules: [
      { userAgent: "*", allow: "/" },
      ...AI_CRAWLERS.map((userAgent) => ({ userAgent, allow: "/" })),
    ],
    sitemap: new URL("/sitemap.xml", siteConfig.url).toString(),
    host: siteConfig.url,
  }
}
