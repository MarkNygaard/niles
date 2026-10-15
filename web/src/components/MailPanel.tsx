import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BrandMark } from "@/components/BrandMark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ACCOUNTS, change, connectUrl, outcomeOf, ticksOf } from "@/lib/accounts";
import type { MailLevel, Ticks } from "@/lib/accounts";
import { api } from "@/lib/api";
import type { CalendarConnection, CalendarProvider, CalendarsView } from "@/lib/api";
import { cn } from "@/lib/utils";

const LABEL: Record<CalendarProvider, string> = {
  microsoft: "Microsoft",
  google: "Google",
  icloud: "iCloud",
};

const ACCESS: { value: Exclude<MailLevel, "none">; label: string; hint: string }[] = [
  { value: "read", label: "Read only", hint: "Niles can tell you what's in your mail." },
  {
    value: "send",
    label: "Read and send",
    hint: "Niles can also write mail, which you check and send in the app.",
  },
];

/** The providers to offer for mail, the available ones first. */
export function mailProviders(view?: CalendarsView): CalendarsView["providers"] {
  return [...(view?.providers ?? [])].sort((a, b) => Number(b.available) - Number(a.available));
}

/**
 * Your mail: read by Niles only when it hears you, or when you ask here.
 * Reading is the default. An account that may send still sends nothing
 * on its own: every mail waits for you in the chat, to check and allow.
 */
export function MailPanel() {
  const queryClient = useQueryClient();
  const accounts = useQuery({ queryKey: ACCOUNTS, queryFn: api.calendars, retry: false });
  const settle = () => queryClient.invalidateQueries({ queryKey: ACCOUNTS });
  const remove = useMutation({ mutationFn: api.removeCalendar, onSettled: settle });
  const setTicks = useMutation({
    mutationFn: ({ id, ticks }: { id: string; ticks: Ticks }) =>
      api.setAccountServices(id, ticks),
    onSettled: settle,
  });
  const [outcome, setOutcome] = useState(() => outcomeOf(window.location.search, "mail"));

  useEffect(() => {
    if (!outcome) return;
    window.history.replaceState(null, "", window.location.pathname + window.location.hash);
  }, [outcome]);

  const go = (url: string) => {
    setOutcome(null);
    window.location.assign(url);
  };
  const retick = (account: CalendarConnection, ticks: Ticks) => {
    const how = change(account, ticks, "mail");
    if (how.kind === "remove") remove.mutate(account.id);
    else if (how.kind === "in_place") setTicks.mutate({ id: account.id, ticks });
    else go(how.url);
  };

  const all = accounts.data?.connections ?? [];
  const serving = all.filter((c) => c.mail);
  const others = all.filter((c) => !c.mail);
  const busy = remove.isPending || setTicks.isPending;

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
      {accounts.isError && (
        <p className="text-muted-foreground text-sm">{accounts.error.message}</p>
      )}
      {(setTicks.error ?? remove.error) && (
        <p className="text-destructive text-sm">{(setTicks.error ?? remove.error)?.message}</p>
      )}

      {serving.map((c) => (
        <MailRow
          key={c.id}
          account={c}
          busy={busy}
          onChangeAccess={() =>
            retick(c, { ...ticksOf(c), mail: c.mail === "send" ? "read" : "send" })
          }
          // The account stays for its calendar, if it serves that too.
          onRemove={() => retick(c, { ...ticksOf(c), mail: "none" })}
        />
      ))}

      {accounts.data && (
        <ConnectMail
          view={accounts.data}
          others={others}
          busy={busy}
          onUse={(account, mail) => retick(account, { ...ticksOf(account), mail })}
          onSignIn={(provider, name, mail) =>
            go(connectUrl(provider, name, { calendar: "none", mail }, undefined, "mail"))
          }
          onConnected={() => {
            setOutcome(null);
            settle();
          }}
        />
      )}
    </div>
  );
}

function ConnectMail({
  view,
  others,
  busy,
  onUse,
  onSignIn,
  onConnected,
}: {
  view: CalendarsView;
  /** Accounts already connected that do not serve Mail yet. */
  others: CalendarConnection[];
  busy: boolean;
  onUse: (account: CalendarConnection, mail: Exclude<MailLevel, "none">) => void;
  onSignIn: (
    provider: "microsoft" | "google",
    name: string,
    mail: Exclude<MailLevel, "none">,
  ) => void;
  onConnected: () => void;
}) {
  const providers = mailProviders(view);
  const [provider, setProvider] = useState<CalendarProvider>(
    providers.find((p) => p.available)?.id ?? "icloud",
  );
  const [name, setName] = useState("");
  const [access, setAccess] = useState<Exclude<MailLevel, "none">>("read");
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
          icloud.mutate({ name, calendar: "none", mail: access, apple_id: appleId, password });
        } else {
          onSignIn(provider, name, access);
        }
      }}
    >
      <span className="text-sm font-medium">Connect mail</span>

      <div role="radiogroup" aria-label="What Niles may do with mail" className="flex flex-col gap-2 sm:flex-row">
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

      {others.length > 0 && (
        <div className="flex flex-col gap-2">
          {others.map((account) => (
            <Button
              key={account.id}
              type="button"
              variant="outline"
              className="justify-start"
              disabled={busy}
              onClick={() => onUse(account, access)}
            >
              <BrandMark id={account.provider} label={LABEL[account.provider]} className="size-4" />
              Use {account.account}, already connected for your calendar
            </Button>
          ))}
          <span className="text-muted-foreground text-xs">
            {others.some((a) => a.provider !== "icloud")
              ? "Microsoft and Google ask once more, to let Niles read the mail too. Or connect another:"
              : "Or connect another:"}
          </span>
        </div>
      )}

      <div role="radiogroup" aria-label="Where the mail is" className="flex flex-wrap gap-2">
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
        aria-label="Account name"
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
            Make one at appleid.apple.com → Sign-In and Security → App-Specific Passwords. Apple
            has no read-only password: for iCloud, read only is a rule Niles keeps.
          </p>
        </>
      )}

      {icloud.error && <p className="text-destructive text-sm">{icloud.error.message}</p>}
      <Button type="submit" className="self-start" disabled={!ready || icloud.isPending}>
        {provider === "icloud" ? "Connect iCloud" : `Connect with ${LABEL[provider]}`}
      </Button>
      {provider === "google" && (
        <p className="text-muted-foreground text-xs">
          Google warns that it hasn't verified the app when it is asked for Gmail. That is expected
          for your household's own app.
        </p>
      )}
    </form>
  );
}

function MailRow({
  account,
  busy,
  onChangeAccess,
  onRemove,
}: {
  account: CalendarConnection;
  busy: boolean;
  onChangeAccess: () => void;
  onRemove: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="flex flex-col gap-2 rounded-lg border px-3 py-2 sm:flex-row sm:items-center">
      <BrandMark
        id={account.provider}
        label={LABEL[account.provider]}
        className="hidden size-5 sm:block"
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{account.name}</span>
          <Badge variant="outline">{account.mail === "send" ? "Read and send" : "Read only"}</Badge>
        </div>
        <div className="text-muted-foreground truncate text-xs">
          {LABEL[account.provider]} · {account.account}
        </div>
      </div>
      <div className="flex gap-2">
        <Button variant="outline" size="sm" disabled={busy} onClick={onChangeAccess}>
          {account.mail === "send" ? "Read only" : "Allow sending"}
        </Button>
        {confirming ? (
          <Button variant="destructive" size="sm" disabled={busy} onClick={onRemove}>
            Disconnect {account.name}
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
