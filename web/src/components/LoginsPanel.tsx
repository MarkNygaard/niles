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
import type { SavedLogin } from "@/lib/api";
import { relativeTime } from "@/lib/skills";

export const LOGINS = ["me-logins"];

/**
 * The signed-in person's saved logins: the sites Niles signs in to for
 * them, and nothing of the password, which is write-only from the form.
 */
export function LoginsPanel() {
  const queryClient = useQueryClient();
  const list = useQuery({ queryKey: LOGINS, queryFn: api.logins, retry: false });
  const settle = () => queryClient.invalidateQueries({ queryKey: LOGINS });
  const [editing, setEditing] = useState<SavedLogin | "new" | null>(null);
  const [removing, setRemoving] = useState<SavedLogin | null>(null);

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
              Niles forgets its username and password, and the site's cookies with them.
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
