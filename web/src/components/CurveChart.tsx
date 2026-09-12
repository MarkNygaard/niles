/**
 * The day, drawn.
 *
 * Height is brightness; the colour of the line is the colour the lights
 * will actually be at that hour. Numbers in a form can't show that a
 * ramp is too short or that a night floor sits above the evening, and
 * that is the whole thing this page is for.
 *
 * The maths here mirrors `crates/niles-scheduler/src/curve.rs`
 * exactly — piecewise-linear brightness between four times, colour
 * temperature interpolated between anchors, held flat outside them. A
 * chart that drew an approximation would be worse than no chart.
 */

interface Anchor {
  minute: number;
  kelvin: number;
}

interface Instant {
  /** 0 = Monday, matching `WeekInstant::minute_of_week`. */
  day: number;
  minute: number;
}

export interface Curve {
  morningStart: number;
  morningEnd: number;
  sunsetStart: number;
  sunsetEnd: number;
  nightFloor: number;
  daytime: number;
  anchors: Anchor[];
  pause?: { start: Instant; end: Instant };
}

const WIDTH = 1440;
const HEIGHT = 250;
const LEFT = 46;
const RIGHT = 20;
const TOP = 16;
const BASE = 196;

export function CurveChart({ lighting }: { lighting: unknown }) {
  const curve = readCurve(lighting);
  // Nothing to draw from a config missing half the curve, and a
  // half-drawn one would misinform.
  if (!curve) return null;

  const x = (minute: number) =>
    LEFT + (minute / 1440) * (WIDTH - LEFT - RIGHT);
  const y = (brightness: number) =>
    TOP + (1 - brightness / 100) * (BASE - TOP);

  // Piecewise-linear, so only the corners need plotting.
  const corners = [
    0,
    curve.morningStart,
    curve.morningEnd,
    curve.sunsetStart,
    curve.sunsetEnd,
    1440,
  ];
  const points = corners.map(
    (minute) => `${x(minute)},${y(brightnessAt(curve, Math.min(minute, 1439)))}`,
  );

  const now = new Date();
  const held = curve.pause && isPaused(curve.pause, now);
  const marked = held
    ? curve.pause!.start.minute
    : now.getHours() * 60 + now.getMinutes();
  const markedBrightness = brightnessAt(curve, marked);
  const markedKelvin = colorTempAt(curve, marked);
  const labelAtEnd = marked > 1080;

  return (
    <figure className="m-0 flex flex-col gap-1 py-4">
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="h-auto w-full"
        role="img"
        aria-label={`Brightness over the day, from ${curve.nightFloor}% overnight to ${curve.daytime}% in the day`}
      >
        <defs>
          <linearGradient
            id="curve-kelvin"
            gradientUnits="userSpaceOnUse"
            x1={x(0)}
            x2={x(1440)}
          >
            {curve.anchors.map((anchor) => (
              <stop
                key={anchor.minute}
                offset={anchor.minute / 1440}
                stopColor={kelvinToColor(anchor.kelvin)}
              />
            ))}
          </linearGradient>
        </defs>

        {/* The two ramps, as the only regions that aren't flat. */}
        {[
          [curve.morningStart, curve.morningEnd],
          [curve.sunsetStart, curve.sunsetEnd],
        ].map(([from, to]) => (
          <rect
            key={from}
            x={x(from)}
            y={TOP}
            width={Math.max(x(to) - x(from), 1)}
            height={BASE - TOP}
            fill="var(--foreground)"
            opacity={0.04}
          />
        ))}

        {/* The two levels the curve rests at. */}
        {[curve.nightFloor, curve.daytime].map((level) => (
          <g key={level}>
            <line
              x1={LEFT}
              x2={WIDTH - RIGHT}
              y1={y(level)}
              y2={y(level)}
              stroke="var(--border)"
              strokeDasharray="6 10"
            />
            <text
              x={LEFT - 10}
              y={y(level) + 7}
              textAnchor="end"
              fontSize={20}
              fill="var(--muted-foreground)"
            >
              {level}%
            </text>
          </g>
        ))}

        <polygon
          points={`${x(0)},${BASE} ${points.join(" ")} ${x(1440)},${BASE}`}
          fill="url(#curve-kelvin)"
          opacity={0.16}
        />
        <polyline
          points={points.join(" ")}
          fill="none"
          stroke="url(#curve-kelvin)"
          strokeWidth={4}
          strokeLinejoin="round"
        />

        <line
          x1={x(marked)}
          x2={x(marked)}
          y1={TOP}
          y2={BASE}
          stroke="var(--foreground)"
          strokeWidth={2}
          opacity={0.35}
        />
        <circle
          cx={x(marked)}
          cy={y(markedBrightness)}
          r={9}
          fill={kelvinToColor(markedKelvin)}
          stroke="var(--card)"
          strokeWidth={3}
        />
        <text
          x={x(marked) + (labelAtEnd ? -14 : 14)}
          y={TOP + 18}
          textAnchor={labelAtEnd ? "end" : "start"}
          fontSize={20}
          fill="var(--muted-foreground)"
        >
          {held ? "held" : "now"} · {markedBrightness}% · {markedKelvin}K
        </text>

        {[0, 360, 720, 1080, 1440].map((minute) => (
          <text
            key={minute}
            x={x(minute)}
            y={BASE + 34}
            textAnchor={minute === 0 ? "start" : minute === 1440 ? "end" : "middle"}
            fontSize={20}
            fill="var(--muted-foreground)"
          >
            {String(Math.floor(minute / 60)).padStart(2, "0")}
          </text>
        ))}
      </svg>

      {held && (
        <figcaption className="text-muted-foreground text-xs">
          The curve is paused, so every light is held where it stood at{" "}
          {formatInstant(curve.pause!.start)} until{" "}
          {formatInstant(curve.pause!.end)}.
        </figcaption>
      )}
    </figure>
  );
}

/** Brightness at a minute of the day, per `curve.rs::brightness_at`. */
export function brightnessAt(curve: Curve, minute: number): number {
  const { morningStart, morningEnd, sunsetStart, sunsetEnd } = curve;
  if (minute < morningStart || minute >= sunsetEnd) return curve.nightFloor;
  if (minute < morningEnd) {
    return lerp(
      curve.nightFloor,
      curve.daytime,
      minute - morningStart,
      morningEnd - morningStart,
    );
  }
  if (minute < sunsetStart) return curve.daytime;
  return lerp(
    curve.daytime,
    curve.nightFloor,
    minute - sunsetStart,
    sunsetEnd - sunsetStart,
  );
}

/** Colour temperature at a minute, per `curve.rs::color_temp_at`. */
export function colorTempAt(curve: Curve, minute: number): number {
  const anchors = curve.anchors;
  const first = anchors[0];
  const last = anchors[anchors.length - 1];
  if (minute <= first.minute) return first.kelvin;
  if (minute >= last.minute) return last.kelvin;
  for (let i = 0; i < anchors.length - 1; i++) {
    const a = anchors[i];
    const b = anchors[i + 1];
    if (minute >= a.minute && minute < b.minute) {
      return lerp(a.kelvin, b.kelvin, minute - a.minute, b.minute - a.minute);
    }
  }
  return last.kelvin;
}

/** Integer lerp, truncating like the Rust it mirrors. */
function lerp(from: number, to: number, numerator: number, denominator: number) {
  if (denominator === 0) return from;
  return from + Math.trunc(((to - from) * numerator) / denominator);
}

/** True inside `[start, end)`, wrapping across the week boundary. */
export function isPaused(
  pause: { start: Instant; end: Instant },
  at: Date,
): boolean {
  const week = (i: Instant) => i.day * 1440 + i.minute;
  const s = week(pause.start);
  const e = week(pause.end);
  // JS weeks start on Sunday; `WeekInstant` starts on Monday.
  const n = ((at.getDay() + 6) % 7) * 1440 + at.getHours() * 60 + at.getMinutes();
  return s <= e ? s <= n && n < e : n >= s || n < e;
}

const DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

function formatInstant(instant: Instant): string {
  const hours = String(Math.floor(instant.minute / 60)).padStart(2, "0");
  const minutes = String(instant.minute % 60).padStart(2, "0");
  return `${DAYS[instant.day]} ${hours}:${minutes}`;
}

/** Read the curve out of the effective config, or `null` if incomplete. */
export function readCurve(lighting: unknown): Curve | null {
  if (typeof lighting !== "object" || lighting === null) return null;
  const table = lighting as Record<string, unknown>;

  const morningStart = minuteOf(table.morning_start);
  const morningEnd = minuteOf(table.morning_end);
  const sunsetStart = minuteOf(table.sunset_start);
  const sunsetEnd = minuteOf(table.sunset_end);
  const nightFloor = table.night_floor_brightness;
  const daytime = table.daytime_brightness;

  if (
    morningStart === null ||
    morningEnd === null ||
    sunsetStart === null ||
    sunsetEnd === null ||
    typeof nightFloor !== "number" ||
    typeof daytime !== "number"
  ) {
    return null;
  }

  const anchors = Array.isArray(table.color_temp_anchors)
    ? table.color_temp_anchors
        .map((raw) => {
          const anchor = raw as Record<string, unknown>;
          const minute = minuteOf(anchor.time);
          return minute !== null && typeof anchor.kelvin === "number"
            ? { minute, kelvin: anchor.kelvin }
            : null;
        })
        .filter((a): a is Anchor => a !== null)
    : [];
  if (anchors.length === 0) return null;

  const start = instantOf(table.curve_pause_start);
  const end = instantOf(table.curve_pause_end);

  return {
    morningStart,
    morningEnd,
    sunsetStart,
    sunsetEnd,
    nightFloor,
    daytime,
    anchors,
    pause: start && end ? { start, end } : undefined,
  };
}

function minuteOf(value: unknown): number | null {
  if (typeof value !== "string") return null;
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return hours * 60 + minutes;
}

function instantOf(value: unknown): Instant | null {
  if (typeof value !== "string") return null;
  const [day, time] = value.trim().split(/\s+/);
  const minute = minuteOf(time);
  if (minute === null || !day) return null;
  const index = DAYS.findIndex((name) =>
    name.toLowerCase().startsWith(day.toLowerCase().slice(0, 3)),
  );
  return index === -1 ? null : { day: index, minute };
}

/**
 * Kelvin as a screen colour — Tanner Helland's blackbody approximation,
 * which is the one everything from photo software to WLED uses.
 */
function kelvinToColor(kelvin: number): string {
  const t = Math.min(Math.max(kelvin, 1000), 40000) / 100;
  const red =
    t <= 66 ? 255 : 329.698727446 * Math.pow(t - 60, -0.1332047592);
  const green =
    t <= 66
      ? 99.4708025861 * Math.log(t) - 161.1195681661
      : 288.1221695283 * Math.pow(t - 60, -0.0755148492);
  const blue =
    t >= 66
      ? 255
      : t <= 19
        ? 0
        : 138.5177312231 * Math.log(t - 10) - 305.0447927307;
  const channel = (value: number) =>
    Math.round(Math.min(Math.max(value, 0), 255));
  return `rgb(${channel(red)}, ${channel(green)}, ${channel(blue)})`;
}
