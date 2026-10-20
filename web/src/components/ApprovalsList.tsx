import { useId } from "react";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { MessageCircle } from "lucide-react";
import { ApprovalCard, ApprovalIcon } from "@/components/ApprovalCard";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { DecidedApproval, Decision, PendingApproval } from "@/lib/api";
import { APPROVALS, HISTORY, PENDING, approvalTag, historyLine, waitingLine } from "@/lib/approvals";
import { closeWaiting } from "@/lib/pwa";
import { thisDevice } from "@/lib/push";

export interface ApprovalsViewProps {
  /** Undefined while they are fetched. */
  pending?: PendingApproval[];
  /** Undefined while the first page is fetched. */
  history?: DecidedApproval[];
  /** False where Niles has no database and keeps no history. */
  kept?: boolean;
  /** An older page is there to be asked for. */
  more?: boolean;
  onMore?: () => void;
  /** The approval a decision is on its way for. */
  deciding?: string | null;
  onDecide: (approval: string, decision: Decision) => void;
  /** Go to the conversation a card is in, where there is one to go to. */
  onOpenConversation?: (id: number) => void;
  /** The card a link led to. */
  highlight?: string | null;
  error?: string;
}

/**
 * What waits for your yes, and what you decided: the same cards the
 * chat shows, gathered in one place, with the answers under them.
 *
 * Needs review first, because it is the only part that asks anything of
 * you. The history is a list, not cards: it is read, never acted on.
 */
export function ApprovalsView({
  pending,
  history,
  kept = true,
  more,
  onMore,
  deciding,
  onDecide,
  onOpenConversation,
  highlight,
  error,
}: ApprovalsViewProps) {
  const waitingId = useId();
  const historyId = useId();
  const now = new Date();
  const heading = "text-muted-foreground px-1 text-xs font-medium tracking-wide uppercase";

  return (
    <div className="flex flex-col gap-6">
      <section aria-labelledby={waitingId} className="flex flex-col gap-2">
        <h3 id={waitingId} className={heading}>
          Needs review
        </h3>
        {pending === undefined ? (
          <Skeleton className="h-32 w-full rounded-2xl" />
        ) : pending.length === 0 ? (
          <p className="text-muted-foreground px-1 text-sm">Nothing is waiting for you.</p>
        ) : (
          pending.map((approval) => (
            <ApprovalCard
              key={approval.id}
              card={approval}
              folded
              summary={approval.summary}
              deciding={deciding === approval.id}
              highlighted={approval.id === highlight}
              onDecide={(decision) => onDecide(approval.id, decision)}
              footer={
                <div className="text-muted-foreground flex flex-wrap items-center justify-between gap-2 text-xs">
                  <span>{waitingLine(approval, now)}</span>
                  {approval.conversation_id !== null && onOpenConversation && (
                    <Button
                      variant="link"
                      size="sm"
                      className="h-auto px-0 text-xs"
                      onClick={() => onOpenConversation(approval.conversation_id!)}
                    >
                      <MessageCircle aria-hidden />
                      Show in the chat
                    </Button>
                  )}
                </div>
              }
            />
          ))
        )}
        {error && <p className="text-destructive px-1 text-sm">{error}</p>}
      </section>

      <section aria-labelledby={historyId} className="flex flex-col gap-2">
        <h3 id={historyId} className={heading}>
          Approvals history
        </h3>
        {!kept ? (
          <p className="text-muted-foreground px-1 text-sm">
            Niles has no database here, so it keeps no history of what you decided.
          </p>
        ) : history === undefined ? (
          <Skeleton className="h-24 w-full rounded-2xl" />
        ) : history.length === 0 ? (
          <p className="text-muted-foreground px-1 text-sm">Nothing decided yet.</p>
        ) : (
          <ul className="bg-muted/50 divide-border/60 divide-y overflow-hidden rounded-2xl">
            {history.map((entry) => (
              <li key={entry.id} className="flex items-start gap-3 px-3 py-2.5">
                <ApprovalIcon icon={entry.icon} label={entry.tool} small />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium">{entry.title}</p>
                  <p className="text-muted-foreground truncate text-xs">{entry.summary}</p>
                  <p
                    className={
                      entry.status === "failed"
                        ? "text-destructive text-xs"
                        : "text-muted-foreground text-xs"
                    }
                  >
                    {historyLine(entry, now)}
                  </p>
                </div>
              </li>
            ))}
          </ul>
        )}
        {more && (
          <Button variant="ghost" size="sm" className="self-start" onClick={onMore}>
            Show older
          </Button>
        )}
      </section>
    </div>
  );
}

/**
 * Approvals, fetched and answered: in Niles's panel and on My profile.
 * A decision here goes the way a card's button sends it, so the card in
 * the conversation says what became of it too.
 */
export function Approvals({
  onOpenConversation,
  highlight,
}: {
  onOpenConversation?: (id: number) => void;
  highlight?: string | null;
}) {
  const queryClient = useQueryClient();
  const pending = useQuery({ queryKey: PENDING, queryFn: api.approvals, retry: false });
  const history = useInfiniteQuery({
    queryKey: HISTORY,
    queryFn: ({ pageParam }) => api.approvalHistory(pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.next ?? undefined,
    retry: false,
  });
  const decide = useMutation({
    mutationFn: async ({ approval, decision }: { approval: string; decision: Decision }) =>
      api.decideHeld(approval, decision, await thisDevice()),
    // Answered here: its notification on this device goes now. Niles's
    // push saying what became of it skips the device it was answered on.
    onSuccess: (_, { approval }) => void closeWaiting(approvalTag(approval)),
    // Either way: a 409 means it lapsed or was answered elsewhere, and
    // both lists, and the card in the chat, should say so.
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: APPROVALS });
      void queryClient.invalidateQueries({ queryKey: ["chat"] });
    },
  });

  const pages = history.data?.pages;
  return (
    <ApprovalsView
      pending={pending.isError ? [] : pending.data?.approvals}
      history={history.isError ? [] : pages?.flatMap((p) => p.entries)}
      kept={pages?.[0]?.kept ?? true}
      more={history.hasNextPage}
      onMore={() => void history.fetchNextPage()}
      deciding={decide.isPending ? decide.variables.approval : null}
      onDecide={(approval, decision) => decide.mutate({ approval, decision })}
      onOpenConversation={onOpenConversation}
      highlight={highlight}
      error={decide.error?.message ?? pending.error?.message ?? history.error?.message}
    />
  );
}
