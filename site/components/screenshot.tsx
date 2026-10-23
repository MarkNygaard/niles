/**
 * A screenshot of the app, framed like a window. Plain `<img>`: the site is
 * a static export, so Next's image optimiser has no server to run on, and the
 * files in `public/screenshots` are already WebP at twice the size they are
 * shown.
 */
export function Screenshot({
  src,
  alt,
  caption,
  priority = false,
  className,
}: {
  src: string
  alt: string
  caption?: string
  /** The one above the fold: loaded at once rather than lazily. */
  priority?: boolean
  className?: string
}) {
  return (
    <figure className={["flex flex-col gap-3", className].filter(Boolean).join(" ")}>
      {/* eslint-disable-next-line @next/next/no-img-element -- static export */}
      <img
        src={src}
        alt={alt}
        width={2400}
        height={1500}
        loading={priority ? "eager" : "lazy"}
        decoding="async"
        className="h-auto w-full rounded-xl border border-border shadow-lg"
      />
      {caption && <figcaption className="text-sm text-muted-foreground">{caption}</figcaption>}
    </figure>
  )
}
