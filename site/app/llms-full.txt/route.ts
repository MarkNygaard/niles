import fs from "node:fs/promises"
import path from "node:path"

import { siteConfig } from "@/lib/config"
import { docsInOrder } from "@/lib/docs-order"

export const dynamic = "force-static"

const CONTENT_DIR = path.join(process.cwd(), "content/docs")
const PAGES_DIR = path.join(process.cwd(), "content/pages")

/** Strip YAML frontmatter — the title and description are re-emitted as prose. */
function stripFrontmatter(raw: string): string {
  if (!raw.startsWith("---")) return raw.trim()
  const end = raw.indexOf("\n---", 3)
  return end === -1 ? raw.trim() : raw.slice(end + 4).trim()
}

/** One frontmatter field, enough for the one standalone page. */
function frontmatterField(raw: string, key: string): string | undefined {
  const match = new RegExp(`^${key}:\\s*"?(.*?)"?\\s*$`, "m").exec(raw)
  return match?.[1]
}

/**
 * Inline `<include>` tags the way the page renders them, so a page that is a
 * repository file (deploy/README.md, MANIFEST.md) carries that file here too.
 */
async function resolveIncludes(body: string, file: string): Promise<string> {
  const tags = [...body.matchAll(/<include>(.+?)<\/include>/g)]
  let out = body
  for (const [tag, target] of tags) {
    const included = await fs
      .readFile(path.resolve(path.dirname(file), target), "utf8")
      .catch(() => "")
    // A function, so a `$` in the included file is not read as a pattern.
    out = out.replace(tag, () => included.trim())
  }
  return out
}

/**
 * Every documentation page concatenated as markdown, in reading order.
 *
 * Served from the source files rather than from the rendered pages: a model
 * asking how to write an add-on should get the markdown, not HTML with a
 * sidebar, a search box and a theme toggle in it.
 */
export async function GET() {
  const sections = await Promise.all(
    docsInOrder().map(async (page) => {
      const rel = (page as { path?: string }).path
      if (!rel) return null

      const file = path.join(CONTENT_DIR, rel)
      let raw: string
      try {
        raw = await fs.readFile(file, "utf8")
      } catch {
        return null
      }

      const url = new URL(page.url, siteConfig.url).toString()
      return [
        `# ${page.data.title}`,
        page.data.description ? `> ${page.data.description}` : null,
        `Source: ${url}`,
        "",
        await resolveIncludes(stripFrontmatter(raw), file),
      ]
        .filter(Boolean)
        .join("\n")
    })
  )

  // The standalone /why page sits outside the docs tree, so it is read
  // directly and placed first. It is the page a model should see before any
  // of the reference material.
  const whyRaw = await fs
    .readFile(path.join(PAGES_DIR, "why.mdx"), "utf8")
    .catch(() => null)
  const why = whyRaw
    ? `# ${frontmatterField(whyRaw, "title") ?? "Why"}\nSource: ${new URL("/why", siteConfig.url).toString()}\n\n${stripFrontmatter(whyRaw)}`
    : null

  const text = `# ${siteConfig.name}: full documentation

> ${siteConfig.description}

Generated from ${siteConfig.url}. Index: ${new URL("/llms.txt", siteConfig.url).toString()}

${[why, ...sections].filter(Boolean).join("\n\n---\n\n")}
`

  return new Response(text, {
    headers: { "Content-Type": "text/plain; charset=utf-8" },
  })
}
