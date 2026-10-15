import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { TrainingOverview } from "@/lib/api";
import { relativeTime } from "@/lib/skills";

export const TRAINING = ["me-training"];

/** Where Health Auto Export posts: this Niles, as the browser reached it. */
export function ingestUrl(origin: string = window.location.origin): string {
  return `${origin}/training/ingest`;
}

const plural = (n: number, one: string, many = `${one}s`) =>
  `${n} ${n === 1 ? one : many}`;

/** "Last data 10 minutes ago · 42 runs · 30 days of wellness". */
export function receivedText(o: TrainingOverview, now: Date = new Date()): string {
  if (!o.last_received_at) {
    return o.key_made_at ? "Nothing has arrived yet." : "";
  }
  return [
    `Last data ${relativeTime(o.last_received_at, now)}`,
    plural(o.runs, "run"),
    `${plural(o.wellness_days, "day")} of sleep, heart and weight`,
  ].join(" · ");
}

/**
 * The person's runs, sleep, heart rate and weight, from their iPhone.
 *
 * Garmin has no way in for one person, so the watch and the scale
 * write to Apple Health and the Health Auto Export app posts that here,
 * with a key that opens nothing else. The key is shown the once it is
 * made: Niles keeps only a hash of it.
 */
export function TrainingCard() {
  const queryClient = useQueryClient();
  const overview = useQuery({ queryKey: TRAINING, queryFn: api.training });
  const [fresh, setFresh] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<"replace" | "revoke" | null>(null);
  const refresh = () => queryClient.invalidateQueries({ queryKey: TRAINING });
  const make = useMutation({
    mutationFn: api.makeTrainingKey,
    onSuccess: ({ key }) => {
      setFresh(key);
      setConfirming(null);
      refresh();
    },
  });
  const revoke = useMutation({
    mutationFn: api.revokeTrainingKey,
    onSuccess: () => {
      setFresh(null);
      setConfirming(null);
      refresh();
    },
  });
  const busy = make.isPending || revoke.isPending;
  const error = make.error?.message ?? revoke.error?.message;

  return (
    <Card>
      <CardHeader>
        <CardTitle>Training: connect your iPhone</CardTitle>
        <CardDescription>
          Your runs, sleep, heart rate and weight, from your Garmin through Apple
          Health. Yours alone: Niles reads them only for you.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {overview.isPending ? (
          <Skeleton className="h-10 w-full" />
        ) : overview.isError ? (
          <p className="text-destructive text-sm">{overview.error.message}</p>
        ) : (
          <>
            {receivedText(overview.data) && (
              <p className="text-sm" role="status">
                {receivedText(overview.data)}
              </p>
            )}
            {fresh ? (
              <div className="flex flex-col gap-2">
                <CopyRow label="Your key" value={fresh} />
                <p className="text-muted-foreground text-xs">
                  Shown this once. Put it in Health Auto Export now; Niles keeps
                  only a fingerprint of it.
                </p>
              </div>
            ) : overview.data.key_made_at ? (
              <div className="flex flex-wrap items-center justify-between gap-3">
                <p className="text-muted-foreground text-sm">
                  A key is in use, made{" "}
                  {new Date(overview.data.key_made_at).toLocaleDateString()}.
                </p>
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirming("replace")}
                  >
                    Replace
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirming("revoke")}
                  >
                    Revoke
                  </Button>
                </div>
              </div>
            ) : (
              <div className="flex items-center justify-between gap-3">
                <p className="text-muted-foreground text-sm">
                  Make a key for your phone to send with.
                </p>
                <Button variant="outline" disabled={busy} onClick={() => make.mutate()}>
                  Create key
                </Button>
              </div>
            )}
          </>
        )}
        {error && !confirming && <p className="text-destructive text-sm">{error}</p>}
        <Steps />
      </CardContent>
      <Confirm
        what={confirming}
        busy={busy}
        error={error}
        onConfirm={() => (confirming === "revoke" ? revoke.mutate() : make.mutate())}
        onClose={() => {
          make.reset();
          revoke.reset();
          setConfirming(null);
        }}
      />
    </Card>
  );
}

function Steps() {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-muted-foreground px-1 text-xs font-medium tracking-wide uppercase">
        Setting it up
      </h3>
      <CopyRow label="URL" value={ingestUrl()} />
      <ol className="text-muted-foreground flex list-decimal flex-col gap-1.5 pl-5 text-sm">
        <li>
          In Garmin Connect: Settings → Connected Apps → Apple Health, and allow
          everything. Your Index S2 weighings go the same way.
        </li>
        <li>
          In Health Auto Export: Automations → new → REST API. URL: the one above.
          Under Headers add <code className="font-mono">Authorization</code> with
          the value <code className="font-mono">Bearer</code> and your key.
          Export format JSON.
        </li>
        <li>
          Data type Health Metrics, with Summarize Data on: Resting Heart Rate,
          Heart Rate Variability, Sleep Analysis, Weight &amp; Body Mass and Body
          Fat Percentage.
        </li>
        <li>
          A second automation the same way, with data type Workouts and export
          version 2.
        </li>
        <li>
          Sync every hour or so. iPhone runs it while the phone is unlocked, so
          the odd gap is normal.
        </li>
      </ol>
    </section>
  );
}

function CopyRow({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);

  const copy = async () => {
    setFailed(false);
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
    } catch {
      // No clipboard over plain HTTP; it is on screen to select instead.
      setFailed(true);
    }
  };

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <code
          aria-label={label}
          className="bg-muted min-w-0 flex-1 rounded-md px-3 py-2 font-mono text-xs break-all select-all"
        >
          {value}
        </code>
        <Button variant="ghost" size="icon" aria-label={`Copy ${label.toLowerCase()}`} onClick={copy}>
          {copied ? <Check /> : <Copy />}
        </Button>
      </div>
      {(copied || failed) && (
        <span className="text-muted-foreground text-xs" role="status">
          {copied ? "Copied" : "Could not copy; select it and copy it by hand"}
        </span>
      )}
    </div>
  );
}

function Confirm({
  what,
  busy,
  error,
  onConfirm,
  onClose,
}: {
  what: "replace" | "revoke" | null;
  busy: boolean;
  error?: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog
      open={what !== null}
      onOpenChange={(next: boolean) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="gap-0 p-0">
        <div className="border-border border-b px-4 py-3">
          <DialogTitle>{what === "revoke" ? "Revoke the key?" : "Replace the key?"}</DialogTitle>
          <DialogDescription>
            {what === "revoke"
              ? "Your phone stops sending to Niles. What it already sent is kept."
              : "Your phone stops getting in until it has the new key."}
          </DialogDescription>
        </div>
        <DialogBody className="flex flex-col gap-3">
          {error && <p className="text-destructive text-sm">{error}</p>}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>
              Keep it
            </Button>
            <Button variant="destructive" disabled={busy} onClick={onConfirm}>
              {what === "revoke" ? "Revoke" : "Make a new key"}
            </Button>
          </div>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
