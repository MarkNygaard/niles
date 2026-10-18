"use client"

import { MoonIcon, SunIcon } from "lucide-react"
import { useTheme } from "next-themes"

/**
 * Light/dark switch.
 *
 * Which icon shows is decided by CSS (`dark:` variants) rather than by React
 * state, so there is nothing to reconcile on hydration and no placeholder
 * flash -- `resolvedTheme` is only read inside the click handler, which never
 * runs on the server.
 */
export function ModeToggle() {
  const { setTheme, resolvedTheme } = useTheme()

  return (
    <button
      type="button"
      aria-label="Toggle between light and dark theme"
      onClick={() => setTheme(resolvedTheme === "dark" ? "light" : "dark")}
      className="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
    >
      <SunIcon className="hidden size-4 dark:block" />
      <MoonIcon className="size-4 dark:hidden" />
    </button>
  )
}
