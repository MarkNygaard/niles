import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowLeft,
  Check,
  Copy,
  History,
  LoaderCircle,
  PanelTopClose,
  RotateCcw,
  TriangleAlert,
} from "lucide-react";
import { SandboxedPage } from "@/components/SandboxedPage";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { ArtifactImagesProvider } from "@/lib/artifactImages";
import type { ArtifactAuthor, ArtifactDetail, ArtifactPatch } from "@/lib/api";
import { ARTIFACTS, artifactKey, kindOf } from "@/lib/artifacts";
import { relativeTime } from "@/lib/skills";
import { localDay } from "@/lib/trainingPlan";
import { cn } from "@/lib/utils";

const AUTHORS: Record<ArtifactAuthor, string> = {
  person: "You",
  claude: "Claude",
  chatgpt: "ChatGPT",
  niles: "Niles",
};

type Look = "page" | "builtin";

/** The two ways an artifact with a page can be shown. */
function LookSwitch({ look, onChange }: { look: Look; onChange: (look: Look) => void }) {
  const option = (value: Look, label: string) => (
    <button
      type="button"
      aria-pressed={look === value}
      onClick={() => onChange(value)}
      className={cn(
        "rounded-md px-2 py-0.5",
        look === value ? "bg-background text-foreground shadow-sm" : "text-muted-foreground",
      )}
    >
      {label}
    </button>
  );
  return (
    <div role="group" aria-label="Show as" className="bg-muted flex self-end rounded-lg p-0.5 text-xs">
      {option("page", "Page")}
      {option("builtin", "Built-in")}
    </div>
  );
}

export interface ArtifactViewProps {
  id: number;
  /** Back to the list. */
  onBack: () => void;
}

/**
 * One artifact, open in Niles's panel: its kind's view, or its page in
 * a sealed frame when it has one, the versions it has been through,
 * whether its person is told, and a copy of it.
 */
export function ArtifactView({ id, onBack }: ArtifactViewProps) {
  const queryClient = useQueryClient();
  const detail = useQuery({ queryKey: artifactKey(id), queryFn: () => api.artifact(id) });
  const [versions, setVersions] = useState(false);
  const [reading, setReading] = useState<number>();
  const [copied, setCopied] = useState(false);
  const [look, setLook] = useState<Look>("page");
  // Why the page shown under this key was swapped for the built-in
  // view; a new page, or another version, is given its own chance.
  const [fellBack, setFellBack] = useState<{ key: string; why: string }>();

  const saved = (next: ArtifactDetail) => {
    queryClient.setQueryData(artifactKey(id), next);
    void queryClient.invalidateQueries({ queryKey: ARTIFACTS });
  };

  const patch = useMutation({
    mutationFn: (p: ArtifactPatch) => api.patchArtifact(id, p),
    onSuccess: saved,
  });
  const restore = useMutation({
    mutationFn: (n: number) => api.restoreArtifact(id, n),
    onSuccess: (next) => {
      saved(next);
      setReading(undefined);
      setVersions(false);
    },
  });
  const retry = useMutation({
    mutationFn: () => api.retryArtifact(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ARTIFACTS }),
  });
  const old = useQuery({
    queryKey: [...artifactKey(id), "revision", reading],
    queryFn: () => api.artifactRevision(id, reading!),
    enabled: reading !== undefined,
  });

  const back = (
    <button
      type="button"
      onClick={onBack}
      className="text-muted-foreground hover:text-foreground flex items-center gap-1 self-start rounded-md px-1 py-1 text-sm"
    >
      <ArrowLeft aria-hidden className="size-4" />
      Saved
    </button>
  );

  if (detail.isLoading) {
    return (
      <div className="flex flex-col gap-3 px-2">
        {back}
        <Skeleton className="h-6 w-2/3" />
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }
  if (!detail.data) {
    return (
      <div className="flex flex-col gap-3 px-2">
        {back}
        <p className="text-destructive text-sm">{detail.error?.message ?? "It is not there."}</p>
      </div>
    );
  }

  const a = detail.data;
  const kind = kindOf(a.kind);
  const failure = patch.error ?? restore.error ?? retry.error;
  const showing = reading !== undefined ? old.data?.content : a.content;
  const pageHtml = reading !== undefined ? old.data?.page_html : a.page_html;
  const pageKey = `${reading ?? "current"}:${reading !== undefined ? old.data?.page : a.page}`;
  const broken = fellBack?.key === pageKey ? fellBack.why : undefined;
  const onPage = Boolean(pageHtml) && look === "page" && broken === undefined;
  const edit =
    reading === undefined
      ? (content: unknown, reason: string) => patch.mutate({ content, reason })
      : undefined;

  return (
    <div className="flex flex-col gap-3 px-2 pb-2">
      {back}
      <div className="flex items-start gap-2">
        <kind.icon aria-hidden className="text-muted-foreground mt-1 size-4 shrink-0" />
        <div className="min-w-0">
          <h3 className="font-heading text-lg leading-snug font-medium break-words">{a.title}</h3>
          {a.summary && <p className="text-muted-foreground text-sm">{a.summary}</p>}
        </div>
      </div>

      {a.status === "working" && (
        <p className="text-muted-foreground flex items-center gap-1.5 text-sm" role="status">
          <LoaderCircle aria-hidden className="size-4 animate-spin" />
          {a.status_text ?? "Being written…"}
        </p>
      )}
      {a.status === "failed" && (
        <div className="bg-destructive/10 flex flex-col gap-2 rounded-lg px-3 py-2 text-sm">
          <p className="flex items-start gap-1.5">
            <TriangleAlert aria-hidden className="text-destructive mt-0.5 size-4 shrink-0" />
            <span>It didn't finish: {a.error ?? "nobody said why"}</span>
          </p>
          <Button
            size="sm"
            variant="outline"
            className="self-start"
            disabled={retry.isPending}
            onClick={() => retry.mutate()}
          >
            <RotateCcw />
            Try again
          </Button>
        </div>
      )}
      {failure && <p className="text-destructive text-sm">{failure.message}</p>}

      {reading !== undefined && (
        <div className="bg-muted/60 flex flex-wrap items-center gap-2 rounded-lg px-3 py-2 text-sm">
          <span className="flex-1">Version {reading}, as it was</span>
          {reading !== a.current_revision && (
            <Button size="sm" disabled={restore.isPending} onClick={() => restore.mutate(reading)}>
              Restore this version
            </Button>
          )}
          <Button size="sm" variant="outline" onClick={() => setReading(undefined)}>
            Back to the current one
          </Button>
        </div>
      )}

      {pageHtml && showing !== undefined && showing !== null && (
        <LookSwitch
          look={onPage ? "page" : "builtin"}
          onChange={(next) => {
            setLook(next);
            if (next === "page") setFellBack(undefined);
          }}
        />
      )}
      {broken !== undefined && look === "page" && (
        <p className="text-muted-foreground text-xs" role="status">
          The page could not be shown: {broken}. This is the built-in view.
        </p>
      )}

      {showing !== undefined && showing !== null ? (
        onPage && pageHtml ? (
          <SandboxedPage
            key={pageKey}
            html={pageHtml}
            content={showing}
            today={a.today ?? localDay()}
            images={a.images}
            loadImage={(image) => api.artifactImage(id, image)}
            title={a.title}
            onSave={a.status === "ready" ? edit : undefined}
            onFallback={(why) => setFellBack({ key: pageKey, why })}
          />
        ) : (
          <ArtifactImagesProvider artifact={a.id} images={a.images}>
            <kind.View content={showing} busy={patch.isPending} onChange={edit} />
          </ArtifactImagesProvider>
        )
      ) : (
        a.status !== "working" &&
        reading === undefined && (
          <p className="text-muted-foreground text-sm">Nothing has been written yet.</p>
        )
      )}

      <div className="border-border flex flex-col gap-3 border-t pt-3">
        <label className="flex items-center justify-between gap-3 text-sm">
          Tell me when it's ready
          <Switch
            checked={a.notify}
            disabled={patch.isPending}
            onCheckedChange={(notify) => patch.mutate({ notify })}
          />
        </label>
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="outline"
            aria-expanded={versions}
            onClick={() => setVersions((v) => !v)}
          >
            <History />
            Versions ({a.revisions.length})
          </Button>
          {a.content !== null && (
            <Button
              size="sm"
              variant="outline"
              onClick={async () => {
                await navigator.clipboard?.writeText(kind.text(a.content));
                setCopied(true);
              }}
            >
              {copied ? <Check /> : <Copy />}
              {copied ? "Copied" : "Copy"}
            </Button>
          )}
          {a.page_html && reading === undefined && (
            <Button
              size="sm"
              variant="outline"
              disabled={patch.isPending}
              onClick={() => patch.mutate({ remove_page: true })}
            >
              <PanelTopClose />
              Remove page
            </Button>
          )}
        </div>
        {versions && (
          <ol aria-label="Versions" className="flex flex-col">
            {a.revisions.map((r) => (
              <li key={r.n}>
                <button
                  type="button"
                  onClick={() => setReading(r.n === a.current_revision ? undefined : r.n)}
                  className={cn(
                    "hover:bg-muted w-full rounded-md px-2 py-1.5 text-left text-sm",
                    reading === r.n && "bg-muted",
                  )}
                >
                  <span className="block">
                    Version {r.n}
                    {r.n === a.current_revision && " (current)"} · {r.reason}
                  </span>
                  <span className="text-muted-foreground text-xs">
                    {AUTHORS[r.author] ?? r.author} · {relativeTime(r.at)}
                  </span>
                </button>
              </li>
            ))}
          </ol>
        )}
      </div>
    </div>
  );
}
