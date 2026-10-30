import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { ApiError, api, type RestartStatus } from "@/lib/api";
import { canRestart, restartNotes, startWaiting } from "@/lib/restart";
import { RestartDialog } from "@/components/RestartDialog";
import { Button, buttonVariants } from "@/components/ui/button";
import type { VariantProps } from "class-variance-authority";

/** The way to ask for a restart, beside the save note and on the bar.
    Only offered where something starts Niles again and the overrides
    would survive. */
export function RestartButton({
  status,
  size = "sm",
}: {
  status: RestartStatus;
  size?: VariantProps<typeof buttonVariants>["size"];
}) {
  const [open, setOpen] = useState(false);
  const restart = useMutation({
    mutationFn: () => api.restart(),
    onSuccess: ({ boot }) => {
      setOpen(false);
      startWaiting({ before: boot.id, supervisor: status.supervisor });
    },
  });

  if (!canRestart(status)) return null;
  const error = restart.error;
  return (
    <>
      <Button size={size} onClick={() => setOpen(true)}>
        Restart now
      </Button>
      <RestartDialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (!next) restart.reset();
        }}
        notes={restartNotes(status)}
        busy={restart.isPending}
        error={error instanceof ApiError ? error.message : error ? String(error) : undefined}
        onConfirm={() => restart.mutate()}
      />
    </>
  );
}
