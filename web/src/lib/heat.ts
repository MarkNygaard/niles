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
 * Every stop is measured out of tado's own screens rather than guessed:
 * `#329896` at 5°, `#31b27f` at 12°, `#2fb77d` at 18.5°, `#f6c944` at
 * 19° and `#ec6a2c` at 25°. Two of those were guesses first and both
 * were wrong — the yellow sits at hue 90 rather than the 120 that
 * seemed right, and the orange at 43 rather than 60.
 *
 * Note that the lightness is not monotonic: it climbs to 0.853 at the
 * yellow and falls back to 0.674 for the orange. Yellow is simply a
 * light colour, and forcing it down the way the rest of the scale
 * suggests would produce olive rather than yellow.
 *
 * What does not come from tado is the foreground — see [`HEAT_INK`].
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
  { at: 5, l: 0.622, c: 0.092, h: 193 },
  { at: 12, l: 0.682, c: 0.134, h: 162 },
  { at: 18.5, l: 0.694, c: 0.143, h: 160 },
  { at: 19, l: 0.853, c: 0.152, h: 90 },
  { at: 25, l: 0.674, c: 0.176, h: 43 },
];

/**
 * What goes on top of them.
 *
 * Dark, where tado uses white. Their colours are right and their
 * foreground is not: white on the yellow at 19° measures 1.57:1, which
 * is unreadable, and 2.6:1 on the greens. The same ink reads 4.7:1 at
 * worst and 10.8:1 at best across the whole scale, which clears AA for
 * ordinary text rather than only for headings.
 *
 * A fixed value rather than a token, because the sheet's colour does
 * not change with the theme and so its text cannot either — a
 * light-mode foreground on a dark-mode page would be the same mistake
 * in reverse.
 */
export const HEAT_INK = "#1c1c1e";

/** Off is not on the scale. It is the absence of one. */
const OFF = "oklch(0.66 0.018 250)";

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
      // Hues descend 193 → 43 across the whole scale, so there is no
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
