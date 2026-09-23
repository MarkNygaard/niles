import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import type { Capture } from "@/lib/api";

export interface CaptureCardProps {
  enabled: boolean;
  captures?: Capture[];
  saving?: boolean;
  onChange: (enabled: boolean) => void;
  onClear?: () => void;
}

/**
 * How many were kept, and how many were nothing.
 *
 * The second number is the point. A satellite that wakes fifty times a
 * day and means it twice has a wake-word problem, and the only honest
 * way to know that is to count. It is also the training set: the ones
 * nothing was said back to are the negatives.
 */
export function captureSummary(captures: Capture[]): string {
  if (captures.length === 0) return "Nothing kept yet.";
  const dropped = captures.filter((c) => c.outcome === "dropped").length;
  const mb = captures.reduce((sum, c) => sum + c.bytes, 0) / 1_048_576;
  return `${captures.length} kept · ${dropped} came to nothing · ${mb.toFixed(1)} MB`;
}

/**
 * Keeping what the satellite heard.
 *
 * Off by default and plainly described, because it is a microphone
 * writing down a living room. What makes it worth offering at all is
 * that a false wake is only fixable with examples of it: a week of a
 * real room, with its real television, is worth more for training a
 * wake word than any number of synthesised negatives.
 */
export function CaptureCard({
  enabled,
  captures,
  saving,
  onChange,
  onClear,
}: CaptureCardProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Keep what it hears</CardTitle>
        <CardDescription>
          Every time a satellite wakes, keep the recording. It is how a wake
          word gets better at ignoring the television — the failures are the
          training data, and they cannot be invented.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <label className="flex items-center justify-between gap-4">
          <span className="min-w-0">
            <span className="block text-sm font-medium">
              Keep wake recordings
            </span>
            <span className="text-muted-foreground block text-xs">
              A microphone writing down this room, kept in the database and
              capped at the most recent 500. Off unless you switch it on.
            </span>
          </span>
          <Switch
            checked={enabled}
            disabled={saving}
            onCheckedChange={(next) => onChange(next)}
          />
        </label>

        {captures !== undefined && enabled && (
          <div className="flex items-center justify-between gap-3 border-t pt-3">
            <span className="text-muted-foreground text-xs">
              {captureSummary(captures)}
            </span>
            {onClear && captures.length > 0 && (
              <Button variant="outline" size="sm" disabled={saving} onClick={onClear}>
                Delete all
              </Button>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
