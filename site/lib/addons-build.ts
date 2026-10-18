import http from "node:http"
import https from "node:https"

import { readLibrary, type Library } from "@/lib/addons"
import { siteConfig } from "@/lib/config"

/** Niles's own ceiling for the file. */
const MAX_INDEX = 8 * 1024 * 1024
const TIMEOUT_MS = 10_000

/**
 * Where the build looks for the index. `ADDONS_INDEX_URL` overrides it, which
 * is also how to check the offline path (point it somewhere that fails).
 *
 * Plain HTTP is tried after HTTPS because GitHub Pages serves the custom
 * domain over HTTP until its certificate is issued. That costs nothing: the
 * file is read as a stranger's either way. The browser fallback uses HTTPS
 * only, since the site itself is HTTPS and would block the mixed request.
 */
function candidates(): string[] {
  const override = process.env.ADDONS_INDEX_URL
  if (override) return [override]
  return [siteConfig.addons.index, siteConfig.addons.index.replace(/^https:/, "http:")]
}

/**
 * One GET, with Node's own client rather than `fetch`. Next wraps `fetch` in
 * its data cache, which either keeps an old library across local builds or,
 * told not to, refuses to render the route statically at all.
 */
function get(url: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const client = url.startsWith("https:") ? https : http
    const request = client.get(url, { timeout: TIMEOUT_MS }, (response) => {
      if (response.statusCode !== 200) {
        response.resume()
        reject(new Error(`HTTP ${response.statusCode}`))
        return
      }
      const chunks: Buffer[] = []
      let size = 0
      response.on("data", (chunk: Buffer) => {
        size += chunk.length
        if (size > MAX_INDEX) {
          request.destroy(new Error("larger than 8 MiB"))
          return
        }
        chunks.push(chunk)
      })
      response.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")))
      response.on("error", reject)
    })
    request.on("timeout", () => request.destroy(new Error(`no answer in ${TIMEOUT_MS / 1000} s`)))
    request.on("error", reject)
  })
}

let pending: Promise<Library | null> | undefined

/**
 * The library at build time, or null when it can't be had. Null never fails
 * the build: the list page then loads the index in the browser instead.
 */
export function buildTimeLibrary(): Promise<Library | null> {
  pending ??= (async () => {
    for (const url of candidates()) {
      try {
        const library = readLibrary(JSON.parse(await get(url)))
        if (library) {
          console.log(`[add-ons] ${library.addons.length} add-ons from ${url}`)
          return library
        }
        console.warn(`[add-ons] ${url} is not a format 1 index`)
      } catch (error) {
        console.warn(`[add-ons] could not read ${url}: ${error instanceof Error ? error.message : error}`)
      }
    }
    console.warn("[add-ons] building without the library; /add-ons will load it in the browser")
    return null
  })()
  return pending
}
