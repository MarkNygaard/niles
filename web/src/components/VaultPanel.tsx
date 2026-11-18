import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronLeft,
  Copy,
  KeyRound,
  Lock,
  LogIn,
  LogOut,
  Pencil,
  Plus,
  Search,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { LoginForm } from "@/components/LoginForm";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { api, loginIconUrl } from "@/lib/api";
import type { LoginActivity, SavedLogin } from "@/lib/api";
import { relativeTime } from "@/lib/skills";
import { cn } from "@/lib/utils";

export const LOGINS = ["me-logins"];

/** What each kind of use reads as in a login's activity. */
export const USED: Record<LoginActivity["used"], string> = {
  added: "Added",
  edited: "Edited",
  signed_out: "Signed out",
  profile_dropped: "Kept cookies forgotten",
  signed_in: "Signed in",
  sign_in_failed: "Sign-in refused: the page still asked for the login",
  sign_in_uncertain: "Signed in? Niles couldn't see the result",
  profile_restored: "Signed in with the cookies kept from last time",
  code_entered: "Code you typed entered",
  code_made: "Code from the authenticator key entered",
};

/** The site of an origin as a person reads it: its host. */
function hostOf(origin: string): string {
  try {
    return new URL(origin.includes("://") ? origin : `https://${origin}`).host;
  } catch {
    return origin;
  }
}

/** The logins whose label, username or sites hold what was searched. */
export function matching(logins: SavedLogin[], search: string): SavedLogin[] {
  const wanted = search.trim().toLowerCase();
  if (!wanted) return logins;
  return logins.filter((l) =>
    [l.label, l.username, ...l.origins].some((s) => s.toLowerCase().includes(wanted)),
  );
}

/** A login's uses grouped by the day they fell on, newest first: Today,
    Yesterday, then the date. */
export function byDay(
  uses: LoginActivity[],
  now: Date = new Date(),
): { day: string; uses: LoginActivity[] }[] {
  const start = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const today = start(now);
  const groups: { day: string; uses: LoginActivity[] }[] = [];
  for (const use of uses) {
    const at = new Date(use.at);
    const days = Math.round((today - start(at)) / 86_400_000);
    const day =
      days === 0
        ? "Today"
        : days === 1
          ? "Yesterday"
          : at.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
    const last = groups[groups.length - 1];
    if (last && last.day === day) last.uses.push(use);
    else groups.push({ day, uses: [use] });
  }
  return groups;
}

/**
 * Vault: the signed-in person's saved logins, the sites Niles signs in
 * to for them, and what each was used for. A list beside the chosen
 * login's detail on a desk; on a phone the detail pushes in. Nothing of
 * the password or the authenticator key, which are write-only from the
 * form.
 */
export function VaultPanel() {
  const queryClient = useQueryClient();
  const list = useQuery({ queryKey: LOGINS, queryFn: api.logins, retry: false });
  const settle = () => queryClient.invalidateQueries({ queryKey: LOGINS });
  const wide = useMediaQuery("(min-width: 1024px)");
  const [chosen, setChosen] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [editing, setEditing] = useState<SavedLogin | "new" | null>(null);
  const [removing, setRemoving] = useState<SavedLogin | null>(null);

  const remove = useMutation({
    mutationFn: api.removeLogin,
    onSuccess: () => {
      setRemoving(null);
      setChosen(null);
    },
    onSettled: settle,
  });
  const signOut = useMutation({ mutationFn: api.signOutLogin, onSettled: settle });

  if (list.isPending) {
    return <Skeleton className="h-10 w-full" />;
  }
  if (list.isError) {
    // 501 carries the server's sentence: no database, or no secrets key.
    return <p className="text-destructive text-sm">{list.error.message}</p>;
  }

  const logins = list.data.logins;
  const shown = matching(logins, search);
  // On a desk the first login is open until another is chosen.
  const open =
    logins.find((l) => l.id === chosen) ?? (wide ? (shown[0] ?? null) : null);

  const listing = (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <Search
            aria-hidden
            className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2"
          />
          <Input
            aria-label="Search logins"
            placeholder="Search"
            className="pl-9"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <Button aria-label="Add login" onClick={() => setEditing("new")}>
          <Plus aria-hidden />
          Add
        </Button>
      </div>
      {logins.length === 0 ? (
        <p className="text-muted-foreground text-sm">
          No saved logins yet. Add one here, or when Niles asks for one in the chat.
        </p>
      ) : shown.length === 0 ? (
        <p className="text-muted-foreground text-sm">No login matches that.</p>
      ) : (
        <ul aria-label="Your logins" className="flex flex-col gap-1">
          {shown.map((login) => (
            <li key={login.id}>
              <button
                type="button"
                aria-current={open?.id === login.id ? "true" : undefined}
                onClick={() => setChosen(login.id)}
                className={cn(
                  "hover:bg-muted flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition-colors",
                  open?.id === login.id && "bg-muted",
                )}
              >
                <SiteIcon login={login} />
                <span className="flex min-w-0 flex-col">
                  <span className="truncate font-medium">{login.label}</span>
                  <span className="text-muted-foreground truncate text-sm">{login.username}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <p className="text-muted-foreground flex items-center gap-1.5 text-xs">
        <Lock aria-hidden className="size-3.5" />
        Encrypted in Niles · only you can use these
      </p>
    </div>
  );

  const detail = open && (
    <Detail
      login={open}
      onBack={wide ? undefined : () => setChosen(null)}
      onEdit={() => setEditing(open)}
      onRemove={() => setRemoving(open)}
      onSignOut={() => signOut.mutate(open.id)}
      signingOut={signOut.isPending}
    />
  );

  return (
    <>
      {wide ? (
        <div className="grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-6">
          {listing}
          <div className="border-border min-w-0 border-l pl-6">{detail}</div>
        </div>
      ) : (
        (detail ?? listing)
      )}
      <Dialog
        open={editing !== null}
        onOpenChange={(next: boolean) => {
          if (!next) setEditing(null);
        }}
      >
        <DialogContent className="gap-0 p-0">
          <div className="border-border border-b px-4 py-3">
            <DialogTitle>
              {editing === null || editing === "new" ? "Add a login" : `Edit ${editing.label}`}
            </DialogTitle>
          </div>
          <DialogBody className="flex flex-col gap-3">
            <LoginForm
              login={editing === null || editing === "new" ? undefined : editing}
              onSaved={(saved) => {
                setEditing(null);
                setChosen(saved.id);
                void settle();
              }}
              onCancel={() => setEditing(null)}
            />
          </DialogBody>
        </DialogContent>
      </Dialog>
      <Dialog
        open={removing !== null}
        onOpenChange={(next: boolean) => {
          if (!next) {
            remove.reset();
            setRemoving(null);
          }
        }}
      >
        <DialogContent className="gap-0 p-0">
          <div className="border-border border-b px-4 py-3">
            <DialogTitle>Remove {removing?.label}?</DialogTitle>
            <DialogDescription>
              Niles forgets its username, password and authenticator key, the site's cookies,
              and what it was used for.
            </DialogDescription>
          </div>
          <DialogBody className="flex flex-col gap-3">
            {remove.error && <p className="text-destructive text-sm">{remove.error.message}</p>}
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                onClick={() => {
                  remove.reset();
                  setRemoving(null);
                }}
              >
                Keep it
              </Button>
              <Button
                variant="destructive"
                disabled={remove.isPending}
                onClick={() => removing && remove.mutate(removing.id)}
              >
                Remove
              </Button>
            </div>
          </DialogBody>
        </DialogContent>
      </Dialog>
    </>
  );
}

/** A colour for a login's initial, the same each time for its label. */
function hueOf(label: string): number {
  let hash = 0;
  for (const c of label) hash = (hash * 31 + c.charCodeAt(0)) % 360;
  return hash;
}

/** The site's icon, fetched by Niles; its initial while it loads or
    when the site has none. */
function SiteIcon({ login, large }: { login: SavedLogin; large?: boolean }) {
  const [failed, setFailed] = useState(false);
  const size = large ? "size-12" : "size-9";
  const initial = (login.label.trim()[0] ?? "?").toUpperCase();
  return (
    <span
      aria-hidden
      className={cn(
        "relative flex shrink-0 items-center justify-center overflow-hidden rounded-xl font-medium text-white",
        size,
        large && "text-lg",
      )}
      style={{ backgroundColor: `hsl(${hueOf(login.label)} 45% 45%)` }}
    >
      {initial}
      {!failed && (
        <img
          src={loginIconUrl(login.id)}
          alt=""
          className="bg-background absolute inset-0 size-full object-contain p-1"
          onError={() => setFailed(true)}
        />
      )}
    </span>
  );
}

/** One field of a login, on its own row. */
function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="border-border flex flex-col gap-1 border-b py-3">
      <span className="text-muted-foreground text-sm">{label}</span>
      <div className="flex min-h-8 items-center gap-2">{children}</div>
    </div>
  );
}

function Saved({ children }: { children: React.ReactNode }) {
  return <Badge variant="secondary">{children}</Badge>;
}

/** A chosen login: its fields, its activity by day, and what can be done
    with it. The password shows as saved, never itself. */
function Detail({
  login,
  onBack,
  onEdit,
  onRemove,
  onSignOut,
  signingOut,
}: {
  login: SavedLogin;
  onBack?: () => void;
  onEdit: () => void;
  onRemove: () => void;
  onSignOut: () => void;
  signingOut: boolean;
}) {
  const [copied, setCopied] = useState(false);
  return (
    <section aria-label={login.label} className="flex min-w-0 flex-col">
      {onBack && (
        <Button variant="ghost" size="sm" className="mb-2 self-start" onClick={onBack}>
          <ChevronLeft aria-hidden />
          All logins
        </Button>
      )}
      <div className="flex items-start gap-3">
        <SiteIcon login={login} large />
        <div className="flex min-w-0 flex-1 flex-col">
          <h3 className="truncate text-lg font-medium">{login.label}</h3>
          <p className="text-muted-foreground text-sm break-all">
            {login.origins.map(hostOf).join(" · ")}
          </p>
        </div>
        <Button variant="secondary" size="sm" aria-label={`Edit ${login.label}`} onClick={onEdit}>
          <Pencil aria-hidden />
          Edit
        </Button>
      </div>
      <div className="mt-2 flex flex-col">
        <Field label="Username or email">
          <span className="min-w-0 flex-1 break-all">{login.username}</span>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Copy the username"
            onClick={() => {
              void navigator.clipboard?.writeText(login.username).then(() => setCopied(true));
            }}
          >
            <Copy aria-hidden />
          </Button>
          {copied && <span className="text-muted-foreground text-xs">Copied</span>}
        </Field>
        <Field label="Password">
          <Saved>Saved</Saved>
          <span className="text-muted-foreground text-xs">Only Niles's browser uses it</span>
        </Field>
        <Field label="Authenticator">
          {login.authenticator ? (
            <Saved>Saved</Saved>
          ) : (
            <span className="text-muted-foreground text-sm">Not set</span>
          )}
        </Field>
        <Field label="Browser">
          {login.signed_in ? (
            <>
              <Saved>Signed in</Saved>
              <Button
                variant="outline"
                size="sm"
                className="ml-auto"
                aria-label={`Sign out of ${login.label}`}
                disabled={signingOut}
                onClick={onSignOut}
              >
                <LogOut aria-hidden />
                Sign out
              </Button>
            </>
          ) : (
            <span className="text-muted-foreground text-sm">
              Not signed in
              {login.last_used && <> · Last used {relativeTime(login.last_used)}</>}
            </span>
          )}
        </Field>
      </div>
      <Activity login={login} />
      <div className="pt-4">
        <Button
          variant="link"
          className="text-destructive h-auto px-0"
          aria-label={`Remove ${login.label}`}
          onClick={onRemove}
        >
          <Trash2 aria-hidden />
          Delete login
        </Button>
      </div>
    </section>
  );
}

/** What a use reads as, with the icon that goes with it. */
function useIcon(used: LoginActivity["used"]) {
  if (used === "code_made" || used === "code_entered") return ShieldCheck;
  if (used === "signed_in" || used === "profile_restored") return LogIn;
  if (used === "signed_out" || used === "profile_dropped") return LogOut;
  return KeyRound;
}

/** One login's uses, by day, newest first: what happened on which site,
    when, and the conversation it was for. */
function Activity({ login }: { login: SavedLogin }) {
  const uses = useQuery({
    queryKey: [...LOGINS, login.id, "activity"],
    queryFn: () => api.loginActivity(login.id),
    retry: false,
  });
  if (uses.isPending) {
    return <Skeleton className="mt-4 h-10 w-full" />;
  }
  if (uses.isError) {
    return <p className="text-destructive mt-4 text-sm">{uses.error.message}</p>;
  }
  if (uses.data.activity.length === 0) {
    return <p className="text-muted-foreground mt-4 text-sm">Nothing in the last 90 days.</p>;
  }
  return (
    <div
      role="list"
      aria-label={`Activity of ${login.label}`}
      className="mt-4 flex flex-col gap-4"
    >
      {byDay(uses.data.activity).map((group) => (
        <div key={group.day} role="listitem" className="flex flex-col gap-2">
          <h4 className="text-muted-foreground text-sm font-medium">{group.day}</h4>
          <ul className="flex flex-col gap-3">
            {group.uses.map((use, i) => {
              const Icon = useIcon(use.used);
              const time = new Date(use.at).toLocaleTimeString(undefined, {
                hour: "2-digit",
                minute: "2-digit",
              });
              return (
                <li key={`${use.at}-${i}`} className="flex items-start gap-3">
                  <span className="bg-muted flex size-8 shrink-0 items-center justify-center rounded-full">
                    <Icon aria-hidden className="size-4" />
                  </span>
                  <span className="flex min-w-0 flex-col">
                    <span className="text-sm font-medium break-words">
                      {USED[use.used] ?? use.used}
                      {use.site && <> · {hostOf(use.site)}</>}
                    </span>
                    <span className="text-muted-foreground text-xs break-words">
                      {time}
                      {use.conversation !== null && (
                        <>
                          {" · "}
                          <a className="underline" href={`#/chat?conversation=${use.conversation}`}>
                            {use.conversation_title || "the conversation"}
                          </a>
                        </>
                      )}
                    </span>
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </div>
  );
}
