import {
  defineCollections,
  defineConfig,
  defineDocs,
  frontmatterSchema,
} from "fumadocs-mdx/config"

export const docs = defineDocs({
  dir: "content/docs",
})

/**
 * Standalone pages that sit beside the landing page rather than inside the docs
 * tree. Authored as MDX like everything else so they stay in `llms.txt`, but
 * rendered without the docs sidebar: the reader has not decided to use the
 * product yet.
 */
export const pages = defineCollections({
  type: "doc",
  dir: "content/pages",
  // Without a schema the frontmatter types as `unknown`.
  schema: frontmatterSchema,
})

type Node = { type: string; depth?: number; children?: Node[] }

/**
 * Tidies what `<include>` brings in from a repository file, which is written
 * for GitHub rather than for this site. It runs after fumadocs' include step,
 * so it sees the included nodes.
 *
 * - A level-one heading goes: the page's title is its frontmatter, and
 *   deploy/README.md and MANIFEST.md each open with their own.
 * - Raw HTML goes: MDX cannot render it, and the only instance is
 *   MANIFEST.md's "generated, do not edit" comment.
 */
function remarkTidyIncludes() {
  const drop = (child: Node) => (child.type === "heading" && child.depth === 1) || child.type === "html"
  // The include step splices a file in as a nested root, so every level is
  // looked at rather than only the top.
  const walk = (node: Node) => {
    if (!node.children) return
    node.children = node.children.filter((child) => !drop(child))
    node.children.forEach(walk)
  }
  return walk
}

export default defineConfig({
  mdxOptions: {
    remarkPlugins: [remarkTidyIncludes],
    rehypeCodeOptions: {
      themes: {
        light: "github-light-default",
        dark: "vesper",
      },
    },
  },
})
