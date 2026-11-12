import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import { LoginForm } from "@/components/LoginForm";
import { LOGINS } from "@/components/LoginsPanel";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import type { LoginRequest } from "@/lib/api";

/**
 * A turn's ask for a login the person has not saved: the whole site,
 * fixed, so the login can never be pointed anywhere else. The form
 * posts to /me/logins, and only the new login's id reaches the chat.
 */
export function LoginRequestCard({
  request,
  onAdded,
  adding,
}: {
  request: LoginRequest;
  /** Called with the saved login's id; never its values. */
  onAdded?: (login: number) => void;
  /** The conversation is carrying on after an add. */
  adding?: boolean;
}) {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const waiting = request.status === "waiting";
  return (
    <section
      aria-label={`${request.label} wants a login`}
      data-login-request={request.id}
      className="bg-muted/60 flex w-full max-w-xl flex-col gap-3 self-start rounded-2xl p-4 ring-1 ring-foreground/10"
    >
      <div className="flex items-start gap-3">
        <KeyRound aria-hidden className="mt-0.5 size-5 shrink-0" />
        <div className="min-w-0">
          <h3 className="text-base leading-snug font-medium">{request.label} wants a login</h3>
          <p className="text-muted-foreground font-mono text-sm break-all">{request.site}</p>
        </div>
      </div>
      {waiting ? (
        <Button
          className="rounded-full px-5 self-start"
          disabled={adding}
          onClick={() => setOpen(true)}
        >
          Add login
        </Button>
      ) : (
        <p role="status" className="text-muted-foreground text-sm">
          Login added
        </p>
      )}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="gap-0 p-0">
          <div className="border-border border-b px-4 py-3">
            <DialogTitle>Add a login for {request.label}</DialogTitle>
            <DialogDescription>It goes to Niles alone, never into the chat.</DialogDescription>
          </div>
          <DialogBody className="flex flex-col gap-3">
            <LoginForm
              asked={{ label: request.label, site: request.site }}
              onSaved={(login) => {
                setOpen(false);
                void queryClient.invalidateQueries({ queryKey: LOGINS });
                onAdded?.(login.id);
              }}
              onCancel={() => setOpen(false)}
            />
          </DialogBody>
        </DialogContent>
      </Dialog>
    </section>
  );
}
