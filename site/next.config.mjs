import path from "node:path"
import { fileURLToPath } from "node:url"
import { createMDX } from "fumadocs-mdx/next"

const withMDX = createMDX()

const dirname = path.dirname(fileURLToPath(import.meta.url))

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  // Next writes its own AGENTS.md/CLAUDE.md into this directory otherwise.
  // The repository root already carries the canonical pair, and a second,
  // auto-generated set inside `site/` would quietly compete with it.
  agentRules: false,
  // The site is fully static: no server runtime, deployable to any host.
  output: "export",
  images: { unoptimized: true },
  turbopack: {
    // `app/globals.css` imports the shared theme from `web/`, one level up.
    // Without this, Turbopack refuses the import as escaping the project root.
    root: path.resolve(dirname, ".."),
  },
}

export default withMDX(config)
