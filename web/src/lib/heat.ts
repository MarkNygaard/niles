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
 * White, as tado has it, and asked for after seeing both.
 *
 * Worth recording what it costs, because the scale is not evenly dark:
 * white measures 3.5:1 on the teal at 5° and 3.1:1 on the orange at
 * 25°, which is AA for large text — but only 1.6:1 on the yellow around
 * 19°, which is the lightest point of the scale and genuinely hard to
 * read. The text nearest that band is the one sized up below. Capping
 * the yellow's lightness would fix it and would also stop it being
 * yellow, which is why the colour won.
 *
 * A fixed value rather than a token, because the sheet's colour does
 * not change with the theme and so its text cannot either — a
 * dark-mode foreground on this would be the same mistake in reverse.
 */
export const HEAT_INK = "#ffffff";

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

/**
 * How far the sheet's gradient travels either side of its colour.
 *
 * Measured off the screenshots. `5.png` has the clearest lightness and
 * chroma travel — across the height of the screen its lightness falls
 * 0.057 and its chroma rises 0.024 — and `1.png` has almost none of
 * either, but rotates its hue 179 to 199. Applying a share of all three
 * either side of the stop reproduces both well enough that neither
 * looks flat, without moving the colour the scale actually names.
 *
 * Note the direction: tado's sheets darken and saturate *downwards*.
 */
const SHEET_TRAVEL = { l: 0.02, c: 0.008, h: 8 };

/**
 * Which way the hue turns on the way down.
 *
 * Away from green, in both samples: the teal runs 179 → 199 and the
 * orange 54 → 40, one climbing and one falling but both moving further
 * from the middle of the scale. One rule covers them because it is the
 * same rule — a sheet deepens into its own colour rather than drifting
 * towards its neighbour.
 */
function away(hue: number): number {
  return hue > 120 ? 1 : -1;
}

/**
 * The sheet behind the dial, as a CSS gradient.
 *
 * A flat fill beside one of tado's reads as a swatch rather than a
 * surface — the same thing that makes their tiles look physical, at the
 * size of a whole screen.
 */
export function heatSheet(celsius: number | null): string {
  const [top, bottom] = sheetStops(celsius);
  return `linear-gradient(180deg, ${top} 0%, ${bottom} 100%)`;
}

/**
 * The colour at the very top of that sheet.
 *
 * For the status bar above it. In a standalone app the strip behind
 * the clock belongs to iOS rather than to the page, and the only say
 * the page has over it is the `theme-color` meta — so the sheet hands
 * it the colour its own first pixel has, and the seam disappears.
 */
export function heatTop(celsius: number | null): string {
  return sheetStops(celsius)[0];
}

function sheetStops(celsius: number | null): [string, string] {
  if (celsius === null) {
    return ["oklch(0.688 0.016 250)", "oklch(0.632 0.020 256)"];
  }
  const { l, c, h } = between(celsius);
  const turn = away(h) * SHEET_TRAVEL.h;
  return [
    `oklch(${round(l + SHEET_TRAVEL.l)} ${round(c - SHEET_TRAVEL.c)} ${round(h - turn, 1)})`,
    `oklch(${round(l - SHEET_TRAVEL.l)} ${round(c + SHEET_TRAVEL.c)} ${round(h + turn, 1)})`,
  ];
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
