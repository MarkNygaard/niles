import { ImageResponse } from "next/og"

import { siteConfig } from "@/lib/config"
import { BUTLER_PATHS, BUTLER_TRANSFORM, BUTLER_VIEWBOX } from "@/lib/butler-paths"

export const size = { width: 1200, height: 630 }
export const contentType = "image/png"
export const alt = siteConfig.name

// Static export: the card is rendered once at build time, not per request.
export const dynamic = "force-static"

/**
 * The app's dark theme (`web/src/styles/theme.css`) in hex, because the
 * renderer reads no CSS variables: the canvas `--background`, the text
 * `--foreground` and `--muted-foreground`, and the mark in `--mark`, the brand
 * green lifted for a dark ground.
 */
const colours = {
  background: "#0a0a0a", // oklch(0.145 0 0)
  foreground: "#fafafa", // oklch(0.985 0 0)
  muted: "#a1a1a1", // oklch(0.708 0 0)
  mark: "#4b986f", // oklch(0.62 0.1 158)
}

/**
 * Social cards are rendered at build time from `siteConfig` and the butler
 * mark's own paths, so there are no PNGs to re-cut when either changes.
 */
export default function OpengraphImage() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          justifyContent: "center",
          padding: "80px",
          background: colours.background,
          color: colours.foreground,
          fontFamily: "sans-serif",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 32 }}>
          <svg width={120} height={120} viewBox={BUTLER_VIEWBOX} fill={colours.mark}>
            <g transform={BUTLER_TRANSFORM}>
              {BUTLER_PATHS.map((d) => (
                <path key={d.slice(0, 16)} d={d} />
              ))}
            </g>
          </svg>
          <div style={{ fontSize: 96, fontWeight: 700, letterSpacing: "-0.03em" }}>
            {siteConfig.name}
          </div>
        </div>
        <div
          style={{
            marginTop: 48,
            fontSize: 34,
            lineHeight: 1.35,
            color: colours.muted,
            maxWidth: 960,
          }}
        >
          {siteConfig.description}
        </div>
      </div>
    ),
    size
  )
}
