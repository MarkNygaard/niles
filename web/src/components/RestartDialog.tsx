import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";

/** Asks before Niles goes away for a while. Restart uses it; NIL-13's
    Update will too. */
export function RestartDialog({
  open,
  onOpenChange,
  title = "Restart Niles?",
  action = "Restart",
  notes,
  busy,
  error,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title?: string;
  action?: string;
  notes: string[];
  busy: boolean;
  error?: string;
  onConfirm: () => void;
}) {
  return (
    <Dialog
      open={open}
      onOpenChange={(next: boolean) => {
        if (!next) onOpenChange(false);
      }}
    >
      <DialogContent className="gap-0 p-0">
        <div className="border-border border-b px-4 py-3">
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            Niles will be unavailable for about half a minute.
          </DialogDescription>
        </div>
        <DialogBody className="flex flex-col gap-3">
          {notes.length > 0 && (
            <ul className="text-muted-foreground flex list-disc flex-col gap-1 pl-5 text-sm">
              {notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          )}
          {error && <p className="text-destructive text-sm">{error}</p>}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button variant="destructive" disabled={busy} onClick={onConfirm}>
              {action}
            </Button>
          </div>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
