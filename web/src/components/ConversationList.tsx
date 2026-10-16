import { useState } from "react";
import { Mic, SquarePen, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type { ConversationSummary } from "@/lib/api";
import { relativeTime } from "@/lib/skills";
import { cn } from "@/lib/utils";

export interface ConversationListProps {
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
 * first: the Conversations tab of Niles's panel.
 */
export function ConversationList({
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

  // In place of the list rather than over it: a dialog on top of the
  // panel would be a third layer for a yes or no.
  if (confirming) {
    return (
      <div className="flex flex-col gap-3 px-2 py-1">
        <div>
          <h4 className="font-heading text-base leading-snug font-medium">
            Delete this conversation?
          </h4>
          <p className="text-muted-foreground text-sm break-words">{confirming.title}</p>
        </div>
        {deleteError && <p className="text-destructive text-sm">{deleteError}</p>}
        <div className="flex justify-end gap-2">
          <Button variant="outline" onClick={leaveConfirm}>
            Keep it
          </Button>
          <Button variant="destructive" disabled={deleting} onClick={() => remove(confirming.id)}>
            Delete
          </Button>
        </div>
      </div>
    );
  }

  const row = cn(
    "min-w-0 flex-1 rounded-lg px-2 py-2 text-left transition-colors",
    "hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
  );

  return (
    <div className="flex flex-col gap-1">
      <button type="button" onClick={onNew} className={cn(row, "flex items-center gap-2 text-sm")}>
        <SquarePen aria-hidden className="text-muted-foreground size-4" />
        New chat
      </button>
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
                className={cn(row, c.id === current && "bg-muted")}
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
                size="icon"
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
    </div>
  );
}
