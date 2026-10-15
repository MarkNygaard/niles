import { useState } from "react";
import { cn } from "@/lib/utils";

const COLOURS = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)"];

const WIDTH = 600;
const HEIGHT = 200;
const LEFT = 40;
const RIGHT = 8;
const TOP = 8;
const BOTTOM = 24;

/** Round numbers for the y axis from 0 (or below, for negatives) to
    just above the largest value. */
export function yTicks(values: number[]): number[] {
  const finite = values.filter(Number.isFinite);
  const high = Math.max(0, ...finite);
  const low = Math.min(0, ...finite);
  const span = high - low || 1;
  const raw = span / 3;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * magnitude).find((s) => s >= raw) ?? raw;
  const ticks: number[] = [];
  for (let t = Math.floor(low / step) * step; t < high + step; t += step) {
    ticks.push(Number(t.toFixed(10)));
    if (t >= high) break;
  }
  return ticks;
}

/** Which labels along the bottom are written: the first, the last, and
    a few between, so they never run into each other. */
export function shownLabels(count: number, room = 6): number[] {
  if (count <= room) return [...Array(count).keys()];
  const every = Math.ceil((count - 1) / (room - 1));
  const shown: number[] = [];
  for (let i = 0; i < count - 1; i += every) shown.push(i);
  shown.push(count - 1);
  return shown;
}

export interface SurfaceChartProps {
  kind: "line" | "bar";
  labels: string[];
  series: { name: string; values: number[] }[];
  unit: string | null;
}

/**
 * A plugin's chart, in the app's series colours: a line or bar chart,
 * one label per point, up to four series on one axis. Thin marks and a
 * recessive grid; a legend once there are two series, a hover readout,
 * and the numbers as a table for a screen reader.
 */
export function SurfaceChart({ kind, labels, series, unit }: SurfaceChartProps) {
  const [hover, setHover] = useState<number | null>(null);
  const count = labels.length;
  const ticks = yTicks(series.flatMap((s) => s.values));
  const low = ticks[0] ?? 0;
  const high = ticks[ticks.length - 1] ?? 1;
  const plotWidth = WIDTH - LEFT - RIGHT;
  const plotHeight = HEIGHT - TOP - BOTTOM;
  const y = (v: number) => TOP + (1 - (v - low) / (high - low || 1)) * plotHeight;
  const slot = plotWidth / Math.max(count, 1);
  const x = (i: number) =>
    kind === "bar" ? LEFT + slot * (i + 0.5) : LEFT + (count > 1 ? (i / (count - 1)) * plotWidth : plotWidth / 2);
  const suffix = unit ? ` ${unit}` : "";
  const barWidth = Math.max(1, (slot - 2) / Math.max(series.length, 1));

  function pick(clientX: number, box: DOMRect) {
    const at = ((clientX - box.left) / box.width) * WIDTH;
    const i =
      kind === "bar"
        ? Math.floor((at - LEFT) / slot)
        : Math.round(((at - LEFT) / plotWidth) * (count - 1));
    setHover(i >= 0 && i < count ? i : null);
  }

  return (
    <figure className="flex flex-col gap-2">
      {series.length > 1 && (
        <ul className="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-xs">
          {series.map((s, n) => (
            <li key={s.name} className="flex items-center gap-1.5">
              <span
                aria-hidden
                className="size-2.5 rounded-full"
                style={{ background: COLOURS[n] }}
              />
              {s.name}
            </li>
          ))}
        </ul>
      )}
      <p className="text-muted-foreground min-h-4 text-xs tabular-nums" aria-hidden>
        {hover !== null &&
          `${labels[hover]}: ${series
            .map((s) => `${series.length > 1 ? `${s.name} ` : ""}${s.values[hover]}${suffix}`)
            .join(", ")}`}
      </p>
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="h-auto w-full touch-none"
        role="img"
        aria-label={`${kind === "bar" ? "Bar" : "Line"} chart of ${series
          .map((s) => s.name)
          .join(", ")}${unit ? `, in ${unit}` : ""}`}
        onPointerMove={(e) => pick(e.clientX, e.currentTarget.getBoundingClientRect())}
        onPointerLeave={() => setHover(null)}
      >
        {ticks.map((t) => (
          <g key={t}>
            <line
              x1={LEFT}
              x2={WIDTH - RIGHT}
              y1={y(t)}
              y2={y(t)}
              className="stroke-border"
              strokeWidth={1}
            />
            <text
              x={LEFT - 6}
              y={y(t)}
              textAnchor="end"
              dominantBaseline="middle"
              className="fill-muted-foreground text-[11px]"
            >
              {t}
            </text>
          </g>
        ))}
        {shownLabels(count).map((i) => (
          <text
            key={i}
            x={x(i)}
            y={HEIGHT - 6}
            textAnchor={i === 0 && kind === "line" ? "start" : i === count - 1 && kind === "line" ? "end" : "middle"}
            className="fill-muted-foreground text-[11px]"
          >
            {labels[i]}
          </text>
        ))}
        {hover !== null && (
          <line
            x1={x(hover)}
            x2={x(hover)}
            y1={TOP}
            y2={TOP + plotHeight}
            className="stroke-muted-foreground/40"
            strokeWidth={1}
          />
        )}
        {kind === "bar"
          ? series.map((s, n) =>
              s.values.map((v, i) => {
                const top = y(Math.max(v, 0));
                const bottom = y(Math.min(v, 0));
                return (
                  <rect
                    key={`${n}-${i}`}
                    x={LEFT + slot * i + 1 + barWidth * n}
                    y={top}
                    width={Math.max(barWidth - 2, 1)}
                    height={Math.max(bottom - top, 0)}
                    rx={Math.min(4, barWidth / 3)}
                    style={{ fill: COLOURS[n] }}
                    className={cn(hover !== null && hover !== i && "opacity-60")}
                  />
                );
              }),
            )
          : series.map((s, n) => (
              <g key={s.name}>
                <polyline
                  fill="none"
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                  style={{ stroke: COLOURS[n] }}
                  points={s.values.map((v, i) => `${x(i)},${y(v)}`).join(" ")}
                />
                {hover !== null && Number.isFinite(s.values[hover]) && (
                  <circle
                    cx={x(hover)}
                    cy={y(s.values[hover])}
                    r={4}
                    className="stroke-card"
                    strokeWidth={2}
                    style={{ fill: COLOURS[n] }}
                  />
                )}
              </g>
            ))}
      </svg>
      <table className="sr-only">
        <thead>
          <tr>
            <th scope="col">Point</th>
            {series.map((s) => (
              <th key={s.name} scope="col">
                {s.name}
                {suffix}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {labels.map((label, i) => (
            <tr key={`${label}-${i}`}>
              <th scope="row">{label}</th>
              {series.map((s) => (
                <td key={s.name}>{s.values[i]}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  );
}
