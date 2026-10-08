import { useEffect, useRef, useState } from "react";
import { ArrowUp, LoaderCircle, Mic, RotateCcw, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { dictationSupported, useDictation } from "@/hooks/useDictation";
import type { Exchange } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface ChatViewProps {
  exchanges: Exchange[];
  /** Sent and not answered yet. */
  pending?: string;
  error?: string;
  onSend: (text: string) => void;
  onForget: () => void;
  /** Words from a recording. Without it there is no microphone. */
  onDictate?: (audio: Blob) => Promise<string>;
}

/** Things worth asking that show what typing to Niles is for. */
export const SUGGESTIONS = [
  "What's on the shopping list?",
  "Is anyone home?",
  "Turn off the lights downstairs",
];

/**
 * A conversation with Niles, typed.
 */
export function ChatView({
  exchanges,
  pending,
  error,
  onSend,
  onForget,
  onDictate,
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
  }, [exchanges.length, pending]);

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
    <div className="flex flex-col gap-4">
      {exchanges.length === 0 && !busy ? (
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
      ) : (
        <div className="flex justify-end">
          <Button variant="ghost" size="sm" onClick={onForget} disabled={busy}>
            <RotateCcw />
            New conversation
          </Button>
        </div>
      )}

      <ol className="flex flex-col gap-3" aria-label="Conversation">
        {exchanges.map((exchange, i) => (
          <li key={i} className="flex flex-col gap-3">
            <Bubble from="you">{exchange.said}</Bubble>
            <Bubble from="niles">{exchange.reply}</Bubble>
            {(exchange.via || exchange.fallback) && <Credit exchange={exchange} />}
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
          onFocus={() => (document.documentElement.dataset.typing = "")}
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
 * of it. A fallback says why, because Claude Code failing quietly looked
 * exactly like Claude Code being fast.
 */
function Credit({ exchange }: { exchange: Exchange }) {
  return (
    <p className="text-muted-foreground -mt-2 self-start px-1 text-[11px]">
      {exchange.via === "claude"
        ? "Claude"
        : `Answered without Claude — ${exchange.fallback}`}
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
