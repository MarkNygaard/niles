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
import { SettingRow } from "@/components/SettingRow";
import type { Setting } from "@/components/SettingRow";
import { ApiError, api, patchForAll, valueAt } from "@/lib/api";
import type { Applied, ConfigView, Revision } from "@/lib/api";
import { AlertTriangle, Undo2 } from "lucide-react";

interface Row {
  label: string;
  description: string;
  settings: Setting[];
  joiner?: string;
}

/**
 * The curve, one row per thing it does over a day.
 *
 * A row holds a pair where the pair is one idea: a ramp is a span, and
 * its start says nothing without its end.
 */
const CURVE_ROWS: Row[] = [
  {
    label: "Morning ramp",
    description: "Lights come up across this window.",
    joiner: "→",
    settings: [
      { path: "lighting.morning_start", kind: "text", caption: "starts" },
      { path: "lighting.morning_end", kind: "text", caption: "ends" },
    ],
  },
  {
    label: "Sunset ramp",
    description: "And wind back down across this one.",
    joiner: "→",
    settings: [
      { path: "lighting.sunset_start", kind: "text", caption: "starts" },
      { path: "lighting.sunset_end", kind: "text", caption: "ends" },
    ],
  },
  {
    label: "Brightness",
    description: "Held flat at these levels outside the two ramps.",
    settings: [
      {
        path: "lighting.night_floor_brightness",
        kind: "number",
        caption: "night %",
        width: "w-24",
      },
      {
        path: "lighting.daytime_brightness",
        kind: "number",
        caption: "day %",
        width: "w-24",
      },
    ],
  },
  {
    label: "Curve pause",
    description:
      "The curve freezes where it stood when the pause began, so a weekend keeps Friday's light.",
    joiner: "→",
    settings: [
      {
        path: "lighting.curve_pause_start",
        kind: "text",
        caption: "from",
        width: "w-32",
      },
      {
        path: "lighting.curve_pause_end",
        kind: "text",
        caption: "until",
        width: "w-32",
      },
    ],
  },
];

/** Lights that sit out the curve, and what they hold instead. */
const AMBIENT_ROWS: Row[] = [
  {
    label: "Ambient lights",
    description:
      "Voice, scenes and the switch still control these normally — only the curve leaves them alone.",
    settings: [
      {
        path: "ambient_lights.devices",
        kind: "list",
        caption: "room/device ids, comma-separated",
        width: "min-w-64 flex-1",
      },
    ],
  },
  {
    label: "Held at",
    description: "What they show instead. Leave unset to not touch them at all.",
    settings: [
      {
        path: "lighting.ambient_brightness",
        kind: "number",
        caption: "brightness %",
        width: "w-28",
      },
      {
        path: "lighting.ambient_kelvin",
        kind: "number",
        caption: "colour K",
        width: "w-28",
      },
    ],
  },
];

type Entry = { path: string; value: unknown };

export function App() {
  const queryClient = useQueryClient();
  const [lastApplied, setLastApplied] = useState<Applied | null>(null);
  const [rowError, setRowError] = useState<{ row: string; message: string } | null>(null);

  const config = useQuery({ queryKey: ["config"], queryFn: api.getConfig });
  const history = useQuery({ queryKey: ["history"], queryFn: api.history });

  function refresh() {
    queryClient.invalidateQueries({ queryKey: ["config"] });
    queryClient.invalidateQueries({ queryKey: ["history"] });
  }

  const save = useMutation({
    mutationFn: ({ entries }: { row: string; entries: Entry[] }) =>
      api.patchConfig(patchForAll(entries)),
    onMutate: ({ row }) => setRowError((e) => (e?.row === row ? null : e)),
    onSuccess: (applied) => {
      setLastApplied(applied);
      setRowError(null);
      refresh();
    },
    onError: (error, { row }) =>
      setRowError({
        row,
        message: error instanceof ApiError ? error.message : String(error),
      }),
  });

  const reset = useMutation({
    // A row can hold two overridden values. Dropped in order, so the
    // history reads the way it happened.
    mutationFn: async ({ paths }: { row: string; paths: string[] }) => {
      let last: Applied | null = null;
      for (const path of paths) last = await api.resetPath(path);
      return last!;
    },
    onSuccess: (applied) => {
      setLastApplied(applied);
      setRowError(null);
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

  function row({ label, description, settings, joiner }: Row) {
    const id = settings[0].path;
    // A section the config file never mentions has no entry here, so it
    // reads as boot-only. That errs towards telling someone to restart
    // when they needn't have, which beats the reverse.
    const sections = settings.map((s) => sectionMeta.get(s.path.split(".")[0]));
    return (
      <SettingRow
        key={id}
        label={label}
        description={description}
        settings={settings}
        joiner={joiner}
        values={Object.fromEntries(
          settings.map((s) => [s.path, valueAt(view.effective, s.path)]),
        )}
        overridden={settings
          .map((s) => s.path)
          .filter((path) => valueAt(view.overrides, path) !== undefined)}
        hot={sections.every((section) => section?.reload === "hot")}
        saving={save.isPending || reset.isPending}
        error={rowError?.row === id ? rowError.message : undefined}
        onSave={(entries) => save.mutate({ row: id, entries })}
        onReset={(paths) => reset.mutate({ row: id, paths })}
      />
    );
  }

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

        <TabsContent value="lighting" className="flex flex-col gap-4">
          <Card>
            <CardHeader>
              <CardTitle>Daily curve</CardTitle>
              <CardDescription>
                Brightness and colour temperature follow these over the day.
                Lights already on pick changes up on the next tick.
              </CardDescription>
            </CardHeader>
            <CardContent className="divide-border divide-y">
              {CURVE_ROWS.map(row)}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Ambient lights</CardTitle>
              <CardDescription>
                Lights that sit out the curve and the morning routine, holding
                one dim, warm setting instead.
              </CardDescription>
            </CardHeader>
            <CardContent className="divide-border divide-y">
              {AMBIENT_ROWS.map(row)}
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
    <main className="mx-auto flex w-full max-w-4xl flex-col gap-4 px-6 py-8">
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
