import { ButlerMark } from "@/components/butler-mark"
import { siteConfig } from "@/lib/config"

const link = "underline underline-offset-4 hover:text-foreground"

/**
 * Contributing and the code of conduct live at the repository root, where
 * GitHub shows them on pull requests; the footer links there rather than the
 * docs keeping a copy.
 */
export function SiteFooter() {
  return (
    <footer>
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-2 px-6 py-8 text-sm text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
        <span className="flex items-center gap-2">
          <ButlerMark className="size-4 shrink-0" />
          {siteConfig.name} is open source under the {siteConfig.license} license.
        </span>
        <span className="flex flex-wrap gap-x-4 gap-y-1">
          <a className={link} href={siteConfig.links.github} target="_blank" rel="noreferrer">
            Source on GitHub
          </a>
          <a className={link} href={siteConfig.links.contributing} target="_blank" rel="noreferrer">
            Contributing
          </a>
          <a className={link} href={siteConfig.links.codeOfConduct} target="_blank" rel="noreferrer">
            Code of conduct
          </a>
        </span>
      </div>
    </footer>
  )
}
