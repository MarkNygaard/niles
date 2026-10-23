import { DocsLayout } from "fumadocs-ui/layouts/docs"

import { source } from "@/lib/source"

/**
 * Sidebar only. The site header and footer come from the `(app)` layout above,
 * so fumadocs' own nav is disabled -- otherwise the docs would render a second,
 * different header and read as a separate site. Its search box and theme
 * switch are disabled for the same reason: both now live in the site header,
 * where they are reachable from the landing page as well.
 */
export default function Layout({ children }: { children: React.ReactNode }) {
  return (
    <DocsLayout
      tree={source.pageTree}
      nav={{ enabled: false }}
      searchToggle={{ enabled: false }}
      themeSwitch={{ enabled: false }}
      sidebar={{ collapsible: false }}
    >
      {children}
    </DocsLayout>
  )
}
