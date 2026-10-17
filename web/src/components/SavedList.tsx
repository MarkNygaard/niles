import { useState } from "react";
import {
  Archive,
  ArchiveRestore,
  ChevronDown,
  ChevronRight,
  Ellipsis,
  LoaderCircle,
  Pencil,
  Pin,
  PinOff,
  Trash2,
  TriangleAlert,
  Users,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import type { Artifact, ArtifactPatch, AssistantId } from "@/lib/api";
import { arrange, kindOf } from "@/lib/artifacts";
import { canEdit, changedNote, isOwner, sharingNote } from "@/lib/sharing";
import { relativeTime } from "@/lib/skills";
import { cn } from "@/lib/utils";

export interface SavedListProps {
  /** Undefined while they are on their way. */
  artifacts?: Artifact[];
  /** Which assistant makes them; null when neither is on. */
  assistant?: AssistantId | null;
  error?: string;
  onOpen: (id: number) => void;
  onChange: (id: number, patch: ArtifactPatch) => Promise<unknown>;
  onDelete: (id: number) => Promise<unknown>;
}

/** Where the assistant is switched on. */
export const VOICE_AND_AI = "#/me/settings/voice";

/**
 * What Niles has made for the signed-in person, and what others in the
 * household shared with them: the Saved tab of its panel. Pinned first,
 * then the rest by when they changed, then the archived ones, folded
 * away. A shared one says whose it is, and offers only what they may
 * do: pin it in their own list, and rename it when shared to edit.
 */
export function SavedList({
  artifacts,
  assistant,
  error,
  onOpen,
  onChange,
  onDelete,
}: SavedListProps) {
  const [actionsFor, setActionsFor] = useState<number>();
  const [confirming, setConfirming] = useState<Artifact>();
  const [renaming, setRenaming] = useState<{ artifact: Artifact; title: string }>();
  const [showArchived, setShowArchived] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string>();

  const act = async (work: () => Promise<unknown>, after?: () => void) => {
    setBusy(true);
    setFailure(undefined);
    try {
      await work();
      after?.();
    } catch (e) {
      setFailure(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // In place of the list rather than over it, as a conversation's
  // delete is: a dialog on the panel would be a third layer.
  if (confirming) {
    return (
      <div className="flex flex-col gap-3 px-2 py-1">
        <div>
          <h4 className="font-heading text-base leading-snug font-medium">Delete this?</h4>
          <p className="text-muted-foreground text-sm break-words">
            {confirming.title}, and every version of it.
          </p>
        </div>
        {failure && <p className="text-destructive text-sm">{failure}</p>}
        <div className="flex justify-end gap-2">
          <Button
            variant="outline"
            onClick={() => {
              setConfirming(undefined);
              setFailure(undefined);
            }}
          >
            Keep it
          </Button>
          <Button
            variant="destructive"
            disabled={busy}
            onClick={() =>
              act(
                () => onDelete(confirming.id),
                () => setConfirming(undefined),
              )
            }
          >
            Delete
          </Button>
        </div>
      </div>
    );
  }

  if (renaming) {
    const title = renaming.title.trim();
    return (
      <form
        className="flex flex-col gap-3 px-2 py-1"
        onSubmit={(e) => {
          e.preventDefault();
          if (!title) return;
          void act(
            () => onChange(renaming.artifact.id, { title }),
            () => setRenaming(undefined),
          );
        }}
      >
        <h4 className="font-heading text-base leading-snug font-medium">Rename</h4>
        <Input
          aria-label="Title"
          value={renaming.title}
          autoFocus
          onChange={(e) => setRenaming({ ...renaming, title: e.target.value })}
        />
        {failure && <p className="text-destructive text-sm">{failure}</p>}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={() => setRenaming(undefined)}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy || !title}>
            Save
          </Button>
        </div>
      </form>
    );
  }

  const row = (a: Artifact) => {
    const kind = kindOf(a.kind);
    const open = actionsFor === a.id;
    const mine = isOwner(a);
    const shared = sharingNote(a);
    const changed = changedNote(a);
    return (
      <li key={a.id} className="flex flex-col">
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={() => onOpen(a.id)}
            className={cn(
              "flex min-w-0 flex-1 items-start gap-2.5 rounded-lg px-2 py-2 text-left transition-colors",
              "hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
            )}
          >
            <kind.icon aria-hidden className="text-muted-foreground mt-0.5 size-4 shrink-0" />
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1 text-sm">
                <span className="truncate">{a.title}</span>
                {a.pinned && (
                  <span role="img" aria-label="Pinned" className="inline-flex">
                    <Pin aria-hidden className="text-muted-foreground size-3" />
                  </span>
                )}
                {mine && shared && (
                  <span role="img" aria-label={shared} className="inline-flex">
                    <Users aria-hidden className="text-muted-foreground size-3" />
                  </span>
                )}
              </span>
              {a.summary && (
                <span className="text-muted-foreground block truncate text-xs">{a.summary}</span>
              )}
              {!mine && shared && (
                <span className="text-muted-foreground flex items-center gap-1 text-xs">
                  <Users aria-hidden className="size-3" />
                  {shared}
                </span>
              )}
              <span className="text-muted-foreground flex items-center gap-1 text-xs">
                {a.status === "working" ? (
                  <>
                    <LoaderCircle aria-hidden className="size-3 animate-spin" />
                    being written…
                  </>
                ) : a.status === "failed" ? (
                  <>
                    <TriangleAlert aria-hidden className="text-destructive size-3" />
                    didn't finish
                  </>
                ) : changed ? (
                  `${relativeTime(a.updated_at)} · ${changed}`
                ) : (
                  relativeTime(a.updated_at)
                )}
              </span>
            </span>
          </button>
          <Button
            variant="ghost"
            size="icon"
            aria-label={`More for ${a.title}`}
            aria-expanded={open}
            onClick={() => setActionsFor(open ? undefined : a.id)}
            className="text-muted-foreground shrink-0"
          >
            <Ellipsis />
          </Button>
        </div>
        {/* Under the row rather than in a menu over it: the panel is a
            layer already, and these are four buttons. */}
        {open && (
          <div className="flex flex-wrap gap-1.5 px-2 pb-2 pl-8">
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => act(() => onChange(a.id, { pinned: !a.pinned }))}
            >
              {a.pinned ? <PinOff /> : <Pin />}
              {a.pinned ? "Unpin" : "Pin"}
            </Button>
            {canEdit(a) && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setRenaming({ artifact: a, title: a.title });
                  setActionsFor(undefined);
                }}
              >
                <Pencil />
                Rename
              </Button>
            )}
            {mine && (
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() =>
                  act(
                    () => onChange(a.id, { archived: !a.archived }),
                    () => setActionsFor(undefined),
                  )
                }
              >
                {a.archived ? <ArchiveRestore /> : <Archive />}
                {a.archived ? "Unarchive" : "Archive"}
              </Button>
            )}
            {mine && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setConfirming(a);
                  setActionsFor(undefined);
                }}
              >
                <Trash2 />
                Delete
              </Button>
            )}
          </div>
        )}
      </li>
    );
  };

  const groups = artifacts ? arrange(artifacts) : undefined;
  const shown = groups ? [...groups.pinned, ...groups.recent] : [];

  return (
    <div className="flex flex-col gap-2">
      {assistant === null && (
        <p className="bg-muted/60 text-muted-foreground mx-2 rounded-lg px-3 py-2 text-sm">
          Claude or ChatGPT makes these: ask for a packing list or a travel plan, and it is kept
          here. Neither is on.{" "}
          <a href={VOICE_AND_AI} className="text-foreground underline underline-offset-2">
            Switch one on in Settings → Voice & AI
          </a>
        </p>
      )}
      {failure && <p className="text-destructive px-2 text-sm">{failure}</p>}
      {error ? (
        <p className="text-destructive px-2 py-1 text-sm">{error}</p>
      ) : !groups ? (
        <div className="flex flex-col gap-2 p-2">
          <Skeleton className="h-12 w-full" />
          <Skeleton className="h-12 w-full" />
        </div>
      ) : shown.length === 0 && groups.archived.length === 0 ? (
        assistant !== null && (
          <p className="text-muted-foreground px-2 py-1 text-sm">
            Nothing saved yet. Ask Niles for a packing list or a travel plan, and it is kept here.
          </p>
        )
      ) : (
        <>
          {shown.length > 0 && (
            <ul aria-label="Saved" className="flex flex-col">
              {shown.map(row)}
            </ul>
          )}
          {groups.archived.length > 0 && (
            <div className="flex flex-col">
              <button
                type="button"
                aria-expanded={showArchived}
                onClick={() => setShowArchived((s) => !s)}
                className="text-muted-foreground hover:text-foreground flex items-center gap-1 px-2 py-1.5 text-xs font-medium tracking-wide uppercase"
              >
                {showArchived ? (
                  <ChevronDown aria-hidden className="size-3.5" />
                ) : (
                  <ChevronRight aria-hidden className="size-3.5" />
                )}
                Archived ({groups.archived.length})
              </button>
              {showArchived && (
                <ul aria-label="Archived" className="flex flex-col">
                  {groups.archived.map(row)}
                </ul>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
