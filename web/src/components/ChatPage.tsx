import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { MessagesSquare } from "lucide-react";
import { ChatView } from "@/components/ChatView";
import type { ShownTurn } from "@/components/ChatView";
import { ConversationList } from "@/components/ConversationList";
import { NilesPanel } from "@/components/NilesPanel";
import type { PanelTab } from "@/components/NilesPanel";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { Conversation, ConversationSummary, Decision } from "@/lib/api";
import { relativeTime } from "@/lib/skills";

export const CHAT = ["chat"];
const LIST = [...CHAT, "conversations"];
const one = (id: number) => [...CHAT, "conversation", id];

/** The quiet after which the server starts a new conversation. */
const QUIET_MS = 5 * 60 * 1000;

/**
 * Which conversation the tab opens on: the latest, while it is still
 * going by the server's five-minute rule, else an empty new one.
 */
export function landingOn(
  conversations: ConversationSummary[],
  now: number = Date.now(),
): number | null {
  const latest = conversations[0];
  if (!latest) return null;
  return now - new Date(latest.last_at).getTime() < QUIET_MS ? latest.id : null;
}

/**
 * The line under Niles's name in its panel: thinking while a reply is
 * on its way, else when it was last talked to, which is the one thing
 * about it this page knows for certain.
 */
export function panelStatus(
  thinking: boolean,
  conversations: ConversationSummary[] | undefined,
  now: Date = new Date(),
): string {
  if (thinking) return "Thinking…";
  const latest = conversations?.[0];
  if (!latest) return "Ready";
  return `Last talked ${relativeTime(latest.last_at, now)}`;
}

/**
 * Every conversation the signed-in person has had with Niles, spoken or
 * typed, one on screen at a time, kept on the server so the next message
 * has its history as context.
 */
export function ChatPage() {
  const queryClient = useQueryClient();
  const list = useQuery({ queryKey: LIST, queryFn: api.conversations });
  // `null` is an empty new chat; undefined is "not chosen yet", settled
  // once from the list. Settled rather than worked out on every render,
  // or a refetch after five quiet minutes would empty the screen under
  // whoever is reading it.
  const [chosen, setChosen] = useState<number | null>();
  if (chosen === undefined && list.data) {
    setChosen(landingOn(list.data));
  }
  const current = chosen ?? null;
  const conversation = useQuery({
    queryKey: one(current ?? 0),
    queryFn: () => api.conversation(current!),
    enabled: current !== null,
  });
  const [panelOpen, setPanelOpen] = useState(false);
  const avatar = useRef<HTMLButtonElement>(null);
  // Kept here because the server does not: only a reply just sent says
  // which assistant did not write it, and why. Keyed by conversation and
  // turn.
  const [fallbacks, setFallbacks] = useState<
    Record<string, Pick<ShownTurn, "fallback" | "fallback_from">>
  >({});

  const send = useMutation({
    mutationFn: ({ text, to }: { text: string; to: number | null }) =>
      api.sendChat(text, to === null ? { new: true } : { conversation_id: to }),
    onSuccess: ({ reply, via, fallback, fallback_from, conversation_id }, { text: said, to }) => {
      const now = new Date().toISOString();
      const turn = {
        at: now,
        said,
        reply,
        via: "app",
        room: null,
        by: via ?? null,
        proposal: null,
      } as const;
      // In place at once, so the message does not drop out between the
      // pending bubble going and the refetch landing.
      const old = queryClient.getQueryData<Conversation>(one(conversation_id));
      queryClient.setQueryData<Conversation>(one(conversation_id), {
        id: conversation_id,
        title: old?.title ?? said,
        started_at: old?.started_at ?? now,
        last_at: now,
        turns: [...(old?.turns ?? []), turn],
      });
      if (fallback) {
        const at = old?.turns.length ?? 0;
        setFallbacks((f) => ({
          ...f,
          [`${conversation_id}:${at}`]: { fallback, fallback_from },
        }));
      }
      // Unless somebody went to another conversation while waiting.
      setChosen((shown) => (shown === to ? conversation_id : shown));
      // The server's own copy says whether the reply asked to save a
      // rule, which only it knows. Not awaited: the pending bubble has
      // to go now that the turn is in place, or it shows twice.
      void queryClient.invalidateQueries({ queryKey: CHAT });
    },
  });

  const answer = useMutation({
    mutationFn: ({ id, turn, save }: { id: number; turn: number; save: boolean }) =>
      api.answerProposal(id, turn, save),
    // Refetched whether it worked or not: a 409 means the conversation
    // moved on elsewhere, and the buttons should go with it.
    onSettled: () => queryClient.invalidateQueries({ queryKey: CHAT }),
  });

  const decide = useMutation({
    mutationFn: ({ id, approval, decision }: { id: number; approval: string; decision: Decision }) =>
      api.decideApproval(id, approval, decision),
    // Refetched either way: a 409 means it lapsed or was answered
    // elsewhere, and the card should say so.
    onSettled: () => queryClient.invalidateQueries({ queryKey: CHAT }),
  });

  const remove = useMutation({
    mutationFn: (id: number) => api.deleteConversation(id),
    onSuccess: async (_, id) => {
      if (id === current) setChosen(null);
      queryClient.removeQueries({ queryKey: one(id) });
      await queryClient.invalidateQueries({ queryKey: LIST });
    },
  });

  const startNew = () => {
    setChosen(null);
    setPanelOpen(false);
    send.reset();
    answer.reset();
  };

  const pick = (id: number) => {
    setChosen(id);
    setPanelOpen(false);
    send.reset();
    answer.reset();
  };

  const tabs: PanelTab[] = [
    {
      id: "conversations",
      label: "Conversations",
      icon: MessagesSquare,
      render: () => (
        <ConversationList
          conversations={list.data}
          current={current}
          onPick={pick}
          onNew={startNew}
          onDelete={(id) => remove.mutateAsync(id)}
        />
      ),
    },
  ];

  if (list.isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }
  if (list.isError) {
    return (
      <p className="text-muted-foreground text-sm">Can't reach Niles: {list.error.message}</p>
    );
  }

  const turns: ShownTurn[] =
    current === null
      ? []
      : (conversation.data?.turns ?? []).map((turn, i) => ({
          ...turn,
          ...fallbacks[`${current}:${i}`],
        }));

  return (
    <>
      <ChatView
        turns={turns}
        loading={current !== null && conversation.isLoading}
        pending={send.isPending && send.variables.to === current ? send.variables.text : undefined}
        error={
          send.error?.message ??
          answer.error?.message ??
          decide.error?.message ??
          conversation.error?.message
        }
        onSend={(text) => send.mutate({ text, to: current })}
        onNew={startNew}
        onOpenPanel={() => setPanelOpen(true)}
        avatarRef={avatar}
        panelOpen={panelOpen}
        onAnswer={(turn, save) => current !== null && answer.mutate({ id: current, turn, save })}
        answering={answer.isPending}
        onDecide={(approval, decision) =>
          current !== null && decide.mutate({ id: current, approval, decision })
        }
        deciding={decide.isPending}
        onDictate={async (audio) => (await api.dictate(audio)).text}
      />
      <NilesPanel
        open={panelOpen}
        onOpenChange={setPanelOpen}
        anchor={avatar}
        status={panelStatus(send.isPending, list.data)}
        tabs={tabs}
      />
    </>
  );
}
