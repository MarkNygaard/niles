import { BUTLER_PATHS, BUTLER_TRANSFORM, BUTLER_VIEWBOX } from "@/lib/butler-paths"

/**
 * The Niles butler mark, drawn as an SVG and filled with the theme's `mark`
 * token, so it follows light and dark the way the app's sign-in screen does.
 */
export function ButlerMark({ className = "size-5" }: { className?: string }) {
  return (
    <svg viewBox={BUTLER_VIEWBOX} className={`fill-current text-mark ${className}`} aria-hidden>
      <g transform={BUTLER_TRANSFORM}>
        {BUTLER_PATHS.map((d) => (
          <path key={d.slice(0, 16)} d={d} />
        ))}
      </g>
    </svg>
  )
}
