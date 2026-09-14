import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import type { Setting } from "@/components/SettingRow";
import { cn } from "@/lib/utils";

/** The seven, in the order a week is read rather than alphabetically. */
const DAYS = [
  { id: "mon", label: "M", name: "Monday" },
  { id: "tue", label: "T", name: "Tuesday" },
  { id: "wed", label: "W", name: "Wednesday" },
  { id: "thu", label: "T", name: "Thursday" },
  { id: "fri", label: "F", name: "Friday" },
  { id: "sat", label: "S", name: "Saturday" },
  { id: "sun", label: "S", name: "Sunday" },
] as const;

export interface MorningCardProps {
  enabled: boolean;
  /** Lowercase three-letter days, as the config stores them. */
  fireDays: string[];
  /** The shared window, for the line that says when it runs. */
  start?: string;
  end?: string;
  saving?: boolean;
  error?: string;
  onToggle: (on: boolean) => void;
  onDays: (days: string[]) => void;
  /** Rendered by the panel, which owns saving and undo for a row. */
  row: (spec: {
    label: string;
    description: string;
    settings: Setting[];
  }) => React.ReactNode;
}

/**
 * The wake-up ramp.
 *
 * Separate from the curve on purpose, and not gated on it: somebody who
 * switches the curve off to stop it touching their lights still wants
 * to be woken up. The one thing the two share is the window, which is
 * why it is stated here rather than left invisible — with the curve
 * switched off, its rows are hidden and this was the only feature still
 * using those times.
 *
 * Off is a flag rather than a deleted section, so a fortnight's holiday
 * does not lose which lights were in it.
 */
export function MorningCard({
  enabled,
  fireDays,
  start,
  end,
  saving,
  error,
  onToggle,
  onDays,
  row,
}: MorningCardProps) {
  function toggleDay(picked: string) {
    // Rebuilt from DAYS rather than appended to, so the stored list
    // stays in week order however it is clicked.
    onDays(
      DAYS.map((day) => day.id).filter((id) =>
        id === picked ? !fireDays.includes(id) : fireDays.includes(id),
      ),
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Morning routine</CardTitle>
        <CardDescription>
          Lights come on at nothing and ramp to full across the morning
          window, whether or not the curve is running.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <label className="flex items-center justify-between gap-4">
          <span className="min-w-0">
            <span className="block text-sm font-medium">Wake up with light</span>
            <span className="text-muted-foreground block text-xs">
              {start && end
                ? `Runs ${start} → ${end}, the same window as the curve's morning ramp.`
                : "Runs across the curve's morning window."}
            </span>
          </span>
          <Switch
            checked={enabled}
            disabled={saving}
            onCheckedChange={onToggle}
          />
        </label>

        {/* Hidden rather than greyed out, as with the curve: a wall of
            disabled settings is a page asking to be read and ignored.
            Nothing is lost — switching it off keeps every value. */}
        {enabled && (
          <div className="divide-border divide-y">
            <div className="grid gap-x-8 gap-y-3 py-4 sm:grid-cols-[minmax(190px,240px)_minmax(0,1fr)]">
              <div className="flex flex-col gap-1">
                <span className="text-sm font-medium">Days</span>
                <p className="text-muted-foreground text-xs">
                  {fireDays.length === 0
                    ? "None picked, so it never fires."
                    : "It only fires on these."}
                </p>
              </div>
              <div
                role="group"
                aria-label="Days it fires on"
                className="flex flex-wrap gap-1.5"
              >
                {DAYS.map((day) => {
                  const on = fireDays.includes(day.id);
                  return (
                    <button
                      key={day.id}
                      type="button"
                      aria-pressed={on}
                      aria-label={day.name}
                      disabled={saving}
                      onClick={() => toggleDay(day.id)}
                      className={cn(
                        "ring-border size-9 rounded-full text-sm ring-1 transition-colors",
                        "hover:bg-background focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                        on
                          ? "bg-background font-medium"
                          : "text-muted-foreground",
                      )}
                    >
                      {day.label}
                    </button>
                  );
                })}
              </div>
            </div>

            {row({
              label: "Morning window",
              description:
                "Shared with the curve's morning ramp — changing it here changes it there.",
              settings: [
                {
                  path: "lighting.morning_start",
                  kind: "text",
                  caption: "starts",
                  width: "w-32",
                },
                {
                  path: "lighting.morning_end",
                  kind: "text",
                  caption: "ends",
                  width: "w-32",
                },
              ],
            })}

            {row({
              label: "Lights",
              description:
                "Leave empty for every light the curve drives, plus any plugs, which switch on at the end rather than ramping.",
              settings: [
                {
                  path: "lighting.morning_routine.target_devices",
                  kind: "devices",
                  caption: "pick from the lights Niles knows about",
                  width: "min-w-72 flex-1",
                },
              ],
            })}

            {row({
              label: "Except these",
              description:
                "Applied after the list above, so an empty list plus exclusions means every light but these.",
              settings: [
                {
                  path: "lighting.morning_routine.exclude_devices",
                  kind: "devices",
                  caption: "lights to leave out",
                  width: "min-w-72 flex-1",
                },
              ],
            })}
          </div>
        )}

        {error && <p className="text-destructive text-sm">{error}</p>}
      </CardContent>
    </Card>
  );
}
