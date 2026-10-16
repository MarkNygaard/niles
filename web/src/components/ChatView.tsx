import { useEffect, useRef, useState } from "react";
import { ArrowUp, LoaderCircle, Mic, Square, SquarePen } from "lucide-react";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ArtifactCard } from "@/components/ArtifactCard";
import { NilesAvatar } from "@/components/NilesPanel";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { dictationSupported, useDictation } from "@/hooks/useDictation";
import type { AssistantId, Decision, Turn } from "@/lib/api";
import { humanize } from "@/lib/rooms";
import { cn } from "@/lib/utils";

/** A turn as the chat shows it: what the server keeps, and for a reply
    just sent, which assistant did not write it and why. */
export type ShownTurn = Turn & { fallback?: string; fallback_from?: AssistantId };

export const ASSISTANT_NAMES: Record<AssistantId, string> = {
  claude: "Claude",
  chatgpt: "ChatGPT",
};

export interface ChatViewProps {
  turns: ShownTurn[];
  /** The conversation is on its way. */
  loading?: boolean;
  /** Sent and not answered yet. */
  pending?: string;
  error?: string;
  onSend: (text: string) => void;
  /** Leave this conversation for an empty one. */
  onNew: () => void;
  /** Open Niles's panel, which holds the past conversations. */
  onOpenPanel: () => void;
  /** The avatar that opens it, for the panel to grow from. */
  avatarRef?: React.Ref<HTMLButtonElement>;
  /** Whether the panel is open. */
  panelOpen?: boolean;
  /** Save, or not, what the last turn proposed; `turn` is its index. */
  onAnswer: (turn: number, save: boolean) => void;
  /** An answer to a proposal is on its way. */
  answering?: boolean;
  /** Allow or deny what an approval card holds. */
  onDecide?: (approval: string, decision: Decision) => void;
  /** A decision is on its way. */
  deciding?: boolean;
  /** Words from a recording. Without it there is no microphone. */
  onDictate?: (audio: Blob) => Promise<string>;
  /** Open an artifact a reply made, in Niles's panel. */
  onOpenArtifact?: (id: number) => void;
}

/** Things worth asking that show what typing to Niles is for. */
export const SUGGESTIONS = [
  "What's on the shopping list?",
  "Is anyone home?",
  "Turn off the lights downstairs",
];

/**
 * One conversation with Niles, spoken to a satellite or typed here, and
 * the field to carry it on.
 */
export function ChatView({
  turns,
  loading,
  pending,
  error,
  onSend,
  onNew,
  onOpenPanel,
  avatarRef,
  panelOpen,
  onAnswer,
  answering,
  onDecide,
  deciding,
  onDictate,
  onOpenArtifact,
}: ChatViewProps) {
  const [draft, setDraft] = useState("");
  const [transcribing, setTranscribing] = useState(false);
  const [dictationError, setDictationError] = useState<string>();
  const dictation = useDictation(async (audio) => {
    if (!onDictate) return;
    setTranscribing(true);
    setDictationError(undefined);
    try {
      const text = await onDictate(audio);
      // Into the field, not sent: a misheard word is easier to fix
      // before Niles has acted on it.
      if (text) setDraft((d) => (d.trim() ? `${d.trimEnd()} ${text}` : text));
    } catch (e) {
      setDictationError(e instanceof Error ? e.message : String(e));
    } finally {
      setTranscribing(false);
    }
  });
  const canDictate = Boolean(onDictate) && dictationSupported();
  const composer = useRef<HTMLFormElement>(null);
  const busy = pending !== undefined;
  // What the pinned composer covers, so the last message can scroll
  // clear of it. Measured, because the field grows with what is typed.
  const [composerHeight, setComposerHeight] = useState(56);

  useEffect(() => {
    const el = composer.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() =>
      setComposerHeight(el.getBoundingClientRect().height),
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // The newest message is the one worth seeing. `#root` is what scrolls,
  // and only `#root` is moved: `scrollIntoView` also scrolls the window,
  // which on iOS pans the screen under the keyboard and starts it and
  // Safari correcting each other.
  useEffect(() => {
    const root = document.getElementById("root");
    root?.scrollTo?.({ top: root.scrollHeight });
  }, [turns.length, pending]);

  // The tab bar steps aside while the keyboard is up; see TabBar.
  useEffect(
    () => () => {
      delete document.documentElement.dataset.typing;
    },
    [],
  );

  const send = (text: string) => {
    const message = text.trim();
    if (!message || busy) return;
    onSend(message);
    setDraft("");
  };

  return (
    // The screen under the title, at least, so the conversation can sit
    // at its bottom: page padding (see App) is 0.25rem on a phone and
    // 1.5rem from `sm`, the header 2.25rem, the gap under it 0.5rem on a
    // phone and 1rem from `sm`.
    <div className="flex min-h-[calc(100dvh-env(safe-area-inset-top)-3rem)] flex-col gap-4 sm:min-h-[calc(100dvh-env(safe-area-inset-top)-4.75rem)]">
      {/* Niles at the head of the conversation, the way a messages app
          puts the other person there; tapping the face opens what it
          keeps. New chat stays out here as well, for the one thing done
          often enough not to want a panel in the way. */}
      <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-2">
        <span aria-hidden />
        <button
          ref={avatarRef}
          type="button"
          aria-label="Open Niles"
          aria-haspopup="dialog"
          aria-expanded={panelOpen ?? false}
          onClick={onOpenPanel}
          className={cn(
            "rounded-full transition-transform active:scale-95",
            "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          )}
        >
          <NilesAvatar className="size-10" />
        </button>
        {(turns.length > 0 || busy) && (
          <Button
            variant="ghost"
            size="icon-lg"
            onClick={onNew}
            disabled={busy}
            aria-label="New chat"
            title="New chat"
            className="text-muted-foreground justify-self-end"
          >
            <SquarePen className="size-5" />
          </Button>
        )}
      </div>

      {loading && (
        <div className="flex flex-col gap-3" aria-label="Loading the conversation">
          <Skeleton className="h-9 w-2/3 self-end rounded-2xl" />
          <Skeleton className="h-16 w-3/4 rounded-2xl" />
        </div>
      )}
      {!loading && turns.length === 0 && !busy && (
        <div className="flex flex-col gap-3 px-1">
          <p className="text-muted-foreground text-sm">
            Anything you would say to Niles out loud works here too.
          </p>
          <div className="flex flex-wrap gap-1.5">
            {SUGGESTIONS.map((text) => (
              <button
                key={text}
                type="button"
                onClick={() => send(text)}
                className={cn(
                  "bg-card rounded-full px-3 py-1.5 text-sm transition-colors",
                  "hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                )}
              >
                {text}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* At the bottom, just above the field, the way a messages app
          has it. At the top, a short conversation was what iOS slid off
          the screen when it lifted the field above the keyboard. */}
      <ol className="mt-auto flex flex-col gap-3" aria-label="Conversation">
        {turns.map((turn, i) => (
          <li key={i} className="flex flex-col gap-3">
            {/* Nothing said is Niles speaking first: a job that was
                asked for earlier, finished. */}
            {turn.said && <Bubble from="you">{turn.said}</Bubble>}
            {turn.via === "voice" && <Spoken room={turn.room} />}
            <Bubble from="niles">{turn.reply}</Bubble>
            {(turn.by || turn.fallback) && <Credit turn={turn} />}
            {turn.artifacts?.map((card) => (
              <ArtifactCard key={card.id} card={card} onOpen={onOpenArtifact} />
            ))}
            {turn.approval && (
              <ApprovalCard
                card={turn.approval}
                deciding={deciding}
                onDecide={(decision) => turn.approval && onDecide?.(turn.approval.id, decision)}
              />
            )}
            {/* Only the last turn's proposal is still open: once
                anything follows it, it has been answered one way or
                the other. */}
            {turn.proposal && i === turns.length - 1 && !busy && (
              <div className="flex gap-2 self-start px-1">
                <Button size="sm" disabled={answering} onClick={() => onAnswer(i, true)}>
                  Save
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={answering}
                  onClick={() => onAnswer(i, false)}
                >
                  Don't save
                </Button>
              </div>
            )}
          </li>
        ))}
        {busy && (
          <li className="flex flex-col gap-3">
            <Bubble from="you">{pending}</Bubble>
            <Bubble from="niles">
              <span className="text-muted-foreground" aria-label="Niles is answering">
                …
              </span>
            </Bubble>
          </li>
        )}
      </ol>

      {(error ?? dictation.error ?? dictationError) && (
        <p className="text-destructive text-sm">
          {error ?? dictation.error ?? dictationError}
        </p>
      )}

      {/* Pinned to the screen rather than to the end of the page, the
          way a messages app does it: just above the tab bar, and at the
          very bottom once the tab bar steps aside for the keyboard.
          From there the keyboard is the browser's business: iOS pans
          the screen up until the field is above the keys, Android
          shrinks the page. Moving it ourselves as well — by the
          keyboard's height, read from visualViewport — had the two of
          us correcting each other, and on an iPhone it jumped to the
          top and flickered. */}
      <form
        ref={composer}
        onSubmit={(e) => {
          e.preventDefault();
          send(draft);
        }}
        className={cn(
          "bg-background fixed inset-x-0 z-30",
          "bottom-[calc(env(safe-area-inset-bottom)+3rem)] in-data-typing:bottom-0",
          "sm:bottom-0 sm:pb-[env(safe-area-inset-bottom)]",
        )}
      >
        <div className="mx-auto flex w-full max-w-5xl items-end gap-2 px-4 py-2 sm:px-6">
        <Textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onFocus={() => {
            document.documentElement.dataset.typing = "";
            // Start from the newest message, so the end of the
            // conversation is what the keyboard's lift keeps in view.
            const root = document.getElementById("root");
            root?.scrollTo?.({ top: root.scrollHeight });
          }}
          onBlur={() => delete document.documentElement.dataset.typing}
          onKeyDown={(e) => {
            // Enter sends, as in every chat; Shift+Enter is a new line.
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              send(draft);
            }
          }}
          rows={1}
          placeholder={
            dictation.recording ? "Listening…" : transcribing ? "Writing it down…" : "Message Niles"
          }
          aria-label="Message Niles"
          className="max-h-40 min-h-10 resize-none"
        />
        {/* The microphone where Send would be while there is nothing to
            send, the way a phone's messages app does it. */}
        {canDictate && dictation.recording ? (
          <Button
            type="button"
            size="icon-lg"
            variant="destructive"
            aria-label="Stop dictating"
            onClick={dictation.stop}
            onMouseDown={(e) => e.preventDefault()}
            className="rounded-full"
          >
            <Square className="fill-current" />
          </Button>
        ) : canDictate && (transcribing || !draft.trim()) ? (
          <Button
            type="button"
            size="icon-lg"
            variant="secondary"
            aria-label="Dictate"
            onClick={dictation.start}
            onMouseDown={(e) => e.preventDefault()}
            disabled={transcribing}
            className="rounded-full"
          >
            {transcribing ? <LoaderCircle className="animate-spin" /> : <Mic />}
          </Button>
        ) : (
          <Button
            type="submit"
            size="icon-lg"
            aria-label="Send"
            disabled={!draft.trim() || busy}
            // Sending keeps the keyboard up for the next message, and a
            // tap that blurred the field first would drop the composer
            // out from under the finger.
            onMouseDown={(e) => e.preventDefault()}
            className="rounded-full"
          >
            <ArrowUp />
          </Button>
        )}
        </div>
      </form>
      {/* Room at the end for what is pinned over it — the composer, and
          below that the tab bar — so the last message scrolls clear of
          both. The page leaves its own bottom padding off this screen
          (see App), so this is the whole of it. */}
      <div
        aria-hidden
        style={{ height: composerHeight }}
        className="box-content pb-[calc(env(safe-area-inset-bottom)+3rem)] in-data-typing:pb-0 sm:pb-0"
      />
    </div>
  );
}

/**
 * Who wrote a reply, when that is worth saying.
 *
 * Under the bubble rather than in it: it is about the reply, not part
 * of it. A fallback says why, because an assistant failing quietly looked
 * exactly like an assistant being fast.
 */
function Credit({ turn }: { turn: ShownTurn }) {
  const missed = ASSISTANT_NAMES[turn.fallback_from ?? "claude"];
  return (
    <p className="text-muted-foreground -mt-2 self-start px-1 text-[11px]">
      {turn.by
        ? (ASSISTANT_NAMES[turn.by] ?? turn.by)
        : `Answered without ${missed} — ${turn.fallback}`}
    </p>
  );
}

/** Said out loud to a satellite rather than typed, and in which room. */
function Spoken({ room }: { room: string | null }) {
  return (
    <p className="text-muted-foreground -mt-2 flex items-center gap-1 self-end px-1 text-[11px]">
      <Mic className="size-3" aria-hidden />
      {room ? humanize(room) : "Spoken"}
    </p>
  );
}

function Bubble({ from, children }: { from: "you" | "niles"; children: React.ReactNode }) {
  return (
    <div
      className={cn(
        // Pre-wrap: a typed reply may come as a few lines or a list,
        // and those line breaks are the formatting.
        "max-w-[85%] rounded-2xl px-3.5 py-2 text-sm whitespace-pre-wrap",
        from === "you"
          ? "bg-primary text-primary-foreground self-end rounded-br-md"
          : "bg-card self-start rounded-bl-md",
      )}
    >
      {children}
    </div>
  );
}
