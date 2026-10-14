import { useState } from "react";
import { Mic, SquarePen, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import type { ConversationSummary } from "@/lib/api";
import { relativeTime } from "@/lib/skills";
import { cn } from "@/lib/utils";

export interface ConversationListProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Undefined while they are on their way. */
  conversations?: ConversationSummary[];
  error?: string;
  /** The one on screen, if it is in the list. */
  current?: number | null;
  onPick: (id: number) => void;
  onNew: () => void;
  onDelete: (id: number) => Promise<unknown>;
}

/**
 * The person's past conversations with Niles, spoken and typed, newest
 * first. A sheet on a phone and a dialog on a desktop, as every
 * `DialogContent` is: beside the chat, it would have to share the width
 * the pinned composer takes for itself.
 */
export function ConversationList({
  open,
  onOpenChange,
  conversations,
  error,
  current,
  onPick,
  onNew,
  onDelete,
}: ConversationListProps) {
  const [confirming, setConfirming] = useState<ConversationSummary>();
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string>();

  const leaveConfirm = () => {
    setConfirming(undefined);
    setDeleteError(undefined);
  };

  const remove = async (id: number) => {
    setDeleting(true);
    setDeleteError(undefined);
    try {
      await onDelete(id);
      setConfirming(undefined);
    } catch (e) {
      setDeleteError(e instanceof Error ? e.message : String(e));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next: boolean) => {
        if (!next) leaveConfirm();
        onOpenChange(next);
      }}
    >
      <DialogContent className="gap-0 p-0">
        {confirming ? (
          <>
            <div className="border-border border-b px-4 py-3">
              <DialogTitle>Delete this conversation?</DialogTitle>
              <DialogDescription className="break-words">{confirming.title}</DialogDescription>
            </div>
            <DialogBody className="flex flex-col gap-3">
              {deleteError && <p className="text-destructive text-sm">{deleteError}</p>}
              <div className="flex justify-end gap-2">
                <Button variant="outline" onClick={leaveConfirm}>
                  Keep it
                </Button>
                <Button
                  variant="destructive"
                  disabled={deleting}
                  onClick={() => remove(confirming.id)}
                >
                  Delete
                </Button>
              </div>
            </DialogBody>
          </>
        ) : (
          <>
            <div className="border-border flex items-center justify-between gap-2 border-b px-4 py-3">
              <DialogTitle>Conversations</DialogTitle>
              <Button variant="ghost" size="sm" onClick={onNew}>
                <SquarePen />
                New chat
              </Button>
            </div>
            <DialogBody className="p-2">
              {error ? (
                <p className="text-destructive px-2 py-1 text-sm">{error}</p>
              ) : conversations === undefined ? (
                <div className="flex flex-col gap-2 p-2">
                  <Skeleton className="h-10 w-full" />
                  <Skeleton className="h-10 w-full" />
                </div>
              ) : conversations.length === 0 ? (
                <p className="text-muted-foreground px-2 py-1 text-sm">
                  Nothing yet. What you say to Niles, out loud or here, is kept here.
                </p>
              ) : (
                <ul aria-label="Conversations" className="flex flex-col">
                  {conversations.map((c) => (
                    <li key={c.id} className="flex items-center gap-1">
                      <button
                        type="button"
                        aria-current={c.id === current ? "true" : undefined}
                        onClick={() => onPick(c.id)}
                        className={cn(
                          "min-w-0 flex-1 rounded-lg px-2 py-1.5 text-left transition-colors",
                          "hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                          c.id === current && "bg-muted",
                        )}
                      >
                        <span className="block truncate text-sm">{c.title}</span>
                        <span className="text-muted-foreground flex items-center gap-1 text-xs">
                          {c.spoken && (
                            <span role="img" aria-label="Said to a satellite" className="inline-flex">
                              <Mic className="size-3" aria-hidden />
                            </span>
                          )}
                          {relativeTime(c.last_at)} · {c.turns === 1 ? "1 turn" : `${c.turns} turns`}
                        </span>
                      </button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label="Delete"
                        onClick={() => setConfirming(c)}
                        className="text-muted-foreground shrink-0"
                      >
                        <Trash2 />
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </DialogBody>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
