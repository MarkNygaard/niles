/**
 * The add-on library's `index.json`, read the way Niles reads it.
 *
 * The format is the registry's contract, documented in
 * `crates/niles-plugins/src/library.rs`. The file is a stranger's: every
 * entry is checked here, text is capped at Niles's own limits, links are
 * built from the validated `owner/name` and never taken from the file, and
 * a logo only ever becomes the `src` of an `<img>` (where an SVG runs no
 * script and loads nothing), never markup in the page.
 *
 * Pure and dependency-free, so the build and the browser share it.
 */

/** Niles's caps, from `niles_plugins::library`. */
const MAX_RAW = 8000
const MAX_ADDONS = 2000
const MAX_NAME = 100
const MAX_TEXT = 300
const MAX_README = 1000
const MAX_VERSION = 64
const MAX_MANIFEST = 64 * 1024
const MAX_LOGO = 32 * 1024

const OWNER = /^[A-Za-z0-9-]{1,39}$/
const NAME = /^[A-Za-z0-9._-]{1,100}$/

export type Standing = "listed" | "unreviewed"

export type Addon = {
  owner: string
  /** The repo's own name, as in its URL. */
  repo: string
  standing: Standing
  /** The manifest's `name`, or the repo's when the manifest doesn't say. */
  name: string
  description: string
  version: string | null
  /** ISO 8601, from the latest release. */
  publishedAt: string | null
  /** Downloads of `niles-installed`; null when no release has one. */
  installs: number | null
  /** Downloads of the component, previews and updates included. */
  downloads: number
  stars: number
  /** A `data:` URI for an `<img>`, or null. */
  logo: string | null
  readme: string | null
}

export type Library = {
  generatedAt: string | null
  addons: Addon[]
}

/** `owner/name`, checked the way Niles checks a pasted address. */
export function parseRepo(value: unknown): { owner: string; repo: string } | null {
  if (typeof value !== "string") return null
  const parts = value.trim().split("/")
  if (parts.length !== 2) return null
  const owner = parts[0]
  const repo = parts[1].endsWith(".git") ? parts[1].slice(0, -4) : parts[1]
  if (!OWNER.test(owner) || !NAME.test(repo) || repo === "." || repo === "..") return null
  return { owner, repo }
}

export const repoUrl = (a: { owner: string; repo: string }) =>
  `https://github.com/${a.owner}/${a.repo}`
export const ownerUrl = (a: { owner: string }) => `https://github.com/${a.owner}`
export const pagePath = (a: { owner: string; repo: string }) => `/add-ons/${a.owner}/${a.repo}`

function capped(text: string, max: number): string {
  return Array.from(text.trim()).slice(0, max).join("").trimEnd()
}

function firstLine(text: string): string {
  return text.split(/\r?\n/, 1)[0] ?? ""
}

function str(value: unknown): string | null {
  return typeof value === "string" ? value : null
}

function count(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : null
}

function date(value: unknown): string | null {
  const s = str(value)
  if (!s) return null
  const t = Date.parse(s)
  return Number.isNaN(t) ? null : new Date(t).toISOString()
}

const ESCAPES: Record<string, string> = { '"': '"', "\\": "\\", n: "\n", t: "\t", r: "\r", b: "\b", f: "\f" }

/** A TOML basic string's escapes. Anything odd leaves the value unread. */
function unescapeBasic(body: string): string | null {
  let out = ""
  for (let i = 0; i < body.length; i++) {
    const c = body[i]
    if (c !== "\\") {
      out += c
      continue
    }
    const next = body[++i]
    if (Object.hasOwn(ESCAPES, next)) {
      out += ESCAPES[next]
    } else if (next === "u" || next === "U") {
      const len = next === "u" ? 4 : 8
      const hex = body.slice(i + 1, i + 1 + len)
      if (!/^[0-9A-Fa-f]+$/.test(hex) || hex.length !== len) return null
      const code = Number.parseInt(hex, 16)
      if (code > 0x10ffff) return null
      out += String.fromCodePoint(code)
      i += len
    } else {
      return null
    }
  }
  return out
}

/**
 * The manifest's top-level `name`, `description` and `version`, and nothing
 * else. Only single-line strings before the first table are read; a key
 * written any other way is left unread, and the entry falls back to what
 * the repo says. Permissions are not read at all: Niles shows those itself,
 * in its own words, before an install.
 */
export function manifestFields(manifest: string): { name?: string; description?: string; version?: string } {
  const fields: { name?: string; description?: string; version?: string } = {}
  for (const line of manifest.split(/\r?\n/)) {
    if (/^\s*\[/.test(line)) break
    const m =
      /^\s*(name|description|version)\s*=\s*(?:"((?:[^"\\]|\\.)*)"|'([^']*)')\s*(?:#.*)?$/.exec(line)
    if (!m) continue
    const key = m[1] as "name" | "description" | "version"
    if (key in fields) continue
    const value = m[2] !== undefined ? unescapeBasic(m[2]) : m[3]
    if (value !== null && value !== undefined) fields[key] = value
  }
  return fields
}

function logoUri(value: unknown): string | null {
  const svg = str(value)
  if (!svg || svg.length > MAX_LOGO) return null
  if (!/^\s*(<\?xml[^>]*>\s*)?<svg[\s>]/.test(svg)) return null
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

function entry(raw: Record<string, unknown>, owner: string, repo: string, standing: Standing): Addon {
  const release =
    raw.release && typeof raw.release === "object" ? (raw.release as Record<string, unknown>) : null
  const manifest = str(release?.manifest)
  const fields = manifest && manifest.length <= MAX_MANIFEST ? manifestFields(manifest) : {}
  const tag = str(release?.tag)
  const version = fields.version ?? (tag ? tag.replace(/^v/, "") : null)
  const description = firstLine(fields.description ?? str(raw.description) ?? "")
  const readme = str(raw.readme)
  return {
    owner,
    repo,
    standing,
    name: capped(firstLine(fields.name ?? "") || repo, MAX_NAME),
    description: capped(description, MAX_TEXT),
    version: version ? capped(firstLine(version), MAX_VERSION) || null : null,
    publishedAt: date(release?.published_at),
    installs: count(raw.installs),
    downloads: count(raw.downloads) ?? 0,
    stars: count(raw.stars) ?? 0,
    logo: logoUri(release?.logo),
    readme: readme ? capped(readme, MAX_README) || null : null,
  }
}

/**
 * The library as the site shows it: listed and unreviewed add-ons, with
 * removed ones dropped. A bad entry costs that entry; a file that isn't
 * format 1 reads as no library at all.
 */
export function readLibrary(raw: unknown): Library | null {
  if (!raw || typeof raw !== "object") return null
  const index = raw as Record<string, unknown>
  if (index.version !== 1) return null
  const rawAddons = Array.isArray(index.addons) ? index.addons.slice(0, MAX_RAW) : []

  // One entry per repo, as Niles keeps them: GitHub names are
  // case-insensitive, and a removal wins over any other standing.
  const seen = new Map<string, { raw: Record<string, unknown>; owner: string; repo: string; standing: Standing | "removed" }>()
  for (const item of rawAddons) {
    if (!item || typeof item !== "object") continue
    const raw = item as Record<string, unknown>
    const parsed = parseRepo(raw.repo)
    if (!parsed) continue
    const standing = raw.standing === "listed" ? "listed" : raw.standing === "removed" ? "removed" : "unreviewed"
    const key = `${parsed.owner}/${parsed.repo}`.toLowerCase()
    const rank = { unreviewed: 0, listed: 1, removed: 2 }
    const before = seen.get(key)
    if (!before || rank[standing] > rank[before.standing]) seen.set(key, { raw, ...parsed, standing })
  }

  const addons: Addon[] = []
  for (const { raw, owner, repo, standing } of seen.values()) {
    if (standing === "removed") continue
    if (addons.length >= MAX_ADDONS) break
    addons.push(entry(raw, owner, repo, standing))
  }
  addons.sort((a, b) => b.stars - a.stars || a.name.localeCompare(b.name))
  return { generatedAt: date(index.generated_at), addons }
}
