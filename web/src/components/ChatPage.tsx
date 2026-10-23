import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { QueryClient } from "@tanstack/react-query";
import { Bookmark, MessagesSquare, ShieldCheck } from "lucide-react";
import { Approvals } from "@/components/ApprovalsList";
import { ArtifactView } from "@/components/ArtifactView";
import { ChatView } from "@/components/ChatView";
import type { ShownTurn } from "@/components/ChatView";
import { ConversationList } from "@/components/ConversationList";
import { NilesPanel } from "@/components/NilesPanel";
import type { PanelTab } from "@/components/NilesPanel";
import { SavedList } from "@/components/SavedList";
import { Skeleton } from "@/components/ui/skeleton";
import { useDeviceStream } from "@/hooks/useDeviceStream";
import { api } from "@/lib/api";
import type { Artifact, ArtifactPatch, Conversation, ConversationSummary, Decision } from "@/lib/api";
import { APPROVALS, PENDING, approvalTag } from "@/lib/approvals";
import { ARTIFACTS, ARTIFACT_LIST, workingLine } from "@/lib/artifacts";
import { closeWaiting } from "@/lib/pwa";
import { thisDevice } from "@/lib/push";
import { hashQuery } from "@/lib/route";
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
 * on its way, else what it is writing in the background, else when it
 * was last talked to, which is the one thing about it this page knows
 * for certain.
 */
export function panelStatus(
  thinking: boolean,
  conversations: ConversationSummary[] | undefined,
  artifacts?: Artifact[],
  now: Date = new Date(),
): string {
  if (thinking) return "Thinking…";
  const writing = workingLine(artifacts);
  if (writing) return writing;
  const latest = conversations?.[0];
  if (!latest) return "Ready";
  return `Last talked ${relativeTime(latest.last_at, now)}`;
}

/** The artifact a link asks the chat to open: `#/chat?artifact=12`,
    which is where a notification about it leads. */
export function linkedArtifact(hash: string): number | null {
  const id = Number(hashQuery(hash).get("artifact"));
  return Number.isInteger(id) && id > 0 ? id : null;
}

/** The approval a link is about: `#/chat?approval=…`, where every
    notification about one leads. */
export function linkedApproval(hash: string): string | null {
  return hashQuery(hash).get("approval") || null;
}

/** The panel tab a link asks the chat to open: `#/chat?panel=approvals`. */
export function linkedPanel(hash: string): "approvals" | null {
  return hashQuery(hash).get("panel") === "approvals" ? "approvals" : null;
}

/** Where an approval's card is, for a link to it: the conversation it is
    in, from what waits or else what was decided lately; none for one
    asked aloud, or one Niles has no record of. */
async function whereIs(queryClient: QueryClient, approval: string): Promise<number | null> {
  try {
    const { approvals } = await queryClient.fetchQuery({
      queryKey: PENDING,
      queryFn: api.approvals,
      retry: false,
      staleTime: 0,
    });
    const waiting = approvals.find((a) => a.id === approval);
    if (waiting) return waiting.conversation_id;
    const { entries } = await api.approvalHistory();
    return entries.find((e) => e.approval === approval)?.conversation_id ?? null;
  } catch {
    return null;
  }
}

/**
 * Every conversation the signed-in person has had with Niles, spoken or
 * typed, one on screen at a time, kept on the server so the next message
 * has its history as context.
 */
export function ChatPage() {
  const queryClient = useQueryClient();
  const list = useQuery({ queryKey: LIST, queryFn: api.conversations });
  const saved = useQuery({ queryKey: ARTIFACT_LIST, queryFn: api.artifacts });
  // For the count on the Approvals tab; the tab itself asks the same.
  const pending = useQuery({ queryKey: PENDING, queryFn: api.approvals, retry: false });
  // Saved, and the status line, follow the jobs as they go.
  useDeviceStream();
  // `null` is an empty new chat; undefined is "not chosen yet", settled
  // once from the list. Settled rather than worked out on every render,
  // or a refetch after five quiet minutes would empty the screen under
  // whoever is reading it.
  const [chosen, setChosen] = useState<number | null>();
  // The card a notification's link led to, drawn out in its conversation
  // or in Approvals.
  const [highlight, setHighlight] = useState<string | null>(null);
  // A link to an approval, still being looked up: the conversation the
  // chat lands on waits for it.
  const [following, setFollowing] = useState<string | null>(() =>
    linkedApproval(window.location.hash),
  );
  const navigation = useRef(0);
  const stopFollowing = () => {
    navigation.current += 1;
    setFollowing(null);
    if (linkedApproval(window.location.hash) !== null) {
      window.history.replaceState(null, "", "#/chat");
    }
  };
  if (chosen === undefined && list.data && following === null) {
    setChosen(landingOn(list.data));
  }
  const current = chosen ?? null;
  const conversation = useQuery({
    queryKey: one(current ?? 0),
    queryFn: () => api.conversation(current!),
    enabled: current !== null,
  });
  const [panelOpen, setPanelOpen] = useState(false);
  const [tab, setTab] = useState("conversations");
  // The artifact open in Saved; null is the list.
  const [opened, setOpened] = useState<number | null>(null);
  const avatar = useRef<HTMLButtonElement>(null);
  // Kept here because the server does not: only a reply just sent says
  // which assistant did not write it, and why. Keyed by conversation and
  // turn.
  const [fallbacks, setFallbacks] = useState<
    Record<string, Pick<ShownTurn, "fallback" | "fallback_from">>
  >({});

  const send = useMutation({
    mutationFn: ({ text, files, to }: { text: string; files: File[]; to: number | null }) =>
      api.sendChat(text, to === null ? { new: true } : { conversation_id: to }, files),
    onSuccess: (
      { reply, via, fallback, fallback_from, conversation_id, attachments },
      { text: said, to },
    ) => {
      const now = new Date().toISOString();
      const turn = {
        at: now,
        said,
        reply,
        via: "app",
        room: null,
        by: via ?? null,
        proposal: null,
        attachments: attachments ?? [],
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
    mutationFn: async ({
      id,
      approval,
      decision,
    }: {
      id: number;
      approval: string;
      decision: Decision;
    }) => api.decideApproval(id, approval, decision, await thisDevice()),
    // Answered here: its notification on this device goes now. Niles's
    // push saying what became of it skips the device it was answered on.
    onSuccess: (_, { approval }) => void closeWaiting(approvalTag(approval)),
    // Refetched either way: a 409 means it lapsed or was answered
    // elsewhere, and the card should say so — and the panel with it.
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: APPROVALS });
      return queryClient.invalidateQueries({ queryKey: CHAT });
    },
  });

  const changeArtifact = useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: ArtifactPatch }) =>
      api.patchArtifact(id, patch),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ARTIFACTS }),
  });

  const removeArtifact = useMutation({
    mutationFn: (id: number) => api.deleteArtifact(id),
    onSuccess: async (_, id) => {
      queryClient.removeQueries({ queryKey: [...ARTIFACTS, "one", id] });
      await queryClient.invalidateQueries({ queryKey: ARTIFACTS });
      // Its cards in the chat go with it.
      void queryClient.invalidateQueries({ queryKey: CHAT });
    },
  });

  const openArtifact = (id: number) => {
    stopFollowing();
    setTab("saved");
    setOpened(id);
    setPanelOpen(true);
  };

  // A notification's link, on the way in and while the chat is open:
  // tapping one focuses this window and sets its hash. Put back to the
  // bare chat afterwards, or a refresh would open it again.
  useEffect(() => {
    const follow = () => {
      const hash = window.location.hash;
      const id = linkedArtifact(hash);
      const approval = linkedApproval(hash);
      const panel = linkedPanel(hash);
      const request = ++navigation.current;
      setFollowing(null);
      if (id !== null) {
        setTab("saved");
        setOpened(id);
        setPanelOpen(true);
      } else if (approval !== null) {
        setFollowing(approval);
        void whereIs(queryClient, approval).then((conversation) => {
          if (navigation.current !== request) return;
          setHighlight(approval);
          if (conversation !== null) {
            setChosen(conversation);
            setPanelOpen(false);
          } else {
            setTab("approvals");
            setPanelOpen(true);
          }
          setFollowing(null);
          window.history.replaceState(null, "", "#/chat");
        });
        return;
      } else if (panel !== null) {
        setTab(panel);
        setPanelOpen(true);
      } else {
        return;
      }
      window.history.replaceState(null, "", "#/chat");
    };
    follow();
    window.addEventListener("hashchange", follow);
    return () => {
      navigation.current += 1;
      window.removeEventListener("hashchange", follow);
    };
  }, [queryClient]);

  const remove = useMutation({
    mutationFn: (id: number) => api.deleteConversation(id),
    onSuccess: async (_, id) => {
      if (id === current) setChosen(null);
      queryClient.removeQueries({ queryKey: one(id) });
      await queryClient.invalidateQueries({ queryKey: LIST });
    },
  });

  const startNew = () => {
    stopFollowing();
    setChosen(null);
    setHighlight(null);
    setPanelOpen(false);
    send.reset();
    answer.reset();
  };

  const pick = (id: number) => {
    stopFollowing();
    setChosen(id);
    setHighlight(null);
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
    {
      id: "saved",
      label: "Saved",
      icon: Bookmark,
      render: () =>
        opened !== null ? (
          <ArtifactView id={opened} onBack={() => setOpened(null)} />
        ) : (
          <SavedList
            artifacts={saved.data?.artifacts}
            assistant={saved.data?.assistant}
            error={saved.error?.message}
            onOpen={setOpened}
            onChange={(id, patch) => changeArtifact.mutateAsync({ id, patch })}
            onDelete={(id) => removeArtifact.mutateAsync(id)}
          />
        ),
    },
    {
      id: "approvals",
      label: "Approvals",
      icon: ShieldCheck,
      badge: pending.data?.approvals.length ?? 0,
      render: () => <Approvals onOpenConversation={pick} highlight={highlight} />,
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
        pendingAttachments={
          send.isPending && send.variables.to === current ? send.variables.files : undefined
        }
        conversationId={current}
        error={
          send.error?.message ??
          answer.error?.message ??
          decide.error?.message ??
          conversation.error?.message
        }
        onSend={(text, files) => send.mutate({ text, files, to: current })}
        onNew={startNew}
        onOpenPanel={() => setPanelOpen((open) => !open)}
        avatarRef={avatar}
        panelOpen={panelOpen}
        onAnswer={(turn, save) => current !== null && answer.mutate({ id: current, turn, save })}
        answering={answer.isPending}
        onDecide={(approval, decision) =>
          current !== null && decide.mutate({ id: current, approval, decision })
        }
        deciding={decide.isPending}
        onDictate={async (audio) => (await api.dictate(audio)).text}
        onOpenArtifact={openArtifact}
        working={saved.data?.artifacts.filter((a) => a.status === "working").map((a) => a.id)}
        highlight={highlight}
      />
      <NilesPanel
        open={panelOpen}
        onOpenChange={setPanelOpen}
        anchor={avatar}
        status={panelStatus(send.isPending, list.data, saved.data?.artifacts)}
        tabs={tabs}
        tab={tab}
        onTabChange={setTab}
      />
    </>
  );
}
