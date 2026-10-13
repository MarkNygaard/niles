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
import { AnchorEditor } from "@/components/AnchorEditor";
import type { Anchor } from "@/components/AnchorEditor";
import { AmbientControls } from "@/components/AmbientControls";
import { deviceOptions } from "@/components/DevicePicker";
import { PeopleCard } from "@/components/PeopleCard";
import { VoicesCard } from "@/components/VoicesCard";
import { CaptureCard } from "@/components/CaptureCard";
import { IntegrationsPage } from "@/components/IntegrationsPage";
import { RoleCard } from "@/components/RoleCard";
import { SecretField } from "@/components/SecretField";
import { SECTIONS, SettingsNav } from "@/components/SettingsNav";
import { cn } from "@/lib/utils";
import { SetupBanner } from "@/components/SetupBanner";
import { WledCard } from "@/components/WledCard";
import { HomeCard } from "@/components/HomeCard";
import { MorningCard } from "@/components/MorningCard";
import { MemoryCard } from "@/components/MemoryCard";
import { MenuCard } from "@/components/MenuCard";
import { RoomOrderCard } from "@/components/RoomOrderCard";
import { SatellitesCard } from "@/components/SatellitesCard";
import { SpeakersCard } from "@/components/SpeakersCard";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import type { Person } from "@/components/PeopleCard";
import { SettingRow } from "@/components/SettingRow";
import type { Setting } from "@/components/SettingRow";
import { ApiError, api, patchForAll, roomOrder, valueAt } from "@/lib/api";
import { mediaShownOf, menuLayout } from "@/lib/menu";
import { roomsOf } from "@/lib/rooms";
import type { Applied, ConfigView, Revision, WledStrip } from "@/lib/api";
import type { Satellite } from "@/components/SatellitesCard";
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

/** Plugs with a lamp on them, which then count as lights. */
const LAMP_PLUG_ROWS: Row[] = [
  {
    label: "Lamps on plugs",
    description:
      "Plugs picked here are lamps: “the lights”, room switches and scenes include them. Any other plug is only switched when asked for by name.",
    settings: [
      {
        path: "lamp_plugs.devices",
        kind: "devices",
        of: "outlet",
        caption: "pick from the plugs Niles knows about",
        width: "min-w-72 flex-1",
      },
    ],
  },
];

/**
 * The broker, which everything on Zigbee and WLED is reached through.
 * Its login is on the same page, as secrets: typed in, never shown.
 */
const MQTT_ROWS: Row[] = [
  {
    label: "Broker",
    description:
      "Where Zigbee2MQTT and the WLED strips publish. Leave the host empty and Niles has no lights at all.",
    joiner: ":",
    settings: [
      { path: "mqtt.host", kind: "text", caption: "host", width: "min-w-48 flex-1" },
      { path: "mqtt.port", kind: "number", caption: "port", width: "w-24" },
    ],
  },
  {
    label: "Zigbee2MQTT prefix",
    description:
      "The topic Zigbee2MQTT publishes under, its base_topic. One word, no slashes.",
    settings: [
      { path: "mqtt.z2m_prefix", kind: "text", caption: "topic", width: "w-48" },
    ],
  },
  {
    label: "Client id",
    description:
      "What Niles calls itself to the broker. Two clients with one id knock each other off.",
    settings: [
      { path: "mqtt.client_id", kind: "text", caption: "client id", width: "w-48" },
    ],
  },
];

/** Piper, which speaks the replies. */
const TTS_ROWS: Row[] = [
  {
    label: "Piper",
    description: "The Piper server Niles sends a reply to, to have it spoken.",
    settings: [
      { path: "tts.base_url", kind: "text", caption: "address", width: "min-w-72 flex-1" },
    ],
  },
  {
    label: "Voice",
    description: "Which of Piper's voice models speaks.",
    settings: [
      { path: "tts.default_voice", kind: "text", caption: "voice model", width: "w-56" },
    ],
  },
];

const LANGUAGE_ROW: Row = {
  label: "Language",
  description:
    "What the house speaks, as a two-letter code. Empty lets the transcriber work it out each time, which costs a little accuracy on short commands.",
  settings: [
    {
      path: "stt.language",
      kind: "text",
      caption: "ISO 639-1, like en",
      width: "w-28",
      optional: true,
      placeholder: "auto-detect",
    },
  ],
};

type Entry = { path: string; value: unknown };

/** The anchors at `lighting.color_temp_anchors`, or `null` for none. */
function anchorsAt(root: unknown): Anchor[] | null {
  const value = valueAt(root, "lighting.color_temp_anchors");
  if (!Array.isArray(value)) return null;
  return value.filter(
    (a): a is Anchor =>
      Boolean(a) &&
      typeof a === "object" &&
      typeof a.time === "string" &&
      typeof a.kelvin === "number",
  );
}

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

/** The satellites as `[satellites.<name>]` has them, sorted by name. */
/** A list of strings at a dotted path, with anything else read as none. */
function stringsAt(root: unknown, path: string): string[] {
  const value = valueAt(root, path);
  return Array.isArray(value) ? value.filter((v) => typeof v === "string") : [];
}

/**
 * The satellites, as the live config has them.
 *
 * Exported for the test that pins the round trip: a field this does not
 * read is a field the card silently shows its default for, which is how
 * the volume slider appeared to save and then came back at 100.
 */
export function satellitesAt(root: unknown): Satellite[] {
  const value = valueAt(root, "satellites");
  if (!value || typeof value !== "object") return [];
  return Object.entries(
    value as Record<string, { ip?: string; room?: string; volume?: number }>,
  )
    .map(([name, entry]) => ({
      name,
      ip: entry?.ip ?? "",
      room: entry?.room ?? "",
      // Left undefined rather than defaulted to 100 here: the card
      // shows the shipped default for an entry that has no volume, and
      // deciding that twice is how the two come to disagree.
      volume: entry?.volume,
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** Where each Sonos is placed, by its id, as `[speakers.sonos]` has it. */
export function sonosAt(root: unknown): Record<string, { room: string; name: string }> {
  const value = valueAt(root, "speakers.sonos");
  if (!value || typeof value !== "object") return {};
  return Object.fromEntries(
    Object.entries(value as Record<string, { room?: string; name?: string }>)
      .filter(([, entry]) => typeof entry?.room === "string")
      .map(([id, entry]) => [id, { room: entry.room ?? "", name: entry.name ?? id }]),
  );
}

/** The Sonos playing in each room, by name. */
export function musicByRoom(root: unknown): Record<string, string[]> {
  const music: Record<string, string[]> = {};
  for (const { room, name } of Object.values(sonosAt(root))) {
    (music[room] ??= []).push(name);
  }
  for (const names of Object.values(music)) names.sort();
  return music;
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
  /** The keys a page sets up, each where it is used: there is no list
      of every key, so a key is never shown in two places. */
  const secretFields = (keys: string[]) =>
    keys.map((key) => {
      const secret = secrets.data?.secrets.find((s) => s.key === key);
      return (
        secret && (
          <SecretField
            key={key}
            secret={secret}
            writable={secrets.data?.writable ?? false}
            onChanged={() => secrets.refetch()}
          />
        )
      );
    });
  const setup = useQuery({ queryKey: ["setup"], queryFn: api.setup });
  // Keyed by the address so a new one is asked at once. Not fetched
  // without one: there is nobody to ask.
  const sonosHost = stringAt(config.data?.effective, "speakers.host")?.trim() ?? "";
  const tvHost = stringAt(config.data?.effective, "tv.host")?.trim() ?? "";
  const tv = useQuery({
    queryKey: ["tv", tvHost],
    queryFn: api.tv,
    enabled: tvHost !== "",
  });
  const speakers = useQuery({
    queryKey: ["speakers", sonosHost],
    queryFn: api.speakers,
    enabled: sonosHost !== "",
    staleTime: 30_000,
  });
  const integrations = useQuery({
    queryKey: ["integrations"],
    queryFn: api.integrations,
  });
  // Answers 501 when recognition is off, which is a fine answer and not
  // worth retrying at: an empty list then stands for "nobody", and the
  // card says separately that recognition is not running.
  const voices = useQuery({
    queryKey: ["voices"],
    queryFn: api.voices,
    retry: false,
  });
  const forget = useMutation({
    mutationFn: api.forgetVoice,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["voices"] }),
  });
  const captures = useQuery({
    queryKey: ["captures"],
    queryFn: api.captures,
    retry: false,
  });
  const clearCaptures = useMutation({
    mutationFn: api.clearCaptures,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["captures"] }),
  });
  const rename = useMutation({
    mutationFn: ({ speaker, name }: { speaker: string; name: string }) =>
      api.renameVoice(speaker, name),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["voices"] }),
  });
  // Every call reaches tado, so this is asked once rather than on a
  // timer: it is a setup list, not a readout.
  const climate = useQuery({
    queryKey: ["climate"],
    queryFn: api.climate,
    staleTime: 60_000,
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
  // The anchor list as it is being edited, drawn on the chart before it
  // is saved: a colour point is judged by the curve it makes.
  const [anchorPreview, setAnchorPreview] = useState<Anchor[] | null>(null);
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
      "climate",
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
  // Niles's own values for what is not set, as the server reports them
  // from its code: a copy kept here fell behind within a day.
  const shippedAt = (path: string) => valueAt(view.defaults, path);
  const shippedAnchors = anchorsAt(view.defaults) ?? [];
  const anchors = anchorsAt(view.effective) ?? shippedAnchors;
  const providers =
    (view.effective.providers as import("@/lib/api").Provider[] | undefined) ?? [];
  /** What each provider offers for a role, from the catalogue. */
  const modelsFor = (role: "stt" | "llm") =>
    Object.fromEntries(
      (integrations.data ?? [])
        .map((i) => [i.id, i.models?.[role] ?? []] as const)
        .filter(([, list]) => list.length > 0),
    );
  /** A field of a role section, as the live config has it. */
  const roleValue = (section: "stt" | "llm", field: string) =>
    (view.effective[section] as Record<string, unknown> | undefined)?.[field] as
      | string
      | undefined;
  // Off unless turned on: a new Niles leaves the lights alone.
  const curveOn =
    ((view.effective.lighting as { enabled?: boolean } | undefined)?.enabled ??
      (view.defaults?.lighting as { enabled?: boolean } | undefined)?.enabled) ===
    true;
  // What the curve runs on: its own settings over Niles's, so the chart
  // draws for a house that has changed nothing.
  const curveLighting = {
    ...(view.defaults?.lighting as object | undefined),
    ...(view.effective.lighting as object | undefined),
  };

  const lights = deviceOptions(devices.data ?? []);
  const plugs = deviceOptions(devices.data ?? [], "outlet");
  const noLights = devices.isLoading
    ? "Still asking Niles which lights it has…"
    : "Niles has no lights registered yet.";
  const noPlugs = devices.isLoading
    ? "Still asking Niles which plugs it has…"
    : "Niles has no smart plugs registered.";

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
      setting.kind !== "devices"
        ? setting
        : setting.of === "outlet"
          ? { ...setting, options: plugs, optionsEmpty: noPlugs }
          : { ...setting, options: lights, optionsEmpty: noLights },
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
        defaults={Object.fromEntries(settings.map((s) => [s.path, shippedAt(s.path)]))}
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
      <Tabs value={section ?? SECTIONS[0].items[0].id} onValueChange={(v) => setSection(String(v))}>
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
                  <CurveChart
                    lighting={
                      anchorPreview
                        ? { ...curveLighting, color_temp_anchors: anchorPreview }
                        : curveLighting
                    }
                  />
                  {CURVE_ROWS.map(row)}
                  <AnchorEditor
                    anchors={anchors}
                    shipped={shippedAnchors}
                    overridden={
                      valueAt(view.overrides, "lighting.color_temp_anchors") !== undefined
                    }
                    hot={sectionMeta.get("lighting")?.reload === "hot"}
                    saving={save.isPending || reset.isPending}
                    error={
                      rowError?.row === "lighting.color_temp_anchors"
                        ? rowError.message
                        : undefined
                    }
                    onPreview={setAnchorPreview}
                    onSave={(next) =>
                      save.mutate({
                        row: "lighting.color_temp_anchors",
                        entries: [{ path: "lighting.color_temp_anchors", value: next }],
                      })
                    }
                    onReset={() =>
                      reset.mutate({
                        row: "lighting.color_temp_anchors",
                        paths: ["lighting.color_temp_anchors"],
                      })
                    }
                  />
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

          <Card>
            <CardHeader>
              <CardTitle>Lamps on plugs</CardTitle>
              <CardDescription>
                A smart plug does not say what is plugged into it. Mark the
                ones with a lamp, so they turn off with the lights and the
                fridge does not.
              </CardDescription>
            </CardHeader>
            <CardContent className="divide-border divide-y">
              {LAMP_PLUG_ROWS.map(row)}
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
              address: stringAt(view.effective, "home.address"),
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

        <TabsContent value="mqtt">
          <Card>
            <CardHeader>
              <CardTitle>Zigbee &amp; MQTT</CardTitle>
              <CardDescription>
                The broker every light, plug and sensor is reached through.
                Niles connects once as it starts, so a change here waits for a
                restart.
              </CardDescription>
            </CardHeader>
            <CardContent className="divide-border divide-y">
              {MQTT_ROWS.map(row)}
              <section aria-label="Broker login" className="flex flex-col gap-3 py-4">
                <div>
                  <h3 className="text-sm font-medium">Login</h3>
                  <p className="text-muted-foreground text-sm">
                    Only if the broker asks for one.
                  </p>
                </div>
                {secretFields(["mqtt.username", "mqtt.password"])}
              </section>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="rooms">
          {/* The same list the dashboard builds, arranged the same way,
              so what is dragged here is what moves there. */}
          <RoomOrderCard
            rooms={roomsOf(
              devices.data ?? [],
              climate.data ?? [],
              roomOrder(view.effective),
            )}
            disabled={save.isPending}
            onChange={(order) =>
              save.mutate({
                row: "rooms.order",
                entries: [{ path: "rooms.order", value: order }],
              })
            }
          />
        </TabsContent>

        <TabsContent value="menu">
          <MenuCard
            layout={menuLayout(view.effective)}
            mediaShown={mediaShownOf(view.effective)}
            onMediaShown={(shown) =>
              save.mutate({ row: "menu", entries: [{ path: "menu.media", value: shown }] })
            }
            disabled={save.isPending}
            onChange={({ avatar, main, hidden }) =>
              save.mutate({
                row: "menu",
                entries: [
                  { path: "menu.avatar", value: avatar },
                  { path: "menu.order", value: main },
                  { path: "menu.hidden", value: hidden },
                ],
              })
            }
          />
        </TabsContent>

        <TabsContent value="satellites">
          <SatellitesCard
            satellites={satellitesAt(view.effective)}
            rooms={[...new Set((devices.data ?? []).map((d) => d.room))].sort()}
            music={sonosHost ? musicByRoom(view.effective) : undefined}
            saving={save.isPending || reset.isPending}
            error={rowError?.row === "satellites" ? rowError.message : undefined}
            onChange={(entries) => save.mutate({ row: "satellites", entries })}
            onRemove={(name) =>
              reset.mutate({ row: "satellites", paths: [`satellites.${name}`] })
            }
          />
        </TabsContent>

        <TabsContent value="speakers">
          <SpeakersCard
            report={sonosHost ? speakers.data : { configured: false, error: null, sonos: [] }}
            loading={speakers.isLoading}
            placed={Object.fromEntries(
              Object.entries(sonosAt(view.effective)).map(([id, s]) => [id, s.room]),
            )}
            rooms={[...new Set((devices.data ?? []).map((d) => d.room))].sort()}
            saving={save.isPending || reset.isPending}
            error={rowError?.row === "speakers" ? rowError.message : undefined}
            onPlace={(speaker, room) =>
              save.mutate({
                row: "speakers",
                entries: [
                  { path: `speakers.sonos.${speaker.id}`, value: { room, name: speaker.name } },
                ],
              })
            }
            onUnplace={(speaker) =>
              reset.mutate({ row: "speakers", paths: [`speakers.sonos.${speaker.id}`] })
            }
          />
        </TabsContent>

        <TabsContent value="people">
          <Card className="mb-4">
            <CardHeader>
              <CardTitle>Signing in</CardTitle>
              <CardDescription>
                The GitHub app the sign-in button uses, from github.com under
                Settings, Developer settings, OAuth Apps. The token is for
                scripts and terminals that read Niles without a browser; leave
                it empty if nothing does.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              {secretFields([
                "auth.github_client_id",
                "auth.github_client_secret",
                "auth.api_token",
              ])}
            </CardContent>
          </Card>
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
                voices={voices.data}
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
          <div className="pt-4">
            <VoicesCard
              voices={voices.data ?? (voices.isError ? [] : undefined)}
              onForget={(speaker) => forget.mutate(speaker)}
              onRename={(speaker, name) => rename.mutate({ speaker, name })}
              knownVoicesOnly={
                (view.effective.recognition as { known_voices_only?: boolean } | undefined)
                  ?.known_voices_only === true
              }
              recognitionOn={
                (view.effective.recognition as { enabled?: boolean } | undefined)
                  ?.enabled === true
              }
              saving={save.isPending || forget.isPending || rename.isPending}
              onChange={(value) =>
                save.mutate({
                  row: "recognition",
                  entries: [
                    { path: "recognition.known_voices_only", value },
                  ],
                })
              }
            />
          </div>
          <div className="pt-4">
            <CaptureCard
              enabled={
                (view.effective.capture as { enabled?: boolean } | undefined)
                  ?.enabled === true
              }
              captures={captures.data}
              saving={save.isPending || clearCaptures.isPending}
              onChange={(value) =>
                save.mutate({
                  row: "capture",
                  entries: [{ path: "capture.enabled", value }],
                })
              }
              onClear={() => clearCaptures.mutate()}
            />
          </div>
        </TabsContent>

        <TabsContent value="memory">
          <MemoryCard />
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
                models={modelsFor("stt")}
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
              <label className="flex items-center justify-between gap-4 pt-5">
                <span className="min-w-0">
                  <span className="block text-sm font-medium">
                    Only when it hears its name
                  </span>
                  <span className="text-muted-foreground block text-xs">
                    Ignore a wake unless what was heard starts with
                    &ldquo;Niles&rdquo; &mdash; the television that set the
                    satellite off almost never says it. Best with ElevenLabs,
                    which spells the name right.
                  </span>
                </span>
                <Switch
                  aria-label="Only when it hears its name"
                  checked={valueAt(view.effective, "stt.require_name") === true}
                  disabled={save.isPending}
                  onCheckedChange={(next: boolean) =>
                    save.mutate({
                      row: "stt",
                      entries: [{ path: "stt.require_name", value: next }],
                    })
                  }
                />
              </label>
              {row(LANGUAGE_ROW)}
              <div className="pt-5">
                <RoleCard
                  role="llm"
                  title="Language model"
                  description="Answers anything a pattern cannot."
                  providers={providers}
                  current={roleValue("llm", "provider")}
                  model={roleValue("llm", "model") ?? ""}
                  effort={roleValue("llm", "reasoning_effort")}
                  fallbackHost={hostOf(roleValue("llm", "base_url"))}
                  defaultModel={stringAt(view.defaults, "llm.model")}
                  models={modelsFor("llm")}
                  saving={save.isPending}
                  onSave={(change) =>
                    save.mutate({
                      row: "llm",
                      entries: [
                        { path: "llm.provider", value: change.provider ?? null },
                        { path: "llm.model", value: change.model },
                        {
                          path: "llm.reasoning_effort",
                          value: change.effort ?? null,
                        },
                      ],
                    })
                  }
                />
              </div>
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>Spoken replies</CardTitle>
              <CardDescription>
                What a satellite says back is spoken by Piper, which runs in
                the house rather than in somebody else&apos;s cloud.
              </CardDescription>
            </CardHeader>
            <CardContent className="divide-border divide-y">
              {TTS_ROWS.map(row)}
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="integrations">
          <IntegrationsPage
            onOpenSettings={setSection}
            integrations={integrations.data ?? []}
            secrets={secrets.data}
            tado={tado.data}
            zones={climate.data}
            rooms={[...new Set((devices.data ?? []).map((d) => d.room))].sort()}
            presenceLights={{
              offWhenAway: valueAt(view.effective, "presence.lights_off_when_away") === true,
              onWhenHome: stringsAt(view.effective, "presence.lights_on_when_home"),
            }}
            lightOptions={lights}
            linear={
              (view.effective.integrations as
                | { linear?: { team?: string; trigger_label?: string } }
                | undefined)?.linear
            }
            claudeCode={
              (view.effective.integrations as
                | { claude_code?: { enabled?: boolean; model?: string } }
                | undefined)?.claude_code
            }
            nemlig={
              (view.effective.integrations as
                | { nemlig?: { enabled?: boolean } }
                | undefined)?.nemlig
            }
            spotify={
              (view.effective.integrations as
                | { spotify?: { enabled?: boolean } }
                | undefined)?.spotify
            }
            microsoft={
              (view.effective.integrations as
                | { microsoft?: { enabled?: boolean } }
                | undefined)?.microsoft
            }
            google={
              (view.effective.integrations as
                | { google?: { enabled?: boolean } }
                | undefined)?.google
            }
            unifiHost={stringAt(view.effective, "presence.unifi.host")}
            sonos={{
              enabled: valueAt(view.effective, "speakers.enabled") !== false,
              host: sonosHost,
            }}
            sonosFound={speakers.data}
            tvConfig={
              valueAt(view.effective, "tv") as
                | { host?: string; room?: string; show_announcements?: boolean }
                | undefined
            }
            tv={tv.data}
            onTvChanged={() => {
              queryClient.invalidateQueries({ queryKey: ["tv"] });
              queryClient.invalidateQueries({ queryKey: ["config"] });
            }}
            saving={save.isPending}
            error={rowError ? rowError.message : undefined}
            onChange={(row, entries) => save.mutate({ row, entries })}
            onReset={(row, paths) => reset.mutate({ row, paths })}
            onSecretsChanged={() => secrets.refetch()}
            onTadoChanged={() => tado.refetch()}
          />
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
        Nothing changed yet — everything is as Niles ships it.
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
