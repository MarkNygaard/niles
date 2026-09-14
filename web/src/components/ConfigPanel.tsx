import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Tabs, TabsContent } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { CurveChart } from "@/components/CurveChart";
import { AmbientControls } from "@/components/AmbientControls";
import { deviceOptions } from "@/components/DevicePicker";
import { PeopleCard } from "@/components/PeopleCard";
import { IntegrationsPage } from "@/components/IntegrationsPage";
import { RoleCard } from "@/components/RoleCard";
import { SecretsCard } from "@/components/SecretsCard";
import { SettingsNav } from "@/components/SettingsNav";
import { cn } from "@/lib/utils";
import { SetupBanner } from "@/components/SetupBanner";
import { WledCard } from "@/components/WledCard";
import { HomeCard } from "@/components/HomeCard";
import { MorningCard } from "@/components/MorningCard";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import type { Person } from "@/components/PeopleCard";
import { SettingRow } from "@/components/SettingRow";
import type { Setting } from "@/components/SettingRow";
import { ApiError, api, patchForAll, valueAt } from "@/lib/api";
import type { Applied, ConfigView, Revision, WledStrip } from "@/lib/api";
import { AlertTriangle, ChevronLeft, Undo2 } from "lucide-react";

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
      {
        path: "lighting.morning_start",
        kind: "text",
        caption: "starts",
        width: "w-32",
      },
      {
        path: "lighting.morning_end",
        kind: "text",
        caption: "ends",
        width: "w-32",
      },
    ],
  },
  {
    label: "Sunset ramp",
    description: "And wind back down across this one.",
    joiner: "→",
    settings: [
      {
        path: "lighting.sunset_start",
        kind: "text",
        caption: "starts",
        width: "w-32",
      },
      {
        path: "lighting.sunset_end",
        kind: "text",
        caption: "ends",
        width: "w-32",
      },
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
    label: "Fade",
    description:
      "How long a light takes to reach each new level. The curve only speaks once a minute, so this is what fills the gap between one instruction and the next. Nothing you do by hand waits for it.",
    settings: [
      {
        path: "lighting.transition_seconds",
        kind: "number",
        caption: "seconds",
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
        kind: "devices",
        caption: "pick from the lights Niles knows about",
        width: "min-w-72 flex-1",
      },
    ],
  },
];

type Entry = { path: string; value: unknown };

function numberAt(root: unknown, path: string): number | undefined {
  const value = valueAt(root, path);
  return typeof value === "number" ? value : undefined;
}

function stringAt(root: unknown, path: string): string | undefined {
  const value = valueAt(root, path);
  return typeof value === "string" ? value : undefined;
}

function peopleAt(root: unknown): Person[] {
  const value = valueAt(root, "auth.allowed");
  return Array.isArray(value) ? (value as Person[]) : [];
}

/** The routine as the config has it, which may be nothing at all. */
function routineAt(root: unknown): { enabled?: boolean; fire_days?: string[] } {
  const value = valueAt(root, "lighting.morning_routine");
  return value && typeof value === "object" ? (value as Record<string, never>) : {};
}

/** The host out of a base URL, for the line saying what answers today. */
function hostOf(baseUrl?: string): string | undefined {
  const rest = baseUrl?.split("://")[1];
  return rest ? rest.split(/[/?]/)[0] || undefined : undefined;
}

function stripsAt(root: unknown): WledStrip[] {
  const value = valueAt(root, "wled.devices");
  return Array.isArray(value) ? (value as WledStrip[]) : [];
}

export function ConfigPanel() {
  const queryClient = useQueryClient();
  const [lastApplied, setLastApplied] = useState<Applied | null>(null);
  const [rowError, setRowError] = useState<{ row: string; message: string } | null>(null);

  const config = useQuery({ queryKey: ["config"], queryFn: api.getConfig });
  const history = useQuery({ queryKey: ["history"], queryFn: api.history });
  const devices = useQuery({ queryKey: ["devices"], queryFn: api.devices });
  const tado = useQuery({ queryKey: ["tado"], queryFn: api.tadoStatus });
  const secrets = useQuery({ queryKey: ["secrets"], queryFn: api.secrets });
  const setup = useQuery({ queryKey: ["setup"], queryFn: api.setup });
  const integrations = useQuery({
    queryKey: ["integrations"],
    queryFn: api.integrations,
  });
  // Six hundred-odd strings that never change while the process runs.
  const timezones = useQuery({
    queryKey: ["timezones"],
    queryFn: api.timezones,
    staleTime: Infinity,
  });

  // On a phone the list *is* the screen and tapping pushes into a
  // section; from `sm` both sit side by side and something has to be
  // open, so nothing lands on an empty pane.
  const phone = useMediaQuery("(max-width: 639px)");
  const [section, setSection] = useState<string | null>(null);
  const showNav = !phone || section === null;
  const showSection = !phone || section !== null;

  /**
   * Everything the server works out from the config, re-asked.
   *
   * Not just `config`: several routes answer questions *about* it, and
   * each one goes stale the moment a value changes. Invalidating only
   * the two obvious ones is how the tado switch came to sit still while
   * presence turned on and off behind it — the card reads
   * `/presence/tado`, which nothing was re-asking.
   *
   * `devices` is deliberately absent. It comes from the registry rather
   * than the config, and the one config change that moves it —
   * a WLED strip's channels — needs a restart anyway.
   */
  function refresh() {
    for (const key of [
      "config",
      "history",
      "setup",
      "secrets",
      "integrations",
      "tado",
    ]) {
      queryClient.invalidateQueries({ queryKey: [key] });
    }
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

  if (config.isLoading) return <LoadingPanel />;
  if (config.error) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Can't reach Niles</CardTitle>
          <CardDescription>{String(config.error)}</CardDescription>
        </CardHeader>
      </Card>
    );
  }

  const view = config.data as ConfigView;
  const sectionMeta = new Map(view.sections.map((s) => [s.name, s]));
  // Absent means on: every config written before the switch existed
  // has a curve that runs, and the server defaults the same way.
  const providers =
    (view.effective.providers as import("@/lib/api").Provider[] | undefined) ?? [];
  /** A field of a role section, as the live config has it. */
  const roleValue = (section: "stt" | "llm", field: string) =>
    (view.effective[section] as Record<string, unknown> | undefined)?.[field] as
      | string
      | undefined;
  const curveOn =
    (view.effective.lighting as { enabled?: boolean } | undefined)?.enabled !==
    false;

  const lights = deviceOptions(devices.data ?? []);
  const noLights = devices.isLoading
    ? "Still asking Niles which lights it has…"
    : "Niles has no lights registered yet.";

  // Only offer a control something can act on: a house of RGB strips
  // has no use for a colour temperature, and offering one would invite
  // setting a value that goes nowhere.
  const chosen = new Set(
    (valueAt(view.effective, "ambient_lights.devices") as string[] | undefined) ?? [],
  );
  const ambient = lights.filter((light) => chosen.has(light.value));

  function row({ label, description, settings: declared, joiner }: Row) {
    // The pickable lights come from the registry, which the page loads
    // separately — so they're attached here rather than in the static
    // row definitions above.
    const settings = declared.map((setting) =>
      setting.kind === "devices"
        ? { ...setting, options: lights, optionsEmpty: noLights }
        : setting,
    );
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
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-muted-foreground text-sm">
          Values Niles is running right now. Changes apply immediately.
        </p>
        <Button
          variant="outline"
          onClick={() => undo.mutate()}
          disabled={undo.isPending || (history.data?.length ?? 0) === 0}
        >
          <Undo2 /> Undo last change
        </Button>
      </div>

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

      {setup.data && <SetupBanner report={setup.data} />}

      {/* Driven by value alone: the nav beside it is the trigger, and a
          tab bar as well would be two controls for one thing. */}
      <Tabs value={section ?? "lighting"} onValueChange={(v) => setSection(String(v))}>
        <div className="flex flex-col gap-5 sm:flex-row sm:gap-6">
          {showNav && (
            <SettingsNav current={phone ? null : section} onPick={setSection} />
          )}
          <div className={cn("min-w-0 flex-1", !showSection && "hidden")}>
            {phone && (
              <button
                type="button"
                onClick={() => setSection(null)}
                className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 -ml-1 mb-3 flex items-center gap-1 rounded-lg py-1 pr-2 text-sm focus-visible:outline-none"
              >
                <ChevronLeft aria-hidden className="size-4" />
                Settings
              </button>
            )}

        <TabsContent value="lighting" className="flex flex-col gap-4">
          <Card>
            <CardHeader>
              <CardTitle>Daily curve</CardTitle>
              <CardDescription>
                Brightness and colour temperature follow these over the day.
                Lights already on pick changes up on the next tick.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <label className="flex items-center justify-between gap-4">
                <span className="min-w-0">
                  <span className="block text-sm font-medium">
                    Follow the curve
                  </span>
                  <span className="text-muted-foreground block text-xs">
                    Off leaves lights exactly where you put them. Voice, the
                    dashboard and the wall dimmer all still work, and the
                    morning routine is separate.
                  </span>
                </span>
                <Switch
                  checked={curveOn}
                  disabled={save.isPending}
                  onCheckedChange={(on) =>
                    save.mutate({
                      row: "lighting.enabled",
                      entries: [{ path: "lighting.enabled", value: on }],
                    })
                  }
                />
              </label>

              {/* Hidden rather than disabled: a wall of greyed-out
                  settings is a page asking to be read and then ignored.
                  Nothing is lost by hiding them — the values stay, and
                  come back untouched when the curve does. */}
              {curveOn && (
                <div className="divide-border divide-y">
                  <CurveChart lighting={view.effective.lighting} />
                  {CURVE_ROWS.map(row)}
                </div>
              )}
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
              <div className="grid gap-x-8 gap-y-3 py-4 sm:grid-cols-[minmax(190px,240px)_minmax(0,1fr)]">
                <div className="flex flex-col gap-1">
                  <span className="text-sm font-medium">Held at</span>
                  <p className="text-muted-foreground text-xs">
                    What they show instead of the curve. Leave them unset and
                    ambient lights are not touched at all.
                  </p>
                </div>
                <AmbientControls
                  supportsRgb={ambient.some((light) => light.supportsRgb)}
                  supportsColorTemp={ambient.some((light) => light.supportsColorTemp)}
                  brightness={numberAt(view.effective, "lighting.ambient_brightness")}
                  color={stringAt(view.effective, "lighting.ambient_color")}
                  kelvin={numberAt(view.effective, "lighting.ambient_kelvin")}
                  disabled={save.isPending}
                  onChange={(path, value) => save.mutate({ row: path, entries: [{ path, value }] })}
                />
              </div>
            </CardContent>
          </Card>

          <MorningCard
            // Present and not switched off. A section written before the
            // switch existed has no `enabled`, and meant yes.
            enabled={
              routineAt(view.effective).enabled ??
              valueAt(view.effective, "lighting.morning_routine") !== undefined
            }
            fireDays={routineAt(view.effective).fire_days ?? []}
            start={stringAt(view.effective, "lighting.morning_start")}
            end={stringAt(view.effective, "lighting.morning_end")}
            saving={save.isPending}
            error={
              rowError?.row === "lighting.morning_routine"
                ? rowError.message
                : undefined
            }
            onToggle={(on) =>
              save.mutate({
                row: "lighting.morning_routine",
                entries: [
                  { path: "lighting.morning_routine.enabled", value: on },
                ],
              })
            }
            onDays={(days) =>
              save.mutate({
                row: "lighting.morning_routine",
                entries: [
                  { path: "lighting.morning_routine.fire_days", value: days },
                ],
              })
            }
            row={row}
          />

          <WledCard
            strips={stripsAt(view.effective)}
            saving={save.isPending}
            error={rowError?.row === "wled.devices" ? rowError.message : undefined}
            onChange={(next) =>
              save.mutate({
                row: "wled.devices",
                entries: [{ path: "wled.devices", value: next }],
              })
            }
          />
        </TabsContent>

        <TabsContent value="home">
          <HomeCard
            values={{
              name: stringAt(view.effective, "home.name"),
              latitude: numberAt(view.effective, "home.latitude"),
              longitude: numberAt(view.effective, "home.longitude"),
              timezone: stringAt(view.effective, "home.timezone"),
              units: stringAt(view.effective, "home.units"),
            }}
            timezones={timezones.data ?? []}
            saving={save.isPending || reset.isPending}
            error={rowError?.row === "home" ? rowError.message : undefined}
            onChange={(entries) => save.mutate({ row: "home", entries })}
            onClear={(path) => reset.mutate({ row: "home", paths: [path] })}
          />
        </TabsContent>

        <TabsContent value="people">
          <Card>
            <CardHeader>
              <CardTitle>People</CardTitle>
              <CardDescription>
                Who can sign in. Signing in uses GitHub, but a GitHub account
                is free — so this list, not GitHub, decides who gets in.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <PeopleCard
                people={peopleAt(view.effective)}
                saving={save.isPending}
                error={rowError?.row === "auth.allowed" ? rowError.message : undefined}
                onChange={(people) =>
                  save.mutate({
                    row: "auth.allowed",
                    entries: [{ path: "auth.allowed", value: people }],
                  })
                }
              />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="credentials" className="flex flex-col gap-4">
          {secrets.data && (
            <SecretsCard
              report={secrets.data}
              onChanged={() => secrets.refetch()}
            />
          )}
        </TabsContent>

        <TabsContent value="language" className="flex flex-col gap-4">
          <Card>
            <CardHeader>
              <CardTitle>Speech &amp; language</CardTitle>
              <CardDescription>
                Which provider does each job, and with which model. The two go
                together: a model name is not portable between providers.
              </CardDescription>
            </CardHeader>
            <CardContent className="divide-border flex flex-col gap-5 divide-y">
              <RoleCard
                role="stt"
                title="Speech-to-text"
                description="Turns what a satellite heard into words."
                providers={providers}
                current={roleValue("stt", "provider")}
                model={roleValue("stt", "model") ?? ""}
                fallbackHost={hostOf(roleValue("stt", "base_url"))}
                defaultModel={stringAt(view.defaults, "stt.model")}
                saving={save.isPending}
                onSave={(change) =>
                  save.mutate({
                    row: "stt",
                    entries: [
                      { path: "stt.provider", value: change.provider ?? null },
                      { path: "stt.model", value: change.model },
                    ],
                  })
                }
              />
              <div className="pt-5">
                <RoleCard
                  role="llm"
                  title="Language model"
                  description="Answers anything a pattern cannot."
                  providers={providers}
                  current={roleValue("llm", "provider")}
                  model={roleValue("llm", "model") ?? ""}
                  fallbackHost={hostOf(roleValue("llm", "base_url"))}
                  defaultModel={stringAt(view.defaults, "llm.model")}
                  saving={save.isPending}
                  onSave={(change) =>
                    save.mutate({
                      row: "llm",
                      entries: [
                        { path: "llm.provider", value: change.provider ?? null },
                        { path: "llm.model", value: change.model },
                      ],
                    })
                  }
                />
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="integrations">
          <IntegrationsPage
            integrations={integrations.data ?? []}
            secrets={secrets.data}
            tado={tado.data}
            linear={
              (view.effective.integrations as
                | { linear?: { team?: string; trigger_label?: string } }
                | undefined)?.linear
            }
            saving={save.isPending}
            error={rowError ? rowError.message : undefined}
            onChange={(row, entries) => save.mutate({ row, entries })}
            onSecretsChanged={() => secrets.refetch()}
            onTadoChanged={() => tado.refetch()}
          />
        </TabsContent>

        <TabsContent value="all">
          <Card>
            <CardHeader>
              <CardTitle>Raw config</CardTitle>
              <CardDescription>
                Every section as Niles has it, read-only — including the ones
                with no page of their own yet, which is why this exists.
                Sections marked “restart required” are only read at startup,
                so editing them here would look like it worked without doing
                anything.
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
                  {view.effective[section.name] === undefined ? (
                    <p className="text-muted-foreground text-xs">
                      Not configured — Niles uses its defaults.
                    </p>
                  ) : (
                    <pre className="bg-muted/40 overflow-x-auto rounded-md p-3 text-xs">
                      {JSON.stringify(view.effective[section.name], null, 2)}
                    </pre>
                  )}
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
          </div>
        </div>
      </Tabs>
    </div>
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

function LoadingPanel() {
  return (
    <div className="flex flex-col gap-4">
      <Skeleton className="h-8 w-64" />
      <Separator />
      <Skeleton className="h-64 w-full" />
    </div>
  );
}
