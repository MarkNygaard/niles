import { useEffect, useState } from "react";
import { Plus, RotateCcw, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { kelvinToCss } from "@/components/ColorField";
import { cn } from "@/lib/utils";

/** One row of `[[lighting.color_temp_anchors]]`. */
export interface Anchor {
  time: string;
  kelvin: number;
}

/**
 * The curve Niles runs when nobody has set one, as `default_anchors()`
 * in `crates/niles-config/src/lighting.rs` has it.
 *
 * Held here because `GET /config` only reports defaults for the two
 * model names. Read after `view.defaults`, so the server's answer wins
 * once it sends one.
 */
export const SHIPPED_ANCHORS: Anchor[] = [
  { time: "00:00", kelvin: 2000 },
  { time: "05:45", kelvin: 2000 },
  { time: "06:30", kelvin: 2700 },
  { time: "12:00", kelvin: 4500 },
  { time: "21:30", kelvin: 2700 },
  { time: "23:00", kelvin: 2000 },
  { time: "23:59", kelvin: 2000 },
];

/** `CurveConfig::validate` in `niles-scheduler`. */
export const KELVIN_MIN = 1000;
export const KELVIN_MAX = 10000;

export interface AnchorDraft {
  time: string;
  kelvin: string;
}

const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;

/**
 * What is wrong with each point, or `null` for one that will save.
 *
 * The server would refuse all of these too, but by then the message is
 * about the whole section and the reader has to find the row it means.
 */
export function anchorProblems(drafts: AnchorDraft[]): (string | null)[] {
  return drafts.map((draft, index) => {
    if (!TIME.test(draft.time.trim())) return "Use HH:MM, like 06:30.";
    const kelvin = draft.kelvin.trim();
    if (!/^\d+$/.test(kelvin) || Number(kelvin) < KELVIN_MIN || Number(kelvin) > KELVIN_MAX) {
      return `Kelvin must be a whole number from ${KELVIN_MIN} to ${KELVIN_MAX}.`;
    }
    // The scheduler wants strictly increasing times, so two points at
    // one minute is not a tie it breaks but a config it refuses.
    const first = drafts.findIndex((d) => d.time.trim() === draft.time.trim());
    if (first !== index) return `Another point is already at ${draft.time.trim()}.`;
    return null;
  });
}

/**
 * The drafts as the list to send, earliest first.
 *
 * Sorted here rather than kept sorted while typing: a row that jumps
 * the moment its hour is typed is a row that loses the cursor.
 */
export function sortedAnchors(drafts: AnchorDraft[]): Anchor[] {
  return drafts
    .map((d) => ({ time: d.time.trim(), kelvin: Number(d.kelvin.trim()) }))
    .sort((a, b) => a.time.localeCompare(b.time));
}

const draftsOf = (anchors: Anchor[]): AnchorDraft[] =>
  anchors.map((a) => ({ time: a.time, kelvin: String(a.kelvin) }));

const signatureOf = (drafts: AnchorDraft[]) =>
  drafts.map((d) => `${d.time.trim()}=${d.kelvin.trim()}`).join(",");

export interface AnchorEditorProps {
  /** What the curve runs now: the config's list, or the shipped one. */
  anchors: Anchor[];
  /** What Niles ships with, for the line that says how this differs. */
  shipped: Anchor[];
  /** Whether the list was set from the app, which is what a reset drops. */
  overridden: boolean;
  /** False for a boot-only section; `lighting` is re-read every tick. */
  hot: boolean;
  saving?: boolean;
  /** The server's refusal of the last save, verbatim. */
  error?: string;
  onSave: (anchors: Anchor[]) => void;
  onReset: () => void;
  /** The list as edited, while it is valid and unsaved; `null` otherwise. */
  onPreview?: (anchors: Anchor[] | null) => void;
}

/**
 * The colour temperature the curve passes through over a day.
 *
 * The whole list is one setting, written in one go: a point means
 * nothing without its neighbours, since the colour between two of them
 * is drawn from both.
 */
export function AnchorEditor({
  anchors,
  shipped,
  overridden,
  hot,
  saving,
  error,
  onSave,
  onReset,
  onPreview,
}: AnchorEditorProps) {
  const server = draftsOf(anchors);
  const serverSignature = signatureOf(server);
  const [drafts, setDrafts] = useState(server);

  // Adopt the server's list whenever it changes underneath us — after a
  // save, a reset, an undo, or a change made by voice.
  useEffect(() => {
    setDrafts(server);
  }, [serverSignature]);

  const problems = anchorProblems(drafts);
  const valid = drafts.length > 0 && problems.every((p) => p === null);
  const changed = signatureOf(drafts) !== serverSignature;
  const draftSignature = signatureOf(drafts);

  useEffect(() => {
    onPreview?.(changed && valid ? sortedAnchors(drafts) : null);
  }, [draftSignature, serverSignature, valid]);

  const differsFromShipped =
    signatureOf(draftsOf(anchors)) !== signatureOf(draftsOf(shipped));

  function edit(index: number, field: keyof AnchorDraft, value: string) {
    setDrafts(drafts.map((d, i) => (i === index ? { ...d, [field]: value } : d)));
  }

  function save() {
    if (!changed || !valid || saving) return;
    onSave(sortedAnchors(drafts));
  }

  return (
    <div className="grid gap-x-8 gap-y-3 py-4 sm:grid-cols-[minmax(190px,240px)_minmax(0,1fr)]">
      <div className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium">Colour temperature</span>
          {!hot && (
            <Badge
              variant="outline"
              title="Stored, but only read when Niles starts — needs a restart to take effect"
            >
              restart required
            </Badge>
          )}
        </div>
        <p className="text-muted-foreground text-xs">
          The points the colour passes through over the day. Between two
          points it shades from one to the next; before the first and after
          the last it holds.
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <ul className="flex flex-col gap-2">
          {drafts.map((draft, index) => {
            const n = index + 1;
            const problem = problems[index];
            const kelvin = Number(draft.kelvin);
            return (
              <li key={index} className="flex flex-col gap-1">
                <div className="flex items-center gap-2">
                  <span
                    aria-hidden
                    className="border-border size-4 shrink-0 rounded-full border"
                    style={{
                      background:
                        kelvin >= KELVIN_MIN && kelvin <= KELVIN_MAX
                          ? kelvinToCss(kelvin)
                          : "transparent",
                    }}
                  />
                  <Input
                    aria-label={`Point ${n} time`}
                    value={draft.time}
                    placeholder="HH:MM"
                    disabled={saving}
                    aria-invalid={problem ? true : undefined}
                    onChange={(e) => edit(index, "time", e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && save()}
                    className="w-24 font-mono"
                  />
                  <Input
                    aria-label={`Point ${n} kelvin`}
                    value={draft.kelvin}
                    inputMode="numeric"
                    placeholder="2700"
                    disabled={saving}
                    aria-invalid={problem ? true : undefined}
                    onChange={(e) => edit(index, "kelvin", e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && save()}
                    className="w-24 font-mono"
                  />
                  <span className="text-muted-foreground text-xs">K</span>
                  <Button
                    size="icon"
                    variant="ghost"
                    aria-label={`Remove point ${n}`}
                    // The curve needs at least one point to read a colour
                    // from, and the server refuses an empty list.
                    disabled={saving || drafts.length === 1}
                    onClick={() => setDrafts(drafts.filter((_, i) => i !== index))}
                  >
                    <X />
                  </Button>
                </div>
                {problem && <p className="text-destructive text-xs">{problem}</p>}
              </li>
            );
          })}
        </ul>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            disabled={saving}
            onClick={() => setDrafts([...drafts, { time: "", kelvin: "" }])}
          >
            <Plus /> Add a point
          </Button>
          <div className="ml-auto flex items-center gap-1">
            {changed && (
              <Button variant="ghost" disabled={saving} onClick={() => setDrafts(server)}>
                Discard
              </Button>
            )}
            <Button
              onClick={save}
              disabled={!changed || !valid || saving}
              className={cn(!changed && "invisible")}
            >
              Save
            </Button>
          </div>
        </div>

        {differsFromShipped && (
          <p className="text-muted-foreground text-xs">
            Niles ships with{" "}
            <span className="font-mono">
              {shipped.map((a) => `${a.time} ${a.kelvin}K`).join(" · ")}
            </span>
            .
          </p>
        )}

        {overridden && (
          <div>
            <Button variant="outline" size="sm" disabled={saving} onClick={onReset}>
              <RotateCcw /> Reset to Niles&apos;s defaults
            </Button>
          </div>
        )}

        {error && <p className="text-destructive text-xs">{error}</p>}
      </div>
    </div>
  );
}
