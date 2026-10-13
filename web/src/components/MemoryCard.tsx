import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
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
import type { MemoryTarget } from "@/lib/api";

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
 * Mostly written by a review after a voice turn that nobody watches, so
 * it can keep things it should not have. This is where they are seen
 * and thrown out; there is nothing to write here — telling Niles is how
 * something gets in.
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
      </CardContent>
      <ConfirmForget entry={forgetting} onClose={() => setForgetting(undefined)} />
    </Card>
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
