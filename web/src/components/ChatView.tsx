import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowUp,
  FileText,
  LoaderCircle,
  Mic,
  Paperclip,
  Square,
  SquarePen,
  TriangleAlert,
  X,
} from "lucide-react";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ArtifactCard } from "@/components/ArtifactCard";
import { PushNudge } from "@/components/PushNudge";
import { NilesAvatar } from "@/components/NilesPanel";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { dictationSupported, useDictation } from "@/hooks/useDictation";
import { ACCEPT, accept, filesFrom, isImage, readable } from "@/lib/attachments";
import type { AssistantId, Attachment, Decision, Turn } from "@/lib/api";
import { chatAttachmentUrl } from "@/lib/api";
import { Markdown } from "@/lib/markdown";
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
  /** The files of the message on its way, for its bubble. */
  pendingAttachments?: File[];
  /** The conversation on screen; its attachments are fetched from it. */
  conversationId?: number | null;
  error?: string;
  onSend: (text: string, files: File[]) => void;
  /** Leave this conversation for an empty one. */
  onNew: () => void;
  /** Open Niles's panel, which holds the past conversations, or close it. */
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
  /** The artifacts being written now. A card keeps the status it had
      when its turn was kept, so this is what says a job still runs. */
  working?: number[];
  /** The approval card a notification's link led to: brought into view
      once it is drawn. */
  highlight?: string | null;
}

/**
 * One conversation with Niles, spoken to a satellite or typed here, and
 * the field to carry it on.
 */
export function ChatView({
  turns,
  loading,
  pending,
  pendingAttachments,
  conversationId,
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
  working,
  highlight,
}: ChatViewProps) {
  const [draft, setDraft] = useState("");
  const [files, setFiles] = useState<File[]>([]);
  const [fileError, setFileError] = useState<string>();
  const [dragging, setDragging] = useState(false);
  // A counter, not a boolean: dragging over a child fires leave+enter
  // on the way through, and only zero really means left.
  const dragDepth = useRef(0);
  const picker = useRef<HTMLInputElement>(null);
  const add = (chosen: File[]) => {
    const { files: next, error } = accept(chosen, files);
    setFiles(next);
    setFileError(error);
  };
  // Object URLs for the chips' thumbnails: made with the list and
  // revoked with it, so nothing leaks. jsdom has no createObjectURL:
  // no thumbnails there, and the chips show the file icon instead.
  const previews = useMemo(() => {
    if (typeof URL.createObjectURL !== "function") return [];
    return files.filter(isImage).map((f) => [f, URL.createObjectURL(f)] as const);
  }, [files]);
  useEffect(
    () => () => previews.forEach(([, url]) => URL.revokeObjectURL(url)),
    [previews],
  );
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

  // A notification's link: its card in the middle of the screen, once,
  // when it is first drawn. Only `#root` is moved, as above.
  const list = useRef<HTMLOListElement>(null);
  const centred = useRef<string | null>(null);
  useEffect(() => {
    if (!highlight) {
      centred.current = null;
      return;
    }
    if (centred.current === highlight) return;
    const root = document.getElementById("root");
    const card = Array.from(
      list.current?.querySelectorAll<HTMLElement>("[data-approval]") ?? [],
    ).find((el) => el.dataset.approval === highlight);
    if (!root || !card) return;
    centred.current = highlight;
    const at = card.getBoundingClientRect();
    const top = at.top - root.getBoundingClientRect().top + root.scrollTop;
    root.scrollTo?.({ top: Math.max(0, top - (root.clientHeight - at.height) / 2) });
  }, [highlight, turns]);

  // The tab bar steps aside while the keyboard is up; see TabBar.
  useEffect(
    () => () => {
      delete document.documentElement.dataset.typing;
    },
    [],
  );

  // The newest card of a job still going is where the offer to notify
  // goes: once, not under every card that was ever working.
  const nudgeUnder = turns
    .flatMap((turn) => turn.artifacts ?? [])
    .filter((card) => working?.includes(card.id))
    .at(-1)?.id;

  const send = (text: string) => {
    const message = text.trim();
    // A file on its own is a message; the field may be empty then.
    if ((!message && files.length === 0) || busy) return;
    onSend(message, files);
    setDraft("");
    setFiles([]);
    setFileError(undefined);
  };

  return (
    // The screen under the title, at least, so the conversation can sit
    // at its bottom: page padding (see App) is 0.25rem on a phone and
    // 1.5rem from `sm`, the header 2.25rem, the gap under it 0.5rem on a
    // phone and 1rem from `sm`.
    <div
      className="flex min-h-[calc(100dvh-env(safe-area-inset-top)-3rem)] flex-col gap-4 sm:min-h-[calc(100dvh-env(safe-area-inset-top)-4.75rem)]"
      onDragEnter={(e) => {
        // Only files: a word dragged out of a reply is not an upload.
        if (!e.dataTransfer?.types.includes("Files")) return;
        // Without preventDefault the browser opens the file instead of
        // letting it be dropped here.
        e.preventDefault();
        dragDepth.current += 1;
        setDragging(true);
      }}
      onDragOver={(e) => e.preventDefault()}
      onDragLeave={() => {
        dragDepth.current = Math.max(0, dragDepth.current - 1);
        if (dragDepth.current === 0) setDragging(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        dragDepth.current = 0;
        setDragging(false);
        // Everything dropped goes to accept, which keeps the good and
        // refuses the rest in a sentence: a .txt drop is told why, not
        // ignored. Paste goes through filesFrom instead, where a plain
        // text paste must keep working untouched.
        add(Array.from(e.dataTransfer?.files ?? []));
      }}
    >
      {dragging && (
        // pointer-events-none: the overlay must not eat the drop that
        // dismisses it.
        // Over the chat's half alone while Niles's panel has the other.
        <div className="bg-background/90 border-primary pointer-events-none fixed inset-0 z-40 m-4 flex items-center justify-center rounded-2xl border-2 border-dashed md:in-data-panel:right-1/2">
          <p className="text-muted-foreground text-sm">Drop a photo or a PDF here</p>
        </div>
      )}
      {/* Niles at the head of the conversation, the way a messages app
          puts the other person there; tapping the face opens what it
          keeps. New chat stays out here as well, for the one thing done
          often enough not to want a panel in the way. */}
      {/* Pinned to the top as the conversation scrolls, the way the
          other person stays at the top of a messages app: under the
          header on a desk (whose height App keeps in --header-h), just
          under the status bar in a hand. What scrolls under it fades out rather
          than meeting a hard edge; the faded strip lets clicks through
          to the text beneath, and only the buttons take them. */}
      <div
        className={cn(
          "pointer-events-none sticky z-20 -mx-4 grid grid-cols-[1fr_auto_1fr] items-center gap-2 px-4 sm:-mx-6 sm:px-6",
          "top-[env(safe-area-inset-top)] sm:top-(--header-h)",
          "from-background via-background/95 -mb-6 bg-linear-to-b from-65% to-transparent pb-6",
        )}
      >
        <span aria-hidden />
        <button
          ref={avatarRef}
          type="button"
          aria-label="Open Niles"
          aria-haspopup="dialog"
          aria-expanded={panelOpen ?? false}
          onClick={onOpenPanel}
          className={cn(
            "pointer-events-auto rounded-full transition-transform active:scale-95",
            "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          )}
        >
          <NilesAvatar className="size-12" />
        </button>
        {(turns.length > 0 || busy) && (
          <Button
            variant="ghost"
            size="icon-lg"
            onClick={onNew}
            disabled={busy}
            aria-label="New chat"
            title="New chat"
            className="text-muted-foreground pointer-events-auto justify-self-end"
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
      {/* At the bottom, just above the field, the way a messages app
          has it. At the top, a short conversation was what iOS slid off
          the screen when it lifted the field above the keyboard. */}
      <ol className="mt-auto flex flex-col gap-3" aria-label="Conversation" ref={list}>
        {turns.map((turn, i) => (
          <li key={i} className="flex flex-col gap-3">
            {/* Nothing said is Niles speaking first: a job that was
                asked for earlier, finished. */}
            {conversationId != null && turn.attachments?.length ? (
              <Attachments conversation={conversationId} items={turn.attachments} />
            ) : null}
            {turn.said && <Bubble from="you">{turn.said}</Bubble>}
            {turn.via === "voice" && <Spoken room={turn.room} />}
            <Bubble from="niles">
              {/* Replies come as markdown — bold, lists, a link —
                  where what the person typed is shown as typed. */}
              <Markdown source={turn.reply} breaks className="gap-2 text-base leading-normal sm:text-sm" />
            </Bubble>
            {(turn.by || turn.fallback) && <Credit turn={turn} />}
            {turn.artifacts?.map((card) => (
              <ArtifactCard key={card.id} card={card} onOpen={onOpenArtifact} />
            ))}
            {nudgeUnder !== undefined &&
              turn.artifacts?.some((card) => card.id === nudgeUnder) && <PushNudge />}
            {turn.approval && (
              <ApprovalCard
                card={turn.approval}
                deciding={deciding}
                highlighted={turn.approval.id === highlight}
                onDecide={(decision) => turn.approval && onDecide?.(turn.approval.id, decision)}
              />
            )}
            {/* Only the last turn's proposal is still open: once
                anything follows it, it has been answered one way or
                the other. */}
            {turn.proposal && i === turns.length - 1 && !busy && (
              <div className="flex max-w-full flex-col gap-2 self-start px-1">
                {/* The rule as Save would keep it, written by the server
                    from the rule itself: what Niles said of it above is
                    its own words, and can be wrong. */}
                {turn.proposal.in_full && (
                  <pre
                    aria-label="The rule in full"
                    className={cn(
                      "bg-muted text-foreground rounded-lg px-3 py-2",
                      "font-sans text-xs leading-relaxed whitespace-pre-wrap",
                    )}
                  >
                    {turn.proposal.in_full}
                  </pre>
                )}
                {turn.proposal.warnings?.map((warning) => (
                  <p key={warning} className="text-muted-foreground flex gap-1.5 text-xs">
                    <TriangleAlert aria-label="Warning" className="mt-0.5 size-3.5 shrink-0" />
                    {warning}
                  </p>
                ))}
                <div className="flex gap-2">
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
              </div>
            )}
          </li>
        ))}
        {busy && (
          <li className="flex flex-col gap-3">
            {pendingAttachments && pendingAttachments.length > 0 && (
              <PendingAttachments files={pendingAttachments} />
            )}
            {/* A file on its own is a message: then there is no said
                bubble, only the chips, the way the kept turn reads
                back. */}
            {pending ? <Bubble from="you">{pending}</Bubble> : null}
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
          // Over the left half only, while Niles's panel has the right.
          "md:in-data-panel:right-1/2 md:transition-[right] md:duration-300 md:ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none",
          "bottom-[calc(env(safe-area-inset-bottom)+3rem)] in-data-typing:bottom-0",
          "sm:bottom-0 sm:pb-[env(safe-area-inset-bottom)]",
        )}
      >
        {/* The files about to go, each removable: above the field the
            way a messages app shows what is attached. */}
        {files.length > 0 && (
          <AttachmentChips
            files={files}
            previews={previews}
            onRemove={(file) => {
              setFiles(files.filter((f) => f !== file));
              setFileError(undefined);
            }}
          />
        )}
        <div className="mx-auto flex w-full max-w-5xl items-end gap-2 px-4 py-2 sm:px-6">
          <input
            ref={picker}
            type="file"
            multiple
            accept={ACCEPT}
            className="hidden"
            onChange={(e) => {
              add(Array.from(e.target.files ?? []));
              // The same file may be picked twice in a row.
              e.target.value = "";
            }}
          />
          {/* On iOS this offers Photo Library, Take Photo and Choose
              File; the camera and the library come free with the
              image types in its accept list. */}
          <Button
            type="button"
            size="icon-lg"
            variant="ghost"
            aria-label="Attach a file"
            onClick={() => picker.current?.click()}
            onMouseDown={(e) => e.preventDefault()}
            className="rounded-full"
          >
            <Paperclip />
          </Button>
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
            onPaste={(e) => {
              // Only when there are files: a plain text paste must keep
              // working.
              const pasted = filesFrom(e.clipboardData);
              if (pasted.length) {
                e.preventDefault();
                add(pasted);
              }
            }}
            onKeyDown={(e) => {
              // Enter sends, as in every chat; Shift+Enter is a new line.
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                send(draft);
              }
            }}
            rows={1}
            placeholder={
              dictation.recording
                ? "Listening…"
                : transcribing
                  ? "Writing it down…"
                  : "Message Niles"
            }
            aria-label="Message Niles"
            className="max-h-40 min-h-10 resize-none"
          />
          {/* The microphone where Send would be while there is nothing
              to send, the way a phone's messages app does it. A file
              counts as something to send. */}
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
          ) : canDictate && (transcribing || (!draft.trim() && files.length === 0)) ? (
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
              disabled={(!draft.trim() && files.length === 0) || busy}
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
        {fileError && <p className="text-destructive px-1 pb-1 text-sm">{fileError}</p>}
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
        // A size up in a hand, where the chat is read at arm's length
        // and the field below is already 16px (smaller, iOS zooms in).
        "max-w-[85%] rounded-2xl px-3.5 py-2 text-base sm:text-sm",
        // What was typed is shown as typed, line breaks and all; a
        // reply's lines are its markdown's business.
        from === "you"
          ? "bg-primary text-primary-foreground self-end rounded-br-md whitespace-pre-wrap"
          : "bg-card self-start rounded-bl-md",
      )}
    >
      {children}
    </div>
  );
}

/** The files on one turn, above the message: a thumbnail per picture,
    a chip opening a PDF in its own tab. Never an iframe or an embed:
    a PDF is only ever opened on its own, where the sandbox policy
    cannot stop it — but that page is never Niles's. */
function Attachments({ conversation, items }: { conversation: number; items: Attachment[] }) {
  return (
    <div className="flex flex-wrap justify-end gap-1.5 self-end">
      {items.map((a) =>
        a.kind === "image" ? (
          <img
            key={a.id}
            src={chatAttachmentUrl(conversation, a.id)}
            alt={a.name}
            width={a.width}
            height={a.height}
            loading="lazy"
            className="max-h-48 rounded-xl"
          />
        ) : (
          <a
            key={a.id}
            href={chatAttachmentUrl(conversation, a.id)}
            target="_blank"
            rel="noopener noreferrer"
            className="bg-card flex items-center gap-1.5 rounded-full px-3 py-1.5 text-sm"
          >
            <FileText className="size-4" aria-hidden />
            <span className="max-w-32 truncate">{a.name}</span>
          </a>
        ),
      )}
    </div>
  );
}

/** The chips of the message on its way: the files just sent, by their
    own object URLs, so a just-sent photo does not vanish between the
    bubble going up and the refetch landing. */
function PendingAttachments({ files }: { files: File[] }) {
  const previews = useMemo(() => {
    if (typeof URL.createObjectURL !== "function") return [];
    return files.filter(isImage).map((f) => [f, URL.createObjectURL(f)] as const);
  }, [files]);
  useEffect(
    () => () => previews.forEach(([, url]) => URL.revokeObjectURL(url)),
    [previews],
  );
  return <AttachmentChips files={files} previews={previews} />;
}

/** A row of the chosen files: a thumbnail when there is one, the name
    (truncated), its size, and a way to take it off the message. */
function AttachmentChips({
  files,
  previews,
  onRemove,
}: {
  files: File[];
  previews: readonly (readonly [File, string])[];
  onRemove?: (file: File) => void;
}) {
  return (
    <div
      className="flex flex-wrap justify-end gap-1.5 px-4 sm:px-6"
      data-testid="attachment-chips"
    >
      {files.map((file) => {
        const url = previews.find(([f]) => f === file)?.[1];
        return (
          <span
            key={`${file.name}-${file.size}-${file.lastModified}`}
            className="bg-card inline-flex items-center gap-1.5 rounded-full py-1 pr-1 pl-2 text-sm"
          >
            {url ? (
              <img src={url} alt="" className="size-10 rounded-md object-cover" />
            ) : (
              <FileText className="text-muted-foreground size-5" aria-hidden />
            )}
            <span className="max-w-32 truncate">{file.name}</span>
            <span className="text-muted-foreground shrink-0">{readable(file.size)}</span>
            {onRemove && (
              <button
                type="button"
                aria-label={`Remove ${file.name}`}
                onClick={() => onRemove(file)}
                className="rounded-full p-1 transition-colors hover:bg-muted"
              >
                <X className="size-4" aria-hidden />
              </button>
            )}
          </span>
        );
      })}
    </div>
  );
}
