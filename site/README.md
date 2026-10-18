# site

getniles.app: the landing page, the user guide, the add-on author guide and
the add-on library, as one static Next.js app.

- **Shared chrome.** `app/(app)/layout.tsx` holds the header, and the landing
  page, the docs and the add-on pages all live under that group. Fumadocs' own
  nav is disabled, and its docs layout contributes only the sidebar.
- **Landing page.** `app/(app)/page.tsx`, and `app/(app)/why/` renders
  `content/pages/why.mdx`.
- **Docs.** Markdown in `content/docs/`, rendered through
  [Fumadocs](https://fumadocs.dev) by `app/(app)/docs/[[...slug]]/page.tsx`.
  A page is a file; order and section titles are the `meta.json` files. The
  sections are `self-hosting/`, `setting-up/`, `using/`, `architecture/`,
  `writing-add-ons/` (the author guide) and `reference/`.
- **Repository files as pages.** `<include>` puts a file from the repo into a
  page, so it is written once: `self-hosting/docker-compose.mdx` is
  `deploy/compose/README.md`, `self-hosting/kubernetes.mdx` is
  `deploy/README.md`, and `architecture/features.mdx` is `MANIFEST.md`.
  Links inside those files are absolute, because a relative one would
  resolve against the page's address. A
  remark step in `source.config.ts` drops their own level-one title and raw
  HTML, which MDX can't render.
- **Identity.** `lib/config.ts` holds the name, URL, links and nav. Nothing in
  `app/` hard-codes the name.
- **Icons.** `app/icon1.png`, `app/icon2.png` and `app/apple-icon.png` are
  copies of the app's icons from `web/public/`. The butler mark in the header,
  the footer and the social card is drawn from `lib/butler-paths.ts`, the
  paths of `web/src/assets/butler.svg`, filled with the theme's `mark` colour.
- **Social cards.** Rendered at build time by `app/opengraph-image.tsx`.
- **Search.** `app/api/search/route.ts`: the index is built once and served as
  a file (`staticGET`), with the client on `type: "static"`.

## The add-on library

`app/(app)/add-ons/` lists what is in the registry's `index.json`
(`https://addons.getniles.app/index.json`, format in
`crates/niles-plugins/src/library.rs`), and `add-ons/[owner]/[name]` is one
page per add-on.

- `lib/addons.ts` reads the file as a stranger's, as Niles does: Niles's caps
  on text, one entry per repo, removed entries hidden, every link built from an
  `owner/name` that passed the same check as a pasted address. Text renders as
  React text. A logo is only ever an `<img>` with a `data:` URI, where an SVG
  runs no script and loads nothing. Permissions are not shown; Niles shows
  those itself before an install.
- `lib/addons-build.ts` fetches the file during the build, with a 10 second
  timeout, over HTTPS and then HTTP (the registry's GitHub Pages serves its
  custom domain over HTTP until its certificate is issued). If it can't, the build still
  succeeds: the list page loads the file in the browser instead, over HTTPS
  only, and those add-ons link to their repos because the build made no page
  for them. `ADDONS_INDEX_URL` points the build at another file, which is how
  to try the offline path.
- A static export needs at least one path for a dynamic route, so with no
  add-ons the detail route renders one placeholder, `/add-ons/_/_`, as a 404.

## Hosting

Vercel builds and hosts getniles.app. The project's settings:

- **Root Directory:** `site`, with "Include files outside the root directory
  in the Build Step" on. The build reads `web/src/styles/theme.css`,
  `deploy/README.md`, `deploy/compose/README.md` and `MANIFEST.md`.
- **Framework:** Next.js. Install and build are detected from `bun.lock` and
  `package.json` (`bun install`, `bun run build`); the static export in
  `out/` is what is served.
- **Production branch:** `main`, so the docs match the released version.
  Every other branch and pull request gets a preview.
- **Environment:** `NEXT_PUBLIC_SITE_URL=https://getniles.app` for
  production.
- **Domains:** `getniles.app` and `www.getniles.app`, with DNS at Cloudflare
  set to DNS only, as Vercel's domain page says.

The site also rebuilds daily (`.github/workflows/site.yml` calls the
project's deploy hook, in the `VERCEL_DEPLOY_HOOK` secret), which is how
add-ons published since the last release get their pages.

## Develop

```bash
cd site
bun install
bun run dev      # http://localhost:4000
```

## Verify

```bash
cd site && bun run typecheck && bun run build
```

Use `bun run`, not `bunx`: `bunx tsc` resolves from the network and can pull a
different TypeScript major than the pinned devDependency.

The output is `site/out`. `NEXT_PUBLIC_SITE_URL` is the base for canonical
URLs, the OG image, `sitemap.xml`, `robots.txt` and `llms.txt`, and defaults
to `https://getniles.app`.

## Discoverability

Generated at build time from `siteConfig`, the docs tree and the library:

- `sitemap.xml`: the landing page, `/why`, `/add-ons`, every doc page and
  every add-on page.
- `robots.txt`: open to everything, with the AI crawlers named, since several
  (Google-Extended, Applebot-Extended) read only a directive addressed to them.
- `llms.txt`: an [llmstxt.org](https://llmstxt.org) index in sidebar order.
- `llms-full.txt`: every page's markdown in one file, with `<include>`s
  filled in.
- JSON-LD: `WebSite` and `SoftwareApplication` on the landing page,
  `TechArticle` and `BreadcrumbList` on each doc page.

## Docs chrome

Fumadocs supplies the layout engine. Its surface is overridden in
`app/globals.css` on fumadocs' own ids, so there are no forked components:

- `#nd-docs-layout` gets `--fd-layout-width: 100%` for a full-width grid.
- `#nd-sidebar` loses its card fill and its solid end border; a pseudo-element
  draws a hairline that fades in below the header and out at the foot.
- From `md` up the docs are a fixed-height shell: the region is the viewport
  below the header, and the grid scrolls inside it. Below `md` the page
  scrolls, so a phone's address bar can collapse.
- The footer is rendered by the landing, why and add-on pages, not the shared
  layout: below a viewport-height docs region it would put the page back into
  scrolling. Contributing and the code of conduct are linked from it, and from
  the docs' introduction.

## Notes

- The theme is `web/src/styles/theme.css`, imported by `app/globals.css`, so
  the site and the app share one palette. `globals.css` maps fumadocs'
  `--color-fd-*` tokens onto it in plain CSS rather than `@theme`, which
  Tailwind hoists above fumadocs' own values. `next.config.mjs` sets
  `turbopack.root` to the repository root to allow that import, and the
  `<include>`s of repository files.
- The site is not part of the server binary or its image.
- `docs/capabilities/` is not published: it is compiled into the binary.
