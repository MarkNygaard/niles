import { SiteHeader } from "@/components/site-header"

/**
 * Chrome shared by the landing page and the docs. Both live under this group so
 * moving between them never swaps the header out -- the docs are a section of
 * the site, not a second site. The docs layout nested below adds only a
 * sidebar.
 *
 * The footer is not here: the docs fill the viewport exactly and scroll
 * internally, so anything below them would put the page back into scrolling.
 * The landing page renders it itself.
 *
 * `--fd-nav-height` tells fumadocs how tall our header is, so its sticky
 * sidebar and table of contents sit below it instead of underneath it. It is
 * `h-14` plus the 1px bottom border -- leave the border out and the docs
 * overshoot the viewport by exactly 1px, which is enough to raise a page
 * scrollbar and steal a gutter from the right edge.
 */
export default function AppLayout({
  children,
}: {
  children: React.ReactNode
}) {
  return (
    <div
      className="relative flex min-h-svh flex-col bg-background"
      style={{ "--fd-nav-height": "calc(3.5rem + 1px)" } as React.CSSProperties}
    >
      <SiteHeader />
      <main className="flex min-h-0 flex-1 flex-col">{children}</main>
    </div>
  )
}
