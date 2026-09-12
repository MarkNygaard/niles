import { useEffect, useRef, useState } from "react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

export interface ColorFieldProps {
  id: string;
  "aria-label": string;
  /** `#rrggbb`, or empty when nothing is set. */
  value: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}

const SIZE = 176;
const RADIUS = SIZE / 2;

/**
 * Pick an ambient colour off a wheel.
 *
 * Hue around, saturation outward, exactly as every light app draws it —
 * the point is to find "warm orange" by looking, not by knowing that it
 * spells `#ff8000`. Brightness is deliberately absent: ambient lights
 * already have their own brightness setting, and a value slider here
 * would be a second control for the same thing.
 *
 * The hex field beside it stays, because a colour you liked is worth
 * being able to write down and type back.
 */
export function ColorField({
  id,
  "aria-label": ariaLabel,
  value,
  disabled,
  onChange,
}: ColorFieldProps) {
  const canvas = useRef<HTMLCanvasElement | null>(null);
  const [draft, setDraft] = useState(value);

  useEffect(() => setDraft(value), [value]);

  // Painted once: the wheel is fixed, only the marker moves.
  useEffect(() => {
    const context = canvas.current?.getContext("2d");
    if (!context) return;
    const image = context.createImageData(SIZE, SIZE);
    for (let y = 0; y < SIZE; y++) {
      for (let x = 0; x < SIZE; x++) {
        const dx = x - RADIUS;
        const dy = y - RADIUS;
        const distance = Math.sqrt(dx * dx + dy * dy);
        const offset = (y * SIZE + x) * 4;
        if (distance > RADIUS) {
          image.data[offset + 3] = 0;
          continue;
        }
        const hue = ((Math.atan2(dy, dx) * 180) / Math.PI + 360) % 360;
        const [r, g, b] = hsvToRgb(hue, Math.min(distance / RADIUS, 1), 1);
        image.data[offset] = r;
        image.data[offset + 1] = g;
        image.data[offset + 2] = b;
        // Feather the rim so the circle doesn't look cut out.
        image.data[offset + 3] = Math.round(255 * Math.min(1, RADIUS - distance));
      }
    }
    context.putImageData(image, 0, 0);
  }, []);

  const rgb = parseHex(value);
  const marker = rgb ? positionOf(rgb) : null;

  function pick(event: React.PointerEvent<HTMLCanvasElement>) {
    if (disabled) return;
    const box = event.currentTarget.getBoundingClientRect();
    const dx = ((event.clientX - box.left) / box.width) * SIZE - RADIUS;
    const dy = ((event.clientY - box.top) / box.height) * SIZE - RADIUS;
    const distance = Math.sqrt(dx * dx + dy * dy);
    const hue = ((Math.atan2(dy, dx) * 180) / Math.PI + 360) % 360;
    // Dragging past the rim keeps the hue at full saturation rather
    // than stopping dead, which is how these are expected to behave.
    const [r, g, b] = hsvToRgb(hue, Math.min(distance / RADIUS, 1), 1);
    onChange(toHex([r, g, b]));
  }

  return (
    <div className="flex flex-wrap items-start gap-4">
      <div className="relative" style={{ width: SIZE, height: SIZE }}>
        <canvas
          ref={canvas}
          width={SIZE}
          height={SIZE}
          role="slider"
          tabIndex={disabled ? -1 : 0}
          aria-label={ariaLabel}
          aria-valuetext={value || "not set"}
          className={cn(
            "rounded-full",
            disabled ? "cursor-not-allowed opacity-50" : "cursor-crosshair",
          )}
          onPointerDown={(e) => {
            e.currentTarget.setPointerCapture(e.pointerId);
            pick(e);
          }}
          onPointerMove={(e) => {
            if (e.buttons === 1) pick(e);
          }}
        />
        {marker && (
          <span
            aria-hidden
            className="border-background pointer-events-none absolute size-5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 shadow"
            style={{
              left: marker.x,
              top: marker.y,
              background: value,
            }}
          />
        )}
      </div>

      <div className="flex flex-col gap-1">
        <Input
          id={id}
          value={draft}
          placeholder="not set"
          disabled={disabled}
          spellCheck={false}
          className="w-32 font-mono"
          onChange={(e) => {
            setDraft(e.target.value);
            // Only a complete colour is worth sending; half-typed hex
            // would be rejected on every keystroke.
            if (parseHex(e.target.value)) onChange(normalize(e.target.value));
          }}
        />
        <span className="text-muted-foreground/70 pl-0.5 text-[11px]">hex</span>
      </div>
    </div>
  );
}

function positionOf([r, g, b]: [number, number, number]) {
  const { hue, saturation } = rgbToHsv(r, g, b);
  const angle = (hue * Math.PI) / 180;
  return {
    x: RADIUS + Math.cos(angle) * saturation * RADIUS,
    y: RADIUS + Math.sin(angle) * saturation * RADIUS,
  };
}

/** `#rrggbb` to channels, or `null` if it isn't one. */
export function parseHex(raw: string): [number, number, number] | null {
  const hex = raw.trim().replace(/^#/, "");
  if (!/^[0-9a-fA-F]{6}$/.test(hex)) return null;
  return [
    parseInt(hex.slice(0, 2), 16),
    parseInt(hex.slice(2, 4), 16),
    parseInt(hex.slice(4, 6), 16),
  ];
}

function normalize(raw: string): string {
  const rgb = parseHex(raw);
  return rgb ? toHex(rgb) : raw;
}

export function toHex([r, g, b]: [number, number, number]): string {
  return `#${[r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

export function hsvToRgb(
  hue: number,
  saturation: number,
  value: number,
): [number, number, number] {
  const c = value * saturation;
  const x = c * (1 - Math.abs(((hue / 60) % 2) - 1));
  const m = value - c;
  const [r, g, b] =
    hue < 60
      ? [c, x, 0]
      : hue < 120
        ? [x, c, 0]
        : hue < 180
          ? [0, c, x]
          : hue < 240
            ? [0, x, c]
            : hue < 300
              ? [x, 0, c]
              : [c, 0, x];
  return [
    Math.round((r + m) * 255),
    Math.round((g + m) * 255),
    Math.round((b + m) * 255),
  ];
}

export function rgbToHsv(r: number, g: number, b: number) {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  let hue = 0;
  if (delta !== 0) {
    if (max === r) hue = ((g - b) / delta) % 6;
    else if (max === g) hue = (b - r) / delta + 2;
    else hue = (r - g) / delta + 4;
  }
  return {
    hue: (hue * 60 + 360) % 360,
    saturation: max === 0 ? 0 : delta / max,
    value: max / 255,
  };
}
