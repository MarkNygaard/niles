/**
 * The product's public identity, in one place.
 *
 * Everything the site renders (page titles, nav, metadata, OG images, the
 * copy-paste install commands) reads from here. Nothing in `app/` should
 * hard-code the name; import `siteConfig` instead.
 */
export const siteConfig = {
  name: "Niles",
  /** The meta description and the OG card's subtitle. From ARCHITECTURE.md's "What Niles is". */
  description:
    "An AI-first home assistant that runs on the household's own hardware, with voice that answers in well under a second and a language model built in.",
  /**
   * Public origin: the base for canonical URLs, OG images, `sitemap.xml`,
   * `robots.txt` and `llms.txt`. CI sets `NEXT_PUBLIC_SITE_URL`; the
   * fallback is the same domain, so a local build publishes nothing wrong.
   */
  url: process.env.NEXT_PUBLIC_SITE_URL ?? "https://getniles.app",

  /** What appears in people's shells and manifests. */
  binary: "niles",
  envPrefix: "NILES_",
  image: "ghcr.io/marknygaard/niles",
  license: "MIT or Apache-2.0",

  links: {
    github: "https://github.com/MarkNygaard/niles",
    contributing: "https://github.com/MarkNygaard/niles/blob/main/CONTRIBUTING.md",
    codeOfConduct: "https://github.com/MarkNygaard/niles/blob/main/CODE_OF_CONDUCT.md",
    architecture: "https://github.com/MarkNygaard/niles/blob/main/ARCHITECTURE.md",
  },

  /** The add-on library: the registry repo and the one file it publishes. */
  addons: {
    index: "https://addons.getniles.app/index.json",
    registry: "https://github.com/MarkNygaard/niles-addons",
    topic: "niles-addon",
  },

  navItems: [
    { href: "/", label: "Home" },
    { href: "/why", label: "Why Niles" },
    { href: "/docs", label: "Docs" },
    { href: "/add-ons", label: "Add-ons" },
  ],
} as const

export type SiteConfig = typeof siteConfig
