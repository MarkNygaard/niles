import { parseHex } from "@/components/ColorField";
import { cn } from "@/lib/utils";

/** Sane ends for a lamp, not the 1000–10000 the API will accept. */
export const KELVIN_MIN = 1800;
export const KELVIN_MAX = 6500;

/** Cool at one end, warm at the other, white through the middle. */
const KELVIN_COOL = [166, 209, 255] as const;
const KELVIN_WARM = [255, 160, 0] as const;

/**
 * A brightness glyph, not a gauge.
 *
 * An earlier version filled to the level, which nobody could read: the
 * unfilled part is dark on a dark card, so there was no container to
 * read the level against.
 */
export function BrightnessGlyph({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 24 24"
      className={cn("size-5 fill-current", className)}
      focusable="false"
    >
      <path d="M12,18V6A6,6 0 0,1 18,12A6,6 0 0,1 12,18M20,15.31L23.31,12L20,8.69V4H15.31L12,0.69L8.69,4H4V8.69L0.69,12L4,15.31V20H8.69L12,23.31L15.31,20H20V15.31Z" />
    </svg>
  );
}

/**
 * The chosen colour, or the wheel itself when there isn't one.
 *
 * The unset face is the same wheel the popover opens: hue around the
 * rim, white in the middle, so the button looks like the thing it
 * leads to.
 */
export function ColorSwatch({
  value,
  className,
}: {
  /** `#rrggbb`, or undefined when nothing is set. */
  value?: string;
  className?: string;
}) {
  const rgb = value ? parseHex(value) : null;
  return (
    <span
      aria-hidden
      className={cn("border-border size-6 rounded-full border", className)}
      style={
        rgb
          ? { background: value }
          : {
              backgroundImage: [
                "radial-gradient(circle closest-side, #ffffff, rgba(255,255,255,0) 78%)",
                "conic-gradient(from 90deg, #ff0000, #ff00ff, #0000ff, #00ffff, #00ff00, #ffff00, #ff0000)",
              ].join(", "),
            }
      }
    />
  );
}

/** The white it holds, over the warm-to-cool range it can hold. */
export function KelvinSwatch({
  value,
  className,
}: {
  value?: number;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      className={cn("border-border size-6 rounded-full border", className)}
      style={{
        background: value
          ? kelvinSwatch(value)
          : `linear-gradient(90deg, rgb(${KELVIN_WARM.join(", ")}) 0%, rgb(255, 255, 255) 50%, rgb(${KELVIN_COOL.join(", ")}) 100%)`,
      }}
    />
  );
}

/**
 * Kelvin as a screen colour, taken from the warm-to-cool ramp rather
 * than from the blackbody curve the chart uses.
 *
 * A physically accurate 2200 K is a muddy brown at thumbnail size; this
 * reads as a warm lamp, which is what the setting means.
 */
export function kelvinSwatch(kelvin: number): string {
  const span = KELVIN_MAX - KELVIN_MIN;
  const t = Math.min(Math.max((kelvin - KELVIN_MIN) / span, 0), 1);
  const white = [255, 255, 255] as const;
  const [from, to, mix] =
    t < 0.5 ? [KELVIN_WARM, white, t * 2] : [white, KELVIN_COOL, (t - 0.5) * 2];
  const channel = (i: number) => Math.round(from[i] + (to[i] - from[i]) * mix);
  return `rgb(${channel(0)}, ${channel(1)}, ${channel(2)})`;
}
