/**
 * What a temperature looks like.
 *
 * Blue-green at the bottom of the range, through green, turning yellow
 * around 19° and reaching orange at the top — tado's scale, because it
 * is a good one: the colour is doing the reading before you get to the
 * number, and warming it by half a degree visibly warms the screen.
 *
 * Interpolated in oklch rather than sRGB, which is the whole reason it
 * looks smooth. Mixing `#438f8a` and `#469364` in sRGB runs through a
 * muddy grey in the middle, because sRGB's channels are not
 * perceptually even; oklch's are, so every step between two stops looks
 * like the same size step.
 *
 * The hues and the rising chroma are measured from tado's own screens —
 * 185 at 5°, 162 at 12°, 155 at 18.5°, then the turn to yellow and
 * orange. The lightness is not. Theirs climbs to 0.74 by 18.5°, where
 * white text sits at 2.18:1 and is genuinely hard to read; ours stops
 * at 0.66, which holds 3.2–3.8:1 across the whole range.
 *
 * That is still AA for large text only. It is why the secondary line on
 * a heated panel is sized up rather than left at `text-xs` — anything
 * smaller does not belong on this background.
 */

interface Stop {
  at: number;
  l: number;
  c: number;
  h: number;
}

/**
 * The scale, coldest first.
 *
 * 18.5 and 19 are both stops because that is where tado turns: 18.5 is
 * the last green, 19 the first yellow. Half a degree carrying a whole
 * hue shift is deliberate on their part and worth keeping — it is the
 * one place on the dial where a small move means something.
 */
const STOPS: Stop[] = [
  { at: 5, l: 0.6, c: 0.09, h: 185 },
  { at: 12, l: 0.618, c: 0.12, h: 162 },
  { at: 18.5, l: 0.632, c: 0.145, h: 155 },
  { at: 19, l: 0.645, c: 0.15, h: 120 },
  { at: 25, l: 0.66, c: 0.15, h: 60 },
];

/** Off is not on the scale. It is the absence of one. */
const OFF = "oklch(0.62 0.015 250)";

/**
 * The colour for a setting, as a CSS `oklch()`.
 *
 * `null` is off, which is grey — not the coldest colour, because off is
 * not cold, it is nothing.
 */
export function heatColor(celsius: number | null): string {
  if (celsius === null) return OFF;
  const stop = between(celsius);
  return `oklch(${round(stop.l)} ${round(stop.c)} ${round(stop.h, 1)})`;
}

/** The interpolated stop at a temperature, clamped to the ends. */
function between(celsius: number): Omit<Stop, "at"> {
  const first = STOPS[0];
  const last = STOPS[STOPS.length - 1];
  if (celsius <= first.at) return first;
  if (celsius >= last.at) return last;

  for (let i = 0; i < STOPS.length - 1; i++) {
    const from = STOPS[i];
    const to = STOPS[i + 1];
    if (celsius > to.at) continue;
    const t = (celsius - from.at) / (to.at - from.at);
    return {
      l: mix(from.l, to.l, t),
      c: mix(from.c, to.c, t),
      // Hues descend 190 → 55 across the whole scale, so there is no
      // wrap to reason about: straight interpolation is the short way
      // round every time.
      h: mix(from.h, to.h, t),
    };
  }
  return last;
}

function mix(from: number, to: number, t: number): number {
  return from + (to - from) * t;
}

function round(value: number, places = 3): number {
  const factor = 10 ** places;
  return Math.round(value * factor) / factor;
}
