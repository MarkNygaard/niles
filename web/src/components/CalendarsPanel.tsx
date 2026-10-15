import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BrandMark } from "@/components/BrandMark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api";
import type {
  CalendarAccess,
  CalendarConnection,
  CalendarProvider,
  CalendarsView,
} from "@/lib/api";
import { cn } from "@/lib/utils";

export const CALENDARS = ["calendars"];

const LABEL: Record<CalendarProvider, string> = {
  microsoft: "Microsoft",
  google: "Google",
  icloud: "iCloud",
};

/** The link that starts a provider's sign-in: followed, not fetched,
    because it ends at Microsoft or Google. */
export function connectUrl(
  provider: "microsoft" | "google",
  name: string,
  access: CalendarAccess,
  replace?: string,
): string {
  const query = new URLSearchParams({ name: name.trim(), access });
  if (replace) query.set("replace", replace);
  return `/calendars/${provider}/start?${query}`;
}

/** What the sign-in came back saying, from the address it returned to. */
export function outcomeOf(search: string): { ok: boolean; text: string } | null {
  const query = new URLSearchParams(search);
  const error = query.get("calendar_error");
  if (error) return { ok: false, text: error };
  if (query.get("calendar") === "connected") {
    const name = query.get("name");
    return { ok: true, text: name ? `${name} is connected.` : "Connected." };
  }
  return null;
}

/** The providers to offer, the available ones first. */
export function offered(view?: CalendarsView): CalendarsView["providers"] {
  return [...(view?.providers ?? [])].sort(
    (a, b) => Number(b.available) - Number(a.available),
  );
}

const ACCESS: { value: CalendarAccess; label: string; hint: string }[] = [
  { value: "read", label: "Read only", hint: "Niles can tell you what's on it." },
  { value: "write", label: "Read and write", hint: "Niles can also add, move and delete events." },
];

/**
 * The calendars Niles reads for you, and only for you: by your voice,
 * or here, signed in. Microsoft and Google are each a sign-in of their
 * own, granting what Niles may do; iCloud takes an app-specific
 * password, and there read only is Niles's own rule.
 */
export function CalendarsPanel() {
  const queryClient = useQueryClient();
  const calendars = useQuery({ queryKey: CALENDARS, queryFn: api.calendars, retry: false });
  const settle = () => queryClient.invalidateQueries({ queryKey: CALENDARS });
  const remove = useMutation({ mutationFn: api.removeCalendar, onSettled: settle });
  const setAccess = useMutation({
    mutationFn: ({ id, access }: { id: string; access: CalendarAccess }) =>
      api.setCalendarAccess(id, access),
    onSettled: settle,
  });
  const [outcome, setOutcome] = useState(() => outcomeOf(window.location.search));

  // Said once, then taken off the address so a refresh does not say it
  // again.
  useEffect(() => {
    if (!outcome) return;
    const clean = window.location.pathname + window.location.hash;
    window.history.replaceState(null, "", clean);
  }, [outcome]);

  const go = (url: string) => {
    setOutcome(null);
    window.location.assign(url);
  };

  return (
    <div className="flex flex-col gap-4">
        {outcome && (
          <p
            role="status"
            className={cn("text-sm", outcome.ok ? "text-foreground" : "text-destructive")}
          >
            {outcome.text}
          </p>
        )}
        {calendars.isError && (
          <p className="text-muted-foreground text-sm">{calendars.error.message}</p>
        )}
        {(setAccess.error ?? remove.error) && (
          <p className="text-destructive text-sm">{(setAccess.error ?? remove.error)?.message}</p>
        )}

        {calendars.data?.connections.map((c) => (
          <ConnectionRow
            key={c.id}
            connection={c}
            busy={remove.isPending || setAccess.isPending}
            onChangeAccess={() => {
              const access = c.access === "read" ? "write" : "read";
              if (c.provider === "icloud") setAccess.mutate({ id: c.id, access });
              else go(connectUrl(c.provider, c.name, access, c.id));
            }}
            onRemove={() => remove.mutate(c.id)}
          />
        ))}

        {calendars.data && (
          <ConnectForm
            view={calendars.data}
            onSignIn={(provider, name, access) => go(connectUrl(provider, name, access))}
            onConnected={() => {
              setOutcome(null);
              settle();
            }}
          />
        )}
    </div>
  );
}

function ConnectForm({
  view,
  onSignIn,
  onConnected,
}: {
  view: CalendarsView;
  onSignIn: (provider: "microsoft" | "google", name: string, access: CalendarAccess) => void;
  onConnected: () => void;
}) {
  const providers = offered(view);
  const [provider, setProvider] = useState<CalendarProvider>(
    providers.find((p) => p.available)?.id ?? "icloud",
  );
  const [name, setName] = useState("");
  const [access, setAccess] = useState<CalendarAccess>("read");
  const [appleId, setAppleId] = useState("");
  const [password, setPassword] = useState("");
  const icloud = useMutation({
    mutationFn: api.connectIcloud,
    onSuccess: () => {
      setName("");
      setAppleId("");
      setPassword("");
      onConnected();
    },
  });
  const chosen = providers.find((p) => p.id === provider);
  const ready =
    Boolean(chosen?.available) &&
    name.trim() !== "" &&
    (provider !== "icloud" || (appleId.trim() !== "" && password.trim() !== ""));

  return (
    <form
      className="flex flex-col gap-3 border-t pt-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (!ready) return;
        if (provider === "icloud") {
          icloud.mutate({ name, access, apple_id: appleId, password });
        } else {
          onSignIn(provider, name, access);
        }
      }}
    >
      <span className="text-sm font-medium">Connect a calendar</span>
      <div role="radiogroup" aria-label="Where the calendar is" className="flex flex-wrap gap-2">
        {providers.map((p) => (
          <button
            key={p.id}
            type="button"
            role="radio"
            aria-checked={provider === p.id}
            disabled={!p.available}
            title={p.reason ?? undefined}
            onClick={() => setProvider(p.id)}
            className={cn(
              "flex items-center gap-2 rounded-lg border px-3 py-2 text-sm transition-colors",
              "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
              "disabled:cursor-not-allowed disabled:opacity-50",
              provider === p.id ? "border-foreground bg-muted" : "hover:bg-muted/50",
            )}
          >
            <BrandMark id={p.id} label={LABEL[p.id]} className="size-4" />
            {LABEL[p.id]}
          </button>
        ))}
      </div>
      {chosen && !chosen.available && chosen.reason && (
        <p className="text-muted-foreground text-xs">{chosen.reason}</p>
      )}

      <Input
        aria-label="Calendar name"
        placeholder="Private, Work…"
        maxLength={40}
        value={name}
        onChange={(e) => setName(e.target.value)}
      />

      {provider === "icloud" && (
        <>
          <Input
            aria-label="Apple ID"
            type="email"
            autoComplete="username"
            placeholder="Apple ID (your iCloud email)"
            value={appleId}
            onChange={(e) => setAppleId(e.target.value)}
          />
          <Input
            aria-label="App-specific password"
            type="password"
            autoComplete="off"
            placeholder="App-specific password, xxxx-xxxx-xxxx-xxxx"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
          <p className="text-muted-foreground text-xs">
            Make one at appleid.apple.com → Sign-In and Security → App-Specific Passwords. Not
            your Apple ID's own password.
          </p>
        </>
      )}

      <div role="radiogroup" aria-label="What Niles may do" className="flex flex-col gap-2 sm:flex-row">
        {ACCESS.map((option) => (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={access === option.value}
            onClick={() => setAccess(option.value)}
            className={cn(
              "flex-1 rounded-lg border px-3 py-2 text-left transition-colors",
              "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
              access === option.value ? "border-foreground bg-muted" : "hover:bg-muted/50",
            )}
          >
            <span className="block text-sm font-medium">{option.label}</span>
            <span className="text-muted-foreground block text-xs">{option.hint}</span>
          </button>
        ))}
      </div>
      {provider === "icloud" && (
        <p className="text-muted-foreground text-xs">
          Apple has no read-only password: for iCloud, read only is a rule Niles keeps, not a lock
          Apple holds. You can revoke the password at appleid.apple.com at any time.
        </p>
      )}

      {icloud.error && <p className="text-destructive text-sm">{icloud.error.message}</p>}
      <Button type="submit" className="self-start" disabled={!ready || icloud.isPending}>
        {provider === "icloud" ? "Connect iCloud" : `Connect with ${LABEL[provider]}`}
      </Button>
      {provider === "microsoft" && (
        <p className="text-muted-foreground text-xs">
          A work account may need your IT department's approval before it can be connected.
        </p>
      )}
    </form>
  );
}

function ConnectionRow({
  connection,
  busy,
  onChangeAccess,
  onRemove,
}: {
  connection: CalendarConnection;
  busy: boolean;
  onChangeAccess: () => void;
  onRemove: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="flex flex-col gap-2 rounded-lg border px-3 py-2 sm:flex-row sm:items-center">
      <BrandMark
        id={connection.provider}
        label={LABEL[connection.provider]}
        className="hidden size-5 sm:block"
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{connection.name}</span>
          <Badge variant="outline">
            {connection.access === "write" ? "Read and write" : "Read only"}
          </Badge>
        </div>
        <div className="text-muted-foreground truncate text-xs">
          {LABEL[connection.provider]} · {connection.account}
          {!connection.enforced_by_provider &&
            connection.access === "read" &&
            " · read only is kept by Niles"}
        </div>
      </div>
      <div className="flex gap-2">
        <Button variant="outline" size="sm" disabled={busy} onClick={onChangeAccess}>
          {connection.access === "write" ? "Make read only" : "Allow writing"}
        </Button>
        {confirming ? (
          <Button variant="destructive" size="sm" disabled={busy} onClick={onRemove}>
            Disconnect {connection.name}
          </Button>
        ) : (
          <Button variant="ghost" size="sm" disabled={busy} onClick={() => setConfirming(true)}>
            Disconnect
          </Button>
        )}
      </div>
    </div>
  );
}
