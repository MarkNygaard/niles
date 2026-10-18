import { siteConfig } from "@/lib/config"
import { docsSections } from "@/lib/docs-order"

export const dynamic = "force-static"

/**
 * https://llmstxt.org — a curated index for language models, so an assistant
 * reading this project does not have to infer its shape from rendered HTML and
 * navigation chrome.
 *
 * This file is the map; `/llms-full.txt` is the territory.
 */
export function GET() {
  const body = docsSections()
    .map((section) => {
      const links = section.pages
        .map((page) => {
          const url = new URL(page.url, siteConfig.url).toString()
          const description = page.data.description
          return `- [${page.data.title}](${url})${description ? `: ${description}` : ""}`
        })
        .join("\n")
      return `## ${section.title}\n\n${links}`
    })
    .join("\n\n")

  const at = (p: string) => new URL(p, siteConfig.url).toString()

  const text = `# ${siteConfig.name}

> ${siteConfig.description}

${siteConfig.name} is open source (${siteConfig.license}) and runs on the
household's own Kubernetes cluster. Satellites hear the wake word on the
device and stream audio to it. Regex intents answer the common commands with
no model; a fast model with tools takes the rest; the assistant (Claude Code
or ChatGPT, on the household's own subscription) takes the bigger jobs.
Third-party add-ons are sandboxed WebAssembly components from GitHub.

The full documentation as a single file is at ${at("/llms-full.txt")}.

## Start here

- [Why ${siteConfig.name}](${at("/why")}): who it is for, who should stay on Home Assistant, the ideas it is built on, and what leaves the house.
- [Add-ons](${at("/add-ons")}): the add-on library, from ${siteConfig.addons.index}.

${body}

## Source

- [Repository](${siteConfig.links.github}): source, issues and releases.
- [ARCHITECTURE.md](${siteConfig.links.architecture}): the crate map, the house rules and the big decisions.
`

  return new Response(text, {
    headers: { "Content-Type": "text/plain; charset=utf-8" },
  })
}
