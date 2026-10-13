import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Pin, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import type { Skill, SkillChange, SkillDetail } from "@/lib/api";
import {
  PROVENANCE_LABEL,
  STATUS_LABEL,
  activityText,
  skillTitle,
  splitArchived,
} from "@/lib/skills";
import { cn } from "@/lib/utils";

export const SKILLS = ["skills"];

/**
 * What Niles has learned to do: the procedures it writes for itself
 * after a conversation, and the ones it was asked to keep.
 *
 * A page to look at rather than to write on. Niles writes them; here
 * they are kept (pinned), put away (archived) or thrown out.
 */
export function SkillsPage() {
  const list = useQuery({ queryKey: SKILLS, queryFn: api.skills });
  const [open, setOpen] = useState<string>();
  const [showArchived, setShowArchived] = useState(false);

  if (list.isPending) {
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }
  if (list.isError) {
    return <p className="text-destructive text-sm">{list.error.message}</p>;
  }

  const { current, archived } = splitArchived(list.data);
  return (
    <div className="flex flex-col gap-3">
      {list.data.length === 0 ? (
        <div className="bg-card text-muted-foreground rounded-xl border px-4 py-6 text-sm">
          <p>
            When Niles works out how to do something for the household, it writes the steps down
            as a skill, and they show up here. You can ask it to, too:
          </p>
          <p className="text-foreground mt-2 italic">
            "Niles, remember how we wind the house down in the evening."
          </p>
        </div>
      ) : (
        <>
          <p className="text-muted-foreground text-sm">
            {current.length === 0
              ? "Every skill is archived."
              : "What Niles has learned to do, in its own words."}
          </p>
          {current.length > 0 && (
            <ul className="flex flex-col gap-2">
              {current.map((skill) => (
                <SkillRow key={skill.name} skill={skill} onOpen={() => setOpen(skill.name)} />
              ))}
            </ul>
          )}
        </>
      )}

      {archived.length > 0 && (
        <section className="mt-2 flex flex-col gap-2">
          {/* Put away, not gone: out of the way until somebody looks. */}
          <button
            type="button"
            aria-expanded={showArchived}
            onClick={() => setShowArchived(!showArchived)}
            className="text-muted-foreground hover:text-foreground focus-visible:ring-ring/50 flex items-center gap-1 self-start rounded px-1 text-xs font-medium tracking-wide uppercase focus-visible:ring-3 focus-visible:outline-none"
          >
            <ChevronRight
              aria-hidden
              className={cn("size-4 transition-transform", showArchived && "rotate-90")}
            />
            Archived ({archived.length})
          </button>
          {showArchived && (
            <ul aria-label="Archived skills" className="flex flex-col gap-2">
              {archived.map((skill) => (
                <SkillRow key={skill.name} skill={skill} onOpen={() => setOpen(skill.name)} />
              ))}
            </ul>
          )}
        </section>
      )}

      <SkillDialog name={open} onClose={() => setOpen(undefined)} />
    </div>
  );
}

function SkillRow({ skill, onOpen }: { skill: Skill; onOpen: () => void }) {
  const archived = skill.status === "archived";
  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        className="bg-card focus-visible:ring-ring/50 flex w-full items-center gap-3 rounded-xl border px-4 py-3 text-left focus-visible:ring-3 focus-visible:outline-none"
      >
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline gap-x-2">
            <span className={cn("text-sm font-medium", archived && "text-muted-foreground")}>
              {skillTitle(skill.name)}
            </span>
            <span className="text-muted-foreground font-mono text-xs break-all">{skill.name}</span>
          </div>
          {skill.description && (
            <p className="text-muted-foreground mt-0.5 line-clamp-2 text-sm">{skill.description}</p>
          )}
          <SkillBadges skill={skill} className="mt-1.5" />
          <p className="text-muted-foreground mt-1 text-xs">{activityText(skill)}</p>
        </div>
        <ChevronRight aria-hidden className="text-muted-foreground size-4 shrink-0" />
      </button>
    </li>
  );
}

function SkillBadges({ skill, className }: { skill: Skill; className?: string }) {
  return (
    <div className={cn("flex flex-wrap gap-1.5", className)}>
      <Badge
        variant={
          skill.status === "active" ? "success" : skill.status === "stale" ? "secondary" : "outline"
        }
      >
        {STATUS_LABEL[skill.status]}
      </Badge>
      <Badge variant="outline">{PROVENANCE_LABEL[skill.provenance]}</Badge>
      {skill.pinned && (
        <Badge>
          <Pin data-icon="inline-start" aria-hidden />
          Pinned
        </Badge>
      )}
    </div>
  );
}

/** One skill and its procedure: a sheet on a phone, a dialog on a
    desktop. */
function SkillDialog({ name, onClose }: { name?: string; onClose: () => void }) {
  return (
    <Dialog
      open={name !== undefined}
      onOpenChange={(open: boolean) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        initialFocus={(openType) => openType === "keyboard"}
        className="flex max-h-[90dvh] flex-col p-0 sm:w-[36rem]"
      >
        {name && <SkillSheet key={name} name={name} onClose={onClose} />}
      </DialogContent>
    </Dialog>
  );
}

function SkillSheet({ name, onClose }: { name: string; onClose: () => void }) {
  const queryClient = useQueryClient();
  const key = [...SKILLS, name];
  const detail = useQuery({ queryKey: key, queryFn: () => api.skill(name) });
  const [confirming, setConfirming] = useState(false);

  const update = useMutation({
    mutationFn: (change: SkillChange) => api.updateSkill(name, change),
    // The switch moves at once and the list catches up after.
    onMutate: async (change) => {
      await queryClient.cancelQueries({ queryKey: key });
      queryClient.setQueryData<SkillDetail>(key, (old) => old && { ...old, ...change });
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: SKILLS }),
  });
  const remove = useMutation({
    mutationFn: () => api.deleteSkill(name),
    onSuccess: () => {
      onClose();
      // Not asked for again: it would only answer 404.
      queryClient.removeQueries({ queryKey: key, exact: true });
      return queryClient.invalidateQueries({ queryKey: SKILLS });
    },
  });

  const skill = detail.data;
  return (
    <>
      <div className="border-border flex items-start gap-3 border-b px-4 py-3">
        <div className="min-w-0 flex-1">
          <DialogTitle>{skillTitle(name)}</DialogTitle>
          <p className="text-muted-foreground font-mono text-xs break-all">
            {name}
            {skill && ` · v${skill.version}`}
          </p>
        </div>
        <DialogClose
          aria-label="Close"
          className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex size-8 shrink-0 items-center justify-center rounded-lg focus-visible:outline-none"
        >
          <X aria-hidden className="size-4" />
        </DialogClose>
      </div>

      <DialogBody className="flex flex-col gap-3">
        {detail.isPending ? (
          <Skeleton className="h-40 w-full" />
        ) : detail.isError ? (
          <p className="text-destructive text-sm">{detail.error.message}</p>
        ) : (
          skill && (
            <>
              {skill.description && <DialogDescription>{skill.description}</DialogDescription>}
              <SkillBadges skill={skill} />
              <p className="text-muted-foreground text-xs">{activityText(skill)}</p>
              {/* Markdown, shown as written: the steps read fine as they
                  are, and a renderer is a dependency for one page. */}
              <pre className="bg-muted/50 rounded-lg p-3 font-mono text-xs leading-relaxed break-words whitespace-pre-wrap">
                {skill.body}
              </pre>
            </>
          )
        )}
      </DialogBody>

      {skill && (
        <div className="border-border flex flex-col gap-3 border-t px-4 py-3">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="text-sm font-medium">Pinned</div>
              <div className="text-muted-foreground text-xs">
                Kept from the curator, and from deletion.
              </div>
            </div>
            <Switch
              aria-label={`Pin ${skillTitle(name)}`}
              checked={skill.pinned}
              onCheckedChange={(pinned) => update.mutate({ pinned })}
            />
          </div>

          {confirming ? (
            <div className="flex flex-col gap-2">
              <p className="text-sm">Delete this skill? Niles forgets how to do this.</p>
              <div className="flex justify-end gap-2">
                <Button variant="outline" onClick={() => setConfirming(false)}>
                  Keep it
                </Button>
                <Button
                  variant="destructive"
                  disabled={remove.isPending}
                  onClick={() => remove.mutate()}
                >
                  Delete
                </Button>
              </div>
            </div>
          ) : (
            <div className="flex flex-col gap-1.5">
              <div className="flex flex-wrap justify-end gap-2">
                <Button
                  variant="outline"
                  disabled={update.isPending}
                  onClick={() =>
                    update.mutate({ status: skill.status === "archived" ? "active" : "archived" })
                  }
                >
                  {skill.status === "archived" ? "Restore" : "Archive"}
                </Button>
                <Button
                  variant="destructive"
                  disabled={skill.pinned}
                  onClick={() => setConfirming(true)}
                >
                  Delete
                </Button>
              </div>
              {skill.pinned && (
                <p className="text-muted-foreground text-right text-xs">
                  Unpin it to delete it.
                </p>
              )}
            </div>
          )}

          {update.error && <p className="text-destructive text-sm">{update.error.message}</p>}
          {remove.error && <p className="text-destructive text-sm">{remove.error.message}</p>}
        </div>
      )}
    </>
  );
}
