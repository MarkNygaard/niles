import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { Memory, MemoryTarget } from "@/lib/api";
import { relativeTime } from "@/lib/skills";
import { cn } from "@/lib/utils";

export const MEMORY = ["memory"];

interface Forgetting {
  target: MemoryTarget;
  text: string;
}

const GROUPS: { target: MemoryTarget; title: string; empty: string }[] = [
  {
    target: "user",
    title: "About the household",
    empty: "Nothing yet. Niles keeps what you tell it about yourselves here.",
  },
  {
    target: "agent",
    title: "What Niles has noted",
    empty: "Nothing yet. Niles notes here what it learns about how the house works.",
  },
];

/**
 * What Niles carries from one conversation to the next.
 *
 * Mostly written by Claude's review once the house is quiet, which
 * nobody watches, and kept in order by its weekly tidy-up. This is where
 * they are seen, thrown out and put back; there is nothing to write
 * here — telling Niles is how something gets in.
 */
export function MemoryCard() {
  const memory = useQuery({ queryKey: MEMORY, queryFn: api.memory });
  const [forgetting, setForgetting] = useState<Forgetting>();

  return (
    <Card>
      <CardHeader>
        <CardTitle>Memory</CardTitle>
        <CardDescription>
          Niles keeps these between conversations and reads them before it
          answers. Delete anything that is wrong or out of date.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {memory.isPending ? (
          <>
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-16 w-full" />
          </>
        ) : memory.isError ? (
          <p className="text-destructive text-sm">{memory.error.message}</p>
        ) : (
          GROUPS.map((group) => (
            <section key={group.target} className="flex flex-col gap-2">
              <h3 className="text-muted-foreground px-1 text-xs font-medium tracking-wide uppercase">
                {group.title}
              </h3>
              {memory.data[group.target].length === 0 ? (
                <p className="text-muted-foreground rounded-lg border border-dashed px-3 py-3 text-sm">
                  {group.empty}
                </p>
              ) : (
                <ul aria-label={group.title} className="flex flex-col gap-2">
                  {memory.data[group.target].map((text) => (
                    <li
                      key={text}
                      className="flex items-start gap-2 rounded-lg border py-2 pr-1.5 pl-3"
                    >
                      {/* As written: an entry can run to several lines. */}
                      <p className="min-w-0 flex-1 py-1 text-sm break-words whitespace-pre-wrap">
                        {text}
                      </p>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label="Delete"
                        className="text-muted-foreground hover:text-destructive shrink-0"
                        onClick={() => setForgetting({ target: group.target, text })}
                      >
                        <Trash2 />
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          ))
        )}
        {memory.data && memory.data.forgotten.length > 0 && (
          <Forgotten forgotten={memory.data.forgotten} />
        )}
      </CardContent>
      <ConfirmForget entry={forgetting} onClose={() => setForgetting(undefined)} />
    </Card>
  );
}

/** What was taken out lately — most of it by Niles's weekly tidy-up —
    folded away, each with a way back. */
function Forgotten({ forgotten }: { forgotten: Memory["forgotten"] }) {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const restore = useMutation({
    mutationFn: ({ target, text }: Forgetting) => api.restoreMemory(target, text),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: MEMORY }),
  });
  return (
    <section className="flex flex-col gap-2">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="text-muted-foreground hover:text-foreground focus-visible:ring-ring/50 flex items-center gap-1 self-start rounded px-1 text-xs font-medium tracking-wide uppercase focus-visible:ring-3 focus-visible:outline-none"
      >
        <ChevronRight
          aria-hidden
          className={cn("size-4 transition-transform", open && "rotate-90")}
        />
        Recently forgotten ({forgotten.length})
      </button>
      {open && (
        <ul aria-label="Recently forgotten" className="flex flex-col gap-2">
          {forgotten.map((f) => (
            <li
              key={f.at + f.text}
              className="flex items-start gap-2 rounded-lg border border-dashed py-2 pr-1.5 pl-3"
            >
              <div className="min-w-0 flex-1 py-1">
                <p className="text-muted-foreground text-sm break-words whitespace-pre-wrap">
                  {f.text}
                </p>
                <p className="text-muted-foreground mt-0.5 text-xs">{relativeTime(f.at)}</p>
              </div>
              <Button
                variant="outline"
                size="sm"
                className="shrink-0"
                disabled={restore.isPending}
                onClick={() => restore.mutate({ target: f.target, text: f.text })}
              >
                Restore
              </Button>
            </li>
          ))}
        </ul>
      )}
      {restore.error && <p className="text-destructive text-sm">{restore.error.message}</p>}
    </section>
  );
}

function ConfirmForget({ entry, onClose }: { entry?: Forgetting; onClose: () => void }) {
  const queryClient = useQueryClient();
  const forget = useMutation({
    mutationFn: ({ target, text }: Forgetting) => api.forgetMemory(target, text),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: MEMORY });
      onClose();
    },
  });
  const close = () => {
    // Otherwise the last refusal is still showing on the next one.
    forget.reset();
    onClose();
  };
  return (
    <Dialog
      open={entry !== undefined}
      onOpenChange={(open: boolean) => {
        if (!open) close();
      }}
    >
      <DialogContent className="gap-0 p-0">
        {entry && (
          <>
            <div className="border-border border-b px-4 py-3">
              <DialogTitle>Delete this memory?</DialogTitle>
              <DialogDescription className="break-words whitespace-pre-wrap">
                {entry.text}
              </DialogDescription>
            </div>
            <DialogBody className="flex flex-col gap-3">
              {forget.error && <p className="text-destructive text-sm">{forget.error.message}</p>}
              <div className="flex justify-end gap-2">
                <Button variant="outline" onClick={close}>
                  Keep it
                </Button>
                <Button
                  variant="destructive"
                  disabled={forget.isPending}
                  onClick={() => forget.mutate(entry)}
                >
                  Delete
                </Button>
              </div>
            </DialogBody>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
