import { createFromSource } from "fumadocs-core/search/server"

import { source } from "@/lib/source"

/**
 * Search index. The site is a static export, so the index is built once at
 * build time and served as a file -- `staticGET` rather than a request handler.
 * The client side is switched to the matching `type: "static"` in the root
 * layout's provider.
 */
export const revalidate = false
export const dynamic = "force-static"

export const { staticGET: GET } = createFromSource(source)
