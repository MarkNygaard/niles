import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { LoginActivity, SavedLogin } from "@/lib/api";
import { relativeTime } from "@/lib/skills";

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

/**
 * Vault: the signed-in person's saved logins, the sites Niles signs in
 * to for them, and what each was used for. Nothing of the password or
 * the authenticator key, which are write-only from the form.
 */
export function VaultPanel() {
  const queryClient = useQueryClient();
  const list = useQuery({ queryKey: LOGINS, queryFn: api.logins, retry: false });
  const settle = () => queryClient.invalidateQueries({ queryKey: LOGINS });
  const [editing, setEditing] = useState<SavedLogin | "new" | null>(null);
  const [removing, setRemoving] = useState<SavedLogin | null>(null);
  const [looking, setLooking] = useState<SavedLogin | null>(null);

  const remove = useMutation({
    mutationFn: api.removeLogin,
    onSuccess: () => setRemoving(null),
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
  return (
    <>
      <div className="flex flex-col gap-4">
        {logins.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            No saved logins yet. Add one here, or when Niles asks for one in the chat.
          </p>
        ) : (
          <ul aria-label="Your logins" className="flex flex-col gap-4">
            {logins.map((login) => (
              <li key={login.id} className="flex flex-col gap-1.5">
                <div className="flex items-center gap-2">
                  <span className="font-medium">{login.label}</span>
                  {login.signed_in && <Badge variant="secondary">Signed in</Badge>}
                  {login.authenticator && <Badge variant="outline">Authenticator</Badge>}
                </div>
                <p className="text-muted-foreground text-sm break-all">
                  {login.origins.join(" · ")}
                </p>
                <p className="text-muted-foreground text-sm">
                  {login.username}
                  {login.last_used && <> · Last used {relativeTime(login.last_used)}</>}
                </p>
                <div className="flex flex-wrap gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`Edit ${login.label}`}
                    onClick={() => setEditing(login)}
                  >
                    Edit
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`Activity of ${login.label}`}
                    onClick={() => setLooking(login)}
                  >
                    Activity
                  </Button>
                  {login.signed_in && (
                    <Button
                      variant="outline"
                      size="sm"
                      aria-label={`Sign out of ${login.label}`}
                      disabled={signOut.isPending}
                      onClick={() => signOut.mutate(login.id)}
                    >
                      Sign out
                    </Button>
                  )}
                  <Button
                    variant="outline"
                    size="sm"
                    aria-label={`Remove ${login.label}`}
                    onClick={() => setRemoving(login)}
                  >
                    Remove
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
        <div>
          <Button onClick={() => setEditing("new")}>Add login</Button>
        </div>
      </div>
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
              onSaved={() => {
                setEditing(null);
                void settle();
              }}
              onCancel={() => setEditing(null)}
            />
          </DialogBody>
        </DialogContent>
      </Dialog>
      <Dialog
        open={looking !== null}
        onOpenChange={(next: boolean) => {
          if (!next) setLooking(null);
        }}
      >
        <DialogContent className="gap-0 p-0">
          <div className="border-border border-b px-4 py-3">
            <DialogTitle>{looking?.label}: activity</DialogTitle>
            <DialogDescription>
              Every time Niles used this login in the last 90 days. Never what was typed.
            </DialogDescription>
          </div>
          <DialogBody className="flex flex-col gap-3">
            {looking && <Activity login={looking} />}
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

/** One login's uses, newest first, each with its site and a link to the
    conversation it was for. */
function Activity({ login }: { login: SavedLogin }) {
  const uses = useQuery({
    queryKey: [...LOGINS, login.id, "activity"],
    queryFn: () => api.loginActivity(login.id),
    retry: false,
  });
  if (uses.isPending) {
    return <Skeleton className="h-10 w-full" />;
  }
  if (uses.isError) {
    return <p className="text-destructive text-sm">{uses.error.message}</p>;
  }
  if (uses.data.activity.length === 0) {
    return <p className="text-muted-foreground text-sm">Nothing in the last 90 days.</p>;
  }
  return (
    <ul aria-label={`Activity of ${login.label}`} className="flex flex-col gap-2">
      {uses.data.activity.map((use, i) => (
        <li key={`${use.at}-${i}`} className="flex flex-col">
          <span className="text-sm">{USED[use.used] ?? use.used}</span>
          <span className="text-muted-foreground text-xs break-all">
            {relativeTime(use.at)}
            {use.site && <> · {use.site}</>}
            {use.conversation !== null && (
              <>
                {" · "}
                <a className="underline" href={`#/chat?conversation=${use.conversation}`}>
                  the conversation
                </a>
              </>
            )}
          </span>
        </li>
      ))}
    </ul>
  );
}
