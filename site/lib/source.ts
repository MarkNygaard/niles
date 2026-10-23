import { docs } from "@/.source/server"
import { loader } from "fumadocs-core/source"

/**
 * The docs tree, derived from the files in `content/docs`. Adding a page means
 * adding a markdown file -- ordering is controlled by `content/docs/meta.json`,
 * not by code.
 */
export const source = loader({
  baseUrl: "/docs",
  source: docs.toFumadocsSource(),
})
