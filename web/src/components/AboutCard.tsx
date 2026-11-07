import { useEffect } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type BuildInfo,
  type RestartStatus,
  type UpdateStatus,
} from "@/lib/api";
import { REPO_URL, UPDATE, howToUpdate } from "@/lib/update";
import { relativeTime } from "@/lib/skills";
import { useUpdateStatus } from "@/hooks/useUpdateStatus";
import { useRestartStatus } from "@/hooks/useRestartStatus";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyField } from "@/components/CopyField";
import { RestartButton } from "@/components/RestartButton";

/** About Niles: which build runs here, and whether a newer one is
    published. Opening the page is asking, since the kept answer may be
    hours old; the background loop and `update_checked` keep it
    current after that. */
export function AboutCard() {
  const queryClient = useQueryClient();
  const update = useUpdateStatus();
  const restart = useRestartStatus();
  const check = useMutation({
    mutationFn: api.checkForUpdate,
    onSuccess: (status) => queryClient.setQueryData(UPDATE, status),
  });
  // Opening the page is asking: the kept answer may be hours old.
  const { mutate } = check;
  useEffect(() => mutate(), [mutate]);

  return (
    <Card>
      <CardHeader>
        <CardTitle>About Niles</CardTitle>
        <CardDescription>
          Which Niles runs here, and whether a newer one is published.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {update.isPending && !update.data ? (
          <AboutSkeleton />
        ) : update.isError && !update.data ? (
          <p role="status">Niles can't say whether a newer one is published here.</p>
        ) : update.data ? (
          <>
            <BuildRows build={update.data.running} />
            <StatusLine update={update.data} checking={check.isPending} />
            {update.data.failed && update.data.checked_at && (
              <p className="text-muted-foreground text-sm" role="status">
                The last check failed. {update.data.failed.reason}. This answer is from{" "}
                {relativeTime(update.data.checked_at)}.
              </p>
            )}
            {update.data.state !== "dev_build" && (
              <Button
                variant="outline"
                size="sm"
                className="self-start"
                disabled={check.isPending}
                onClick={() => check.mutate()}
              >
                {check.isPending ? "Checking…" : "Check again"}
              </Button>
            )}
            {update.data.state === "update_available" && (
              <WhatsNew update={update.data} />
            )}
            {update.data.state === "update_available" && restart.data && (
              <HowTo update={update.data} supervisor={restart.data.supervisor} restart={restart.data} />
            )}
          </>
        ) : null}
      </CardContent>
    </Card>
  );
}

function AboutSkeleton() {
  return (
    <div className="flex flex-col gap-2" aria-hidden>
      <Skeleton className="h-4 w-48" />
      <Skeleton className="h-4 w-64" />
      <Skeleton className="h-4 w-40" />
    </div>
  );
}

/** The version, the commit (linked to its page) and the build date. */
function BuildRows({ build }: { build: BuildInfo }) {
  return (
    <dl className="flex flex-col gap-1">
      <div className="flex gap-2">
        <dt className="text-muted-foreground w-16 shrink-0 text-sm">Version</dt>
        <dd className="text-sm">{build.version}</dd>
      </div>
      <div className="flex gap-2">
        <dt className="text-muted-foreground w-16 shrink-0 text-sm">Commit</dt>
        <dd className="text-sm">
          {build.commit ? (
            <a
              href={`${REPO_URL}/commit/${build.commit}`}
              target="_blank"
              rel="noreferrer"
              className="text-primary hover:underline"
            >
              {build.commit.slice(0, 7)}
            </a>
          ) : (
            "Development build"
          )}
        </dd>
      </div>
      {build.built_at && (
        <div className="flex gap-2">
          <dt className="text-muted-foreground w-16 shrink-0 text-sm">Built</dt>
          <dd className="text-sm">
            {new Date(build.built_at).toLocaleDateString(undefined, {
              dateStyle: "long",
            })}
          </dd>
        </div>
      )}
    </dl>
  );
}

/** The headline: Up to date, Update available, or why it can't say.
    An older answer is shown with its age and the failed check beside
    it. */
function StatusLine({ update, checking }: { update: UpdateStatus; checking: boolean }) {
  switch (update.state) {
    case "dev_build":
      return (
        <p role="status">This is a development build, so Niles doesn't check for a newer one.</p>
      );
    case "not_checked":
      if (checking) return <p role="status">Checking for a newer Niles…</p>;
      if (update.failed)
        return (
          <p role="status">Couldn't check for a newer Niles. {update.failed.reason}.</p>
        );
      return <p role="status">Not checked yet.</p>;
    case "up_to_date":
      return (
        <p role="status">
          Up to date. Checked {relativeTime(update.checked_at ?? "")}.
          {update.newer_than_published && " This build is newer than the newest published image."}
        </p>
      );
    case "update_available":
      return (
        <div className="flex flex-col gap-1">
          <p role="status" className="font-medium">
            Update available
          </p>
          {update.newest?.created && (
            <p>
              The newest image was built{" "}
              {new Date(update.newest.created).toLocaleDateString(undefined, {
                dateStyle: "long",
              })}
              .
              {update.newest.version && ` It is version ${update.newest.version}.`}
            </p>
          )}
        </div>
      );
  }
}

/** What changed between this build and the newest, newest first, each
    linked. A comparison GitHub wouldn't give names its failure and
    still links the compare page. */
function WhatsNew({ update }: { update: UpdateStatus }) {
  const commits = update.changes?.commits ?? [];
  return (
    <section className="flex flex-col gap-2" aria-label="What's new">
      <h3 className="text-sm font-medium">What's new</h3>
      {update.changes_failed ? (
        <>
          <p>Couldn't list what changed. {update.changes_failed}.</p>
          {update.compare_url && (
            <a
              href={update.compare_url}
              target="_blank"
              rel="noreferrer"
              className="text-primary hover:underline"
            >
              See the changes on GitHub
            </a>
          )}
        </>
      ) : (
        <>
          <ul className="flex list-disc flex-col gap-1 pl-5">
            {commits.slice(0, 20).map((change) => (
              <li key={change.sha}>
                <a
                  href={change.url}
                  target="_blank"
                  rel="noreferrer"
                  className="text-primary hover:underline"
                >
                  {change.subject}
                </a>
              </li>
            ))}
          </ul>
          {(commits.length > 20 || update.changes?.truncated) && update.compare_url && (
            <a
              href={update.compare_url}
              target="_blank"
              rel="noreferrer"
              className="text-primary hover:underline"
            >
              All changes on GitHub
            </a>
          )}
        </>
      )}
    </section>
  );
}

/** How to update where Niles runs: the steps for this install's
    supervisor, with the commands to copy. The restart step is the
    page's own button, when the install may. */
function HowTo({
  update,
  supervisor,
  restart,
}: {
  update: UpdateStatus;
  supervisor: string | null;
  restart: RestartStatus;
}) {
  return (
    <section className="flex flex-col gap-2" aria-label="How to update">
      <h3 className="text-sm font-medium">How to update</h3>
      {howToUpdate(update, supervisor).map((step, index) => (
        <div key={index} className="flex flex-col gap-2">
          <p>{step.text}</p>
          {step.restart && <RestartButton status={restart} />}
          {step.command && <CopyField label={step.label ?? "Command"} value={step.command} />}
        </div>
      ))}
    </section>
  );
}
