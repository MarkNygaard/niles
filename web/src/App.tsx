import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { SettingField } from "@/components/SettingField";
import { ApiError, api, patchFor, valueAt } from "@/lib/api";
import type { Applied, ConfigView, Revision } from "@/lib/api";
import { AlertTriangle, Undo2 } from "lucide-react";

/** The lighting fields, in the order they occur over a day. */
const LIGHTING_FIELDS: Array<{ key: string; label: string; hint?: string }> = [
  { key: "morning_start", label: "Morning ramp starts", hint: "HH:MM" },
  { key: "morning_end", label: "Morning ramp ends", hint: "HH:MM" },
  { key: "sunset_start", label: "Sunset ramp starts", hint: "HH:MM" },
  { key: "sunset_end", label: "Sunset ramp ends", hint: "HH:MM" },
  {
    key: "night_floor_brightness",
    label: "Night floor brightness",
    hint: "0–100%, held overnight",
  },
  {
    key: "daytime_brightness",
    label: "Daytime brightness",
    hint: "0–100%, held between the ramps",
  },
  {
    key: "curve_pause_start",
    label: "Curve pause starts",
    hint: "e.g. fri 12:00 — the curve freezes at this value until the pause ends",
  },
  { key: "curve_pause_end", label: "Curve pause ends", hint: "e.g. sun 12:00" },
];

export function App() {
  const queryClient = useQueryClient();
  const [lastApplied, setLastApplied] = useState<Applied | null>(null);
  const [fieldError, setFieldError] = useState<{ path: string; message: string } | null>(
    null,
  );

  const config = useQuery({ queryKey: ["config"], queryFn: api.getConfig });
  const history = useQuery({ queryKey: ["history"], queryFn: api.history });

  function refresh() {
    queryClient.invalidateQueries({ queryKey: ["config"] });
    queryClient.invalidateQueries({ queryKey: ["history"] });
  }

  const save = useMutation({
    mutationFn: ({ path, value }: { path: string; value: unknown }) =>
      api.patchConfig(patchFor(path, value)),
    onMutate: ({ path }) => setFieldError((e) => (e?.path === path ? null : e)),
    onSuccess: (applied) => {
      setLastApplied(applied);
      setFieldError(null);
      refresh();
    },
    onError: (error, { path }) =>
      setFieldError({
        path,
        message: error instanceof ApiError ? error.message : String(error),
      }),
  });

  const reset = useMutation({
    mutationFn: (path: string) => api.resetPath(path),
    onSuccess: (applied) => {
      setLastApplied(applied);
      refresh();
    },
  });

  const undo = useMutation({
    mutationFn: api.undo,
    onSuccess: (applied) => {
      setLastApplied(applied);
      refresh();
    },
    onError: () => setLastApplied(null),
  });

  if (config.isLoading) return <LoadingShell />;
  if (config.error) {
    return (
      <Shell>
        <Card>
          <CardHeader>
            <CardTitle>Can't reach Niles</CardTitle>
            <CardDescription>{String(config.error)}</CardDescription>
          </CardHeader>
        </Card>
      </Shell>
    );
  }

  const view = config.data as ConfigView;
  const sectionMeta = new Map(view.sections.map((s) => [s.name, s]));

  return (
    <Shell>
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">Niles configuration</h1>
          <p className="text-muted-foreground text-sm">
            Values Niles is running right now. Changes apply immediately.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => undo.mutate()}
          disabled={undo.isPending || (history.data?.length ?? 0) === 0}
        >
          <Undo2 /> Undo last change
        </Button>
      </header>

      {/* Without a writable volume every edit here survives only until
          the pod restarts. Better said up front than discovered. */}
      {!view.persistent && (
        <Notice>
          No writable volume is configured, so changes apply now but are lost
          when Niles restarts.
        </Notice>
      )}

      {lastApplied && (
        <Notice tone={lastApplied.noop ? "muted" : "default"}>
          {lastApplied.noop
            ? "That value was already set — nothing changed."
            : lastApplied.summary}
          {lastApplied.needs_restart.length > 0 && (
            <> · needs a restart to take effect ({lastApplied.needs_restart.join(", ")})</>
          )}
        </Notice>
      )}

      <Tabs defaultValue="lighting">
        <TabsList>
          <TabsTrigger value="lighting">Lighting</TabsTrigger>
          <TabsTrigger value="all">Everything else</TabsTrigger>
          <TabsTrigger value="history">History</TabsTrigger>
        </TabsList>

        <TabsContent value="lighting">
          <Card>
            <CardHeader>
              <CardTitle>Daily curve</CardTitle>
              <CardDescription>
                Brightness and colour temperature follow these over the day.
                Lights already on pick changes up on the next tick.
              </CardDescription>
            </CardHeader>
            <CardContent className="divide-border divide-y">
              {LIGHTING_FIELDS.map(({ key, label, hint }) => {
                const path = `lighting.${key}`;
                const value = valueAt(view.effective, path);
                // Optional settings that aren't configured have nothing
                // to show and no sensible empty state; hide them.
                if (value === undefined) return null;
                return (
                  <SettingField
                    key={path}
                    path={path}
                    label={label}
                    hint={hint}
                    value={value}
                    hot={sectionMeta.get("lighting")?.reload === "hot"}
                    overridden={valueAt(view.overrides, path) !== undefined}
                    saving={save.isPending || reset.isPending}
                    error={fieldError?.path === path ? fieldError.message : undefined}
                    onSave={(next) => save.mutate({ path, value: next })}
                    onReset={() => reset.mutate(path)}
                  />
                );
              })}
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="all">
          <Card>
            <CardHeader>
              <CardTitle>All sections</CardTitle>
              <CardDescription>
                Read-only. Sections marked “restart required” are only read
                when Niles starts, so editing them here would look like it
                worked without doing anything.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              {view.sections.map((section) => (
                <div key={section.name}>
                  <div className="mb-1 flex items-center gap-2">
                    <span className="font-mono text-sm">{section.name}</span>
                    {section.reload === "hot" ? (
                      <Badge variant="secondary">live</Badge>
                    ) : (
                      <Badge variant="outline">restart required</Badge>
                    )}
                    {section.overridden && <Badge>overridden</Badge>}
                  </div>
                  <pre className="bg-muted/40 overflow-x-auto rounded-md p-3 text-xs">
                    {JSON.stringify(view.effective[section.name], null, 2)}
                  </pre>
                </div>
              ))}
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="history">
          <Card>
            <CardHeader>
              <CardTitle>Change history</CardTitle>
              <CardDescription>
                Newest first. Changes made by voice appear here too.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <HistoryList revisions={history.data ?? []} />
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </Shell>
  );
}

function HistoryList({ revisions }: { revisions: Revision[] }) {
  if (revisions.length === 0) {
    return (
      <p className="text-muted-foreground text-sm">
        Nothing changed yet — everything is as the config file has it.
      </p>
    );
  }
  return (
    <ul className="space-y-2">
      {[...revisions].reverse().map((revision) => (
        <li key={revision.id} className="flex items-baseline gap-3 text-sm">
          <Badge variant={revision.source === "voice" ? "default" : "secondary"}>
            {revision.source}
          </Badge>
          <span className="font-mono">{revision.summary}</span>
          <span className="text-muted-foreground ml-auto text-xs whitespace-nowrap">
            {new Date(revision.at).toLocaleString()}
          </span>
        </li>
      ))}
    </ul>
  );
}

function Notice({
  children,
  tone = "default",
}: {
  children: React.ReactNode;
  tone?: "default" | "muted";
}) {
  return (
    <div
      className={
        tone === "muted"
          ? "text-muted-foreground bg-muted/40 flex items-center gap-2 rounded-md px-3 py-2 text-sm"
          : "bg-muted/60 flex items-center gap-2 rounded-md px-3 py-2 text-sm"
      }
    >
      <AlertTriangle className="size-4 shrink-0" />
      <span>{children}</span>
    </div>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <main className="mx-auto flex w-full max-w-3xl flex-col gap-4 px-4 py-8">
      {children}
    </main>
  );
}

function LoadingShell() {
  return (
    <Shell>
      <Skeleton className="h-8 w-64" />
      <Separator />
      <Skeleton className="h-64 w-full" />
    </Shell>
  );
}
