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
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { StravaStatus } from "@/lib/api";
import { relativeTime } from "@/lib/skills";
import { cn } from "@/lib/utils";

export const STRAVA = ["me-strava"];

/** Followed, not fetched: it ends at Strava's consent screen. */
export const CONNECT_URL = "/me/strava/start";

const plural = (n: number, one: string, many = `${one}s`) =>
  `${n} ${n === 1 ? one : many}`;

/** "Last synced 10 minutes ago · 42 activities, 30 runs". */
export function syncText(s: StravaStatus, now: Date = new Date()): string {
  const counts = `${plural(s.activities, "activity", "activities")}, ${plural(s.runs, "run")}`;
  if (!s.last_sync) return s.syncing ? "Fetching the last half year…" : counts;
  return `Last synced ${relativeTime(s.last_sync, now)} · ${counts}`;
}

/** What the sign-in came back saying, from the address it returned to. */
export function outcomeOf(search: string): { ok: boolean; text: string } | null {
  const query = new URLSearchParams(search);
  const error = query.get("strava_error");
  if (error) return { ok: false, text: error };
  if (query.get("strava") === "connected") {
    const name = query.get("name");
    return {
      ok: true,
      text: name ? `Connected as ${name}. Fetching your last half year…` : "Connected.",
    };
  }
  return null;
}

type Confirming = "disconnect" | "remove" | null;

/**
 * The person's workouts, from their own Strava.
 *
 * Each person makes their own Strava API app: a new app can only be
 * used by the athlete who made it. Its secret is kept sealed and never
 * shown again; the card only says whether one is saved.
 */
export function StravaCard() {
  const queryClient = useQueryClient();
  const status = useQuery({
    queryKey: STRAVA,
    queryFn: api.strava,
    // While a sync runs, so the count and "last synced" catch up.
    refetchInterval: (query) => (query.state.data?.syncing ? 3000 : false),
  });
  const settle = () => queryClient.invalidateQueries({ queryKey: STRAVA });
  const [outcome, setOutcome] = useState(() => outcomeOf(window.location.search));
  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState<Confirming>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // Said once, then taken off the address so a refresh does not say it
  // again.
  useEffect(() => {
    if (!outcome) return;
    window.history.replaceState(null, "", window.location.pathname + window.location.hash);
  }, [outcome]);

  const sync = useMutation({ mutationFn: api.syncStrava, onSettled: settle });
  const disconnect = useMutation({
    mutationFn: api.disconnectStrava,
    onSuccess: ({ revoked_at_strava }) => {
      setConfirming(null);
      setNotice(
        revoked_at_strava
          ? null
          : "Niles has forgotten the connection, but Strava could not be told. Revoke it at strava.com/settings/apps.",
      );
    },
    onSettled: settle,
  });
  const remove = useMutation({
    mutationFn: api.removeStravaApp,
    onSuccess: () => setConfirming(null),
    onSettled: settle,
  });
  const busy = sync.isPending || disconnect.isPending || remove.isPending;
  const error = sync.error?.message ?? disconnect.error?.message ?? remove.error?.message;

  const s = status.data;
  return (
    <Card>
      <CardHeader>
        <CardTitle>Strava</CardTitle>
        <CardDescription>
          Your workouts, from your own Strava. Yours alone: Niles reads them only for you.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {outcome && (
          <p
            role="status"
            className={cn("text-sm", outcome.ok ? "text-foreground" : "text-destructive")}
          >
            {outcome.text}
          </p>
        )}
        {notice && <p className="text-muted-foreground text-sm">{notice}</p>}
        {status.isPending ? (
          <Skeleton className="h-10 w-full" />
        ) : status.isError ? (
          <p className="text-destructive text-sm">{status.error.message}</p>
        ) : s?.connected ? (
          <Connected
            status={s}
            busy={busy}
            onSync={() => sync.mutate()}
            onDisconnect={() => setConfirming("disconnect")}
          />
        ) : s?.client_id && !editing ? (
          <div className="flex flex-col gap-3">
            <p className="text-muted-foreground text-sm">
              Your app: Client ID <span className="font-mono">{s.client_id}</span>
              {s.secret_set ? ", secret saved." : ", no secret saved."}
            </p>
            {s.last_error && <p className="text-destructive text-sm">{s.last_error}</p>}
            <div className="flex flex-wrap gap-2">
              <Button
                disabled={busy}
                onClick={() => {
                  setOutcome(null);
                  window.location.assign(CONNECT_URL);
                }}
              >
                Connect Strava
              </Button>
              <Button variant="outline" disabled={busy} onClick={() => setEditing(true)}>
                Change app
              </Button>
              <Button variant="outline" disabled={busy} onClick={() => setConfirming("remove")}>
                Remove app
              </Button>
            </div>
          </div>
        ) : (
          <>
            <Steps />
            <AppForm
              status={s}
              onSaved={() => {
                setEditing(false);
                settle();
              }}
              onCancel={s?.client_id ? () => setEditing(false) : undefined}
            />
          </>
        )}
        {error && !confirming && <p className="text-destructive text-sm">{error}</p>}
      </CardContent>
      <Confirm
        what={confirming}
        busy={busy}
        error={disconnect.error?.message ?? remove.error?.message}
        onConfirm={() => (confirming === "remove" ? remove.mutate() : disconnect.mutate())}
        onClose={() => {
          disconnect.reset();
          remove.reset();
          setConfirming(null);
        }}
      />
    </Card>
  );
}

function Connected({
  status,
  busy,
  onSync,
  onDisconnect,
}: {
  status: StravaStatus;
  busy: boolean;
  onSync: () => void;
  onDisconnect: () => void;
}) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1">
        <p className="text-sm">
          Connected as <span className="font-medium">{status.athlete ?? "you"}</span>
        </p>
        <p className="text-muted-foreground text-sm" role="status">
          {syncText(status)}
        </p>
        {status.syncing && status.last_sync && (
          <p className="text-muted-foreground text-xs">Syncing…</p>
        )}
        {status.paused_until && (
          <p className="text-muted-foreground text-xs">
            Strava's request limit is reached; the rest follows after{" "}
            {new Date(status.paused_until).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
            })}
            .
          </p>
        )}
        {status.last_error && <p className="text-destructive text-sm">{status.last_error}</p>}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" disabled={busy || status.syncing} onClick={onSync}>
          Sync now
        </Button>
        <Button variant="outline" disabled={busy} onClick={onDisconnect}>
          Disconnect
        </Button>
      </div>
    </div>
  );
}

function AppForm({
  status,
  onSaved,
  onCancel,
}: {
  status?: StravaStatus;
  onSaved: () => void;
  onCancel?: () => void;
}) {
  const [clientId, setClientId] = useState(status?.client_id ?? "");
  const [secret, setSecret] = useState("");
  const save = useMutation({ mutationFn: api.saveStravaApp, onSuccess: onSaved });
  const sameApp = Boolean(status?.secret_set) && clientId.trim() === status?.client_id;
  const ready = clientId.trim() !== "" && (secret.trim() !== "" || sameApp);

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!ready) return;
        save.mutate({ client_id: clientId.trim(), client_secret: secret.trim() || undefined });
      }}
    >
      <Input
        aria-label="Client ID"
        inputMode="numeric"
        autoComplete="off"
        placeholder="Client ID"
        value={clientId}
        onChange={(e) => setClientId(e.target.value)}
      />
      <Input
        aria-label="Client Secret"
        type="password"
        autoComplete="off"
        placeholder={sameApp ? "Client Secret (saved; blank keeps it)" : "Client Secret"}
        value={secret}
        onChange={(e) => setSecret(e.target.value)}
      />
      {save.error && <p className="text-destructive text-sm">{save.error.message}</p>}
      <div className="flex gap-2">
        <Button type="submit" disabled={!ready || save.isPending}>
          Save
        </Button>
        {onCancel && (
          <Button type="button" variant="outline" onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}

function Steps() {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-muted-foreground px-1 text-xs font-medium tracking-wide uppercase">
        Set up your own Strava app
      </h3>
      <ol className="text-muted-foreground flex list-decimal flex-col gap-1.5 pl-5 text-sm">
        <li>
          On strava.com/settings/api, create an app. Any name and website will do. A new
          Strava app only works for the person who made it, so everyone makes their own.
        </li>
        <li>
          Set its Authorization Callback Domain to this Niles:
          <CopyRow label="Callback domain" value={window.location.hostname} />
        </li>
        <li>Copy its Client ID and Client Secret into the fields below, and save.</li>
        <li>
          Connect Strava, and leave "View data about your private activities" ticked.
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
    <div className="mt-1 flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <code
          aria-label={label}
          className="bg-muted text-foreground min-w-0 flex-1 rounded-md px-3 py-2 font-mono text-xs break-all select-all"
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
  what: Confirming;
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
          <DialogTitle>
            {what === "remove" ? "Remove your Strava app?" : "Disconnect Strava?"}
          </DialogTitle>
          <DialogDescription>
            {what === "remove"
              ? "Niles forgets its Client ID and Secret, and stops reading. The workouts already here stay."
              : "Niles stops reading and Strava takes the access back. The workouts already here stay, and so does your app, to connect again with."}
          </DialogDescription>
        </div>
        <DialogBody className="flex flex-col gap-3">
          {error && <p className="text-destructive text-sm">{error}</p>}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>
              Keep it
            </Button>
            <Button variant="destructive" disabled={busy} onClick={onConfirm}>
              {what === "remove" ? "Remove" : "Disconnect"}
            </Button>
          </div>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
