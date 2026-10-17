import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { api } from "@/lib/api";
import type { ArtifactSharee, ArtifactShares } from "@/lib/api";
import { ARTIFACT_LIST, artifactKey } from "@/lib/artifacts";
import { CHOICE_LABELS, choicesFor, levelOf, personLabel } from "@/lib/sharing";
import type { ShareChoice } from "@/lib/sharing";
import { cn } from "@/lib/utils";

export interface ShareDialogProps {
  id: number;
  title: string;
  open: boolean;
  onClose: () => void;
}

/** None, View or Edit for one person, as a row of buttons: a choice the
    test can press, unlike a select's popup. */
function Choices({
  sharee,
  busy,
  onChoose,
}: {
  sharee: ArtifactSharee;
  busy: boolean;
  onChoose: (choice: ShareChoice) => void;
}) {
  const current: ShareChoice = sharee.level ?? "none";
  return (
    <div
      role="radiogroup"
      aria-label={personLabel(sharee)}
      className="bg-muted flex rounded-lg p-0.5 text-xs"
    >
      {choicesFor(sharee).map((choice) => (
        <button
          key={choice}
          type="button"
          role="radio"
          aria-checked={current === choice}
          disabled={busy}
          onClick={() => current !== choice && onChoose(choice)}
          className={cn(
            "rounded-md px-2 py-0.5",
            current === choice ? "bg-background text-foreground shadow-sm" : "text-muted-foreground",
          )}
        >
          {CHOICE_LABELS[choice]}
        </button>
      ))}
    </div>
  );
}

/**
 * Who else in the household has an artifact, and how: each person with
 * None, View or Edit. Its owner's alone; a change is saved at once.
 */
export function ShareDialog({ id, title, open, onClose }: ShareDialogProps) {
  const queryClient = useQueryClient();
  const key = [...artifactKey(id), "shares"];
  const shares = useQuery({ queryKey: key, queryFn: () => api.artifactShares(id), enabled: open });
  const share = useMutation({
    mutationFn: ({ person, choice }: { person: string; choice: ShareChoice }) =>
      api.shareArtifact(id, person, levelOf(choice)),
    onSuccess: (next: ArtifactShares) => {
      queryClient.setQueryData(key, next);
      // The list and the artifact say how many have it; the shares
      // themselves are the answer already.
      void queryClient.invalidateQueries({ queryKey: ARTIFACT_LIST });
      void queryClient.invalidateQueries({ queryKey: artifactKey(id), exact: true });
    },
  });

  return (
    <Dialog
      open={open}
      onOpenChange={(next: boolean) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="gap-0 p-0">
        <div className="border-border border-b px-4 py-3">
          <DialogTitle>Share “{title}”</DialogTitle>
          <DialogDescription>
            View lets them read it. Edit lets them change it too, and ask their own assistant to.
          </DialogDescription>
        </div>
        <DialogBody className="flex flex-col gap-3">
          {shares.isLoading ? (
            <Skeleton className="h-8 w-full" />
          ) : shares.data && shares.data.people.length > 0 ? (
            <ul aria-label="People" className="flex flex-col gap-2">
              {shares.data.people.map((s) => (
                <li key={s.person} className="flex items-center justify-between gap-3 text-sm">
                  <span className={cn("truncate", s.name === null && "text-muted-foreground")}>
                    {personLabel(s)}
                  </span>
                  <Choices
                    sharee={s}
                    busy={share.isPending}
                    onChoose={(choice) => share.mutate({ person: s.person, choice })}
                  />
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-muted-foreground text-sm">
              {shares.error?.message ?? "There is nobody else in the household to share it with."}
            </p>
          )}
          {share.error && <p className="text-destructive text-sm">{share.error.message}</p>}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
