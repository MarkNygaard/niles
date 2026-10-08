import { useEffect, useRef, useState } from "react";
import { ArrowUp, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import type { Exchange } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface ChatViewProps {
  exchanges: Exchange[];
  /** Sent and not answered yet. */
  pending?: string;
  error?: string;
  onSend: (text: string) => void;
  onForget: () => void;
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
export function ChatView({ exchanges, pending, error, onSend, onForget }: ChatViewProps) {
  const [draft, setDraft] = useState("");
  const end = useRef<HTMLDivElement>(null);
  const busy = pending !== undefined;

  // The newest message is the one worth seeing. `#root` scrolls, so
  // scrolling the last element into view is what reaches it.
  useEffect(() => {
    end.current?.scrollIntoView?.({ block: "end" });
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
    // At least the screen left under the header, so the field starts at
    // the bottom where a thumb expects it, not under the last message.
    <div className="flex min-h-[calc(100dvh-env(safe-area-inset-top)-env(safe-area-inset-bottom)-9.25rem)] flex-col gap-4 sm:min-h-[calc(100dvh-env(safe-area-inset-top)-env(safe-area-inset-bottom)-6.25rem)]">
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

      {error && <p className="text-destructive text-sm">{error}</p>}

      {/* Held at the bottom of the screen, above the tab bar on a phone
          and on top of the keyboard once the tab bar steps aside. */}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          send(draft);
        }}
        className={cn(
          "bg-background sticky mt-auto flex items-end gap-2 py-2",
          "bottom-[calc(env(safe-area-inset-bottom)+3.5rem)] in-data-typing:bottom-0 sm:bottom-0",
        )}
      >
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
          placeholder="Message Niles"
          aria-label="Message Niles"
          className="max-h-40 min-h-10 resize-none"
        />
        <Button
          type="submit"
          size="icon-lg"
          aria-label="Send"
          disabled={!draft.trim() || busy}
          className="rounded-full"
        >
          <ArrowUp />
        </Button>
      </form>
      <div ref={end} />
    </div>
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
