import { source } from "@/lib/source"

export type OrderedPage = ReturnType<typeof source.getPages>[number]

export type DocsSection = {
  title: string
  pages: OrderedPage[]
}

type TreeNode = {
  type: string
  name?: unknown
  url?: string
  index?: { url?: string }
  children?: TreeNode[]
}

function label(name: unknown): string {
  return typeof name === "string" ? name : String(name ?? "")
}

/**
 * Documentation grouped and ordered the way the sidebar presents it.
 *
 * `source.getPages()` returns a flat list in filesystem order, which is
 * alphabetical and puts a folder's own index page in with the top-level pages.
 * The page tree already encodes the `meta.json` ordering and the folder
 * structure, so reading order comes from there instead — it is the same order a
 * human reads the sidebar in, which is the order a model should receive too.
 */
export function docsSections(): DocsSection[] {
  const byUrl = new Map(source.getPages().map((p) => [p.url, p]))
  const tree = source.pageTree as unknown as TreeNode
  const sections: DocsSection[] = []

  // Walk in tree order rather than collecting loose pages and appending
  // folders afterwards. Otherwise a folder always lands after every page,
  // which is neither what the sidebar shows nor the order anyone reads in.
  let title = "Getting started"
  let current: DocsSection = { title, pages: [] }
  const flush = () => {
    if (current.pages.length > 0) sections.push(current)
  }

  for (const node of tree.children ?? []) {
    // A separator ends the section before it and names the one after.
    if (node.type === "separator") {
      flush()
      title = label(node.name)
      current = { title, pages: [] }
      continue
    }

    if (node.type === "page" && node.url) {
      const page = byUrl.get(node.url)
      if (page) current.pages.push(page)
      continue
    }

    if (node.type === "folder") {
      flush()

      const folder: DocsSection = { title: label(node.name), pages: [] }
      const indexUrl = node.index?.url
      if (indexUrl) {
        const page = byUrl.get(indexUrl)
        if (page) folder.pages.push(page)
      }
      for (const child of node.children ?? []) {
        if (child.type !== "page" || !child.url || child.url === indexUrl) continue
        const page = byUrl.get(child.url)
        if (page) folder.pages.push(page)
      }
      if (folder.pages.length > 0) sections.push(folder)

      // Resume the enclosing group. Without this, a page sitting after a
      // folder falls into an unnamed section and is silently dropped.
      current = { title, pages: [] }
      continue
    }
  }
  flush()

  // A group interrupted by a folder produces two entries with one title.
  // Fold them back together, keeping the first position.
  const merged: DocsSection[] = []
  for (const section of sections) {
    const existing = merged.find((m) => m.title === section.title)
    if (existing) existing.pages.push(...section.pages)
    else merged.push(section)
  }
  return merged
}

/** Every page, flattened back out but still in reading order. */
export function docsInOrder(): OrderedPage[] {
  return docsSections().flatMap((section) => section.pages)
}
