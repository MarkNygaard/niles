import { useState, type ReactNode } from "react";
import {
  AudioLines,
  Bot,
  Brain,
  Calendar,
  CalendarDays,
  Check,
  ChevronRight,
  ListTodo,
  Music,
  Network,
  Plug,
  Search,
  ShoppingBasket,
  Sparkles,
  Speaker,
  Thermometer,
  Trash2,
  Tv,
  Wifi,
  type LucideIcon,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { BrandMark, hasBrandMark } from "@/components/BrandMark";
import type { DeviceOption } from "@/components/DevicePicker";
import { SecretField } from "@/components/SecretField";
import { TadoPanel } from "@/components/TadoPanel";
import { ClaudeCodePanel } from "@/components/ClaudeCodePanel";
import { ChatGptPanel } from "@/components/ChatGptPanel";
import { NemligPanel } from "@/components/NemligPanel";
import { LinearPanel } from "@/components/LinearPanel";
import { SonosPanel } from "@/components/SonosPanel";
import { CalendarAppPanel, redirectUri } from "@/components/CalendarAppPanel";
import { SpotifyPanel } from "@/components/SpotifyPanel";
import { TvPanel } from "@/components/TvPanel";
import { UnifiPanel } from "@/components/UnifiPanel";
import { MqttPanel } from "@/components/MqttPanel";
import type { MqttSettings } from "@/components/MqttPanel";
import { AddPluginDialog } from "@/components/AddPluginDialog";
import { PluginPanel, PluginRow } from "@/components/PluginPanel";
import { PluginLogo } from "@/components/PluginPreview";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import {
  RANK,
  groupsOf,
  matchesQuery,
  pluginMatchesQuery,
  pluginStatusOf,
  showsRequired,
  statusOf,
  uncoveredRoles,
} from "@/lib/integrations";
import type { Status } from "@/lib/integrations";
import type {
  ChatGptStatus,
  Integration,
  PluginSummary,
  Secret,
  SecretsReport,
  SpeakersReport,
  TadoStatus,
  TvInfo,
  Zone,
} from "@/lib/api";

export interface IntegrationsPageProps {
  integrations: Integration[];
  secrets?: SecretsReport;
  tado?: TadoStatus;
  /** tado's heating zones, and the rooms they can be paired with. */
  zones?: Zone[];
  rooms?: string[];
  /** What presence does to the lights, and the lights to choose from. */
  presenceLights?: { offWhenAway: boolean; onWhenHome: string[] };
  lightOptions?: DeviceOption[];
  /** The Linear section as the config has it, if there is one. */
  linear?: { team?: string; trigger_label?: string };
  /** The Claude Code section as the config has it, if there is one. */
  claudeCode?: { enabled?: boolean; model?: string };
  /** The ChatGPT section as the config has it, if there is one. */
  chatgpt?: { enabled?: boolean; model?: string };
  /** Whether ChatGPT is signed in, and as whom. */
  chatgptStatus?: ChatGptStatus;
  onChatGptChanged?: () => void;
  /** Which assistant answers when both are set up: `integrations.assistant`. */
  assistant?: Assistant;
  /** The nemlig.com section as the config has it, if there is one. */
  nemlig?: { enabled?: boolean };
  /** The Spotify section as the config has it, if there is one. */
  spotify?: { enabled?: boolean };
  /** The Microsoft section as the config has it, if there is one. */
  microsoft?: { enabled?: boolean };
  /** The Google section as the config has it, if there is one. */
  google?: { enabled?: boolean };
  /** The UniFi console's address, empty until one is typed in. */
  unifiHost?: string;
  /** `[speakers]` as the config has it: Sonos's switch and address. */
  sonos?: { enabled?: boolean; host?: string };
  /** What the speaker at that address described. */
  sonosFound?: SpeakersReport;
  /** `[tv]` as the config has it. */
  tvConfig?: { host?: string; room?: string; show_announcements?: boolean };
  tv?: TvInfo;
  onTvChanged?: () => void;
  saving?: boolean;
  error?: string;
  /** Config writes, batched as one revision per action. */
  onChange: (row: string, entries: { path: string; value: unknown }[]) => void;
  /** Drop what was set, returning those paths to Niles's defaults. */
  onReset: (row: string, paths: string[]) => void;
  onSecretsChanged: () => void;
  onTadoChanged: () => void;
  /** `[mqtt]` as the config has it, and Niles's own values for it. */
  mqtt?: MqttSettings;
  mqttDefaults?: MqttSettings;
  /** Third-party integrations installed from GitHub. */
  plugins?: PluginSummary[];
  /** Something about the plugins changed: ask for them again. */
  onPluginsChanged?: () => void;
}

/**
 * What Niles is connected to, and what it could be.
 *
 * Rows rather than cards. An endpoint, a key and a model are all setup
 * detail — true, and worth nothing at a glance. What this page is for
 * is seeing which things are connected and getting into the one you
 * came to change, and both of those fit on a row; the rest is in the
 * dialog a row opens.
 *
 * Connected and Available are two tabs over the same groups, so a thing
 * is found in the same place whether or not it is set up yet. Only
 * Niles's own catalogue is offered: the pair of text boxes this replaces
 * let any name and any URL buy a provider that would never work.
 */
export function IntegrationsPage({
  integrations,
  secrets,
  tado,
  zones,
  rooms,
  presenceLights,
  lightOptions,
  linear,
  claudeCode,
  chatgpt,
  chatgptStatus,
  onChatGptChanged,
  assistant,
  nemlig,
  spotify,
  microsoft,
  google,
  unifiHost,
  sonos,
  sonosFound,
  tvConfig,
  tv,
  onTvChanged,
  saving,
  error,
  onChange,
  onReset,
  onSecretsChanged,
  onTadoChanged,
  mqtt,
  mqttDefaults,
  plugins = [],
  onPluginsChanged,
}: IntegrationsPageProps) {
  const [open, setOpen] = useState<string | null>(null);
  const [openPlugin, setOpenPlugin] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Tab | null>(null);

  const added = integrations.filter((i) => i.added);
  const available = integrations.filter((i) => !i.added);
  const current = integrations.find((i) => i.id === open);
  const currentPlugin = plugins.find((p) => p.id === openPlugin);
  // Worked out rather than set once: the list arrives after the first
  // render, and a house with nothing set up yet should land on what it
  // can add rather than on an empty tab.
  const connected = added.length + plugins.length;
  const tab: Tab = picked ?? (connected > 0 ? "connected" : "available");
  const uncovered = uncoveredRoles(integrations);

  function secretFor(key: string | null): Secret | undefined {
    if (!key) return undefined;
    return secrets?.secrets.find((s) => s.key === key);
  }

  function status(integration: Integration): Status {
    return statusOf(integration, {
      secrets,
      tado,
      linear,
      claudeCode,
      chatgpt,
      nemlig,
      spotify,
      microsoft,
      google,
      unifiHost,
      sonos,
      tv,
    });
  }

  function add(integration: Integration) {
    // Kept on Available: someone working down the list from a new
    // install comes back to where they were, not to the first thing
    // they added.
    setPicked(tab);
    // Opened straight away: adding one is never the whole job, and a
    // row that appears with a grey dot and no prompt is something you
    // have to work out you are not finished with.
    setOpen(integration.id);
    switch (integration.id) {
      case "tado":
        return onChange("presence", [
          { path: "presence.enabled", value: true },
          { path: "presence.tado", value: {} },
        ]);
      case "linear":
        return onChange("integrations.linear", [
          { path: "integrations.linear", value: { team: "" } },
        ]);
      case "claude_code":
        return onChange("integrations.claude_code", [
          { path: "integrations.claude_code.enabled", value: true },
        ]);
      case "chatgpt":
        return onChange("integrations.chatgpt", [
          { path: "integrations.chatgpt.enabled", value: true },
        ]);
      case "nemlig":
        return onChange("integrations.nemlig", [
          { path: "integrations.nemlig.enabled", value: true },
        ]);
      case "spotify":
        return onChange("integrations.spotify", [
          { path: "integrations.spotify.enabled", value: true },
        ]);
      // The redirect as this browser reaches Niles, written down rather
      // than worked out from forwarded headers on every sign-in:
      // Microsoft compares it letter for letter.
      case "microsoft":
        return onChange("integrations.microsoft", [
          { path: "integrations.microsoft.enabled", value: true },
          { path: "integrations.microsoft.redirect_uri", value: redirectUri("microsoft") },
        ]);
      case "google":
        return onChange("integrations.google", [
          { path: "integrations.google.enabled", value: true },
          { path: "integrations.google.redirect_uri", value: redirectUri("google") },
        ]);
      case "mqtt":
      case "unifi":
      case "sonos":
      case "lg_tv":
        // Nothing to write yet: the address is what makes it added, and
        // that is the first thing the dialog asks for.
        return;
      default:
        // A provider is its endpoint and the roles it serves, both of
        // which Niles already knows. Appending rather than replacing:
        // adding one must not drop the others.
        return onChange("providers", [
          {
            path: "providers",
            value: [...providerRows(integrations), asRow(integration)],
          },
        ]);
    }
  }

  /** The keys it keeps that were typed in here, as against ones an
      environment variable provides, which are not this page's to delete. */
  function storedKeys(integration: Integration): string[] {
    return integration.secret_keys.filter(
      (key) => secretFor(key)?.source === "stored",
    );
  }

  async function remove(integration: Integration) {
    // Its keys go with it. A key shows only on its integration's card,
    // so one left behind is one nobody can see or delete. Cleared
    // before the config changes, because the server accepts a
    // provider's key only while that provider is declared.
    const keys = storedKeys(integration);
    if (keys.length > 0) {
      setRemoveError(null);
      setRemoving(true);
      try {
        for (const key of keys) await api.clearSecret(key);
      } catch (e) {
        // Stopped rather than carried on: removing it anyway would
        // strand whatever is left with nothing to show it.
        setRemoveError(e instanceof Error ? e.message : String(e));
        return;
      } finally {
        setRemoving(false);
        onSecretsChanged();
      }
    }
    setOpen(null);
    switch (integration.id) {
      case "tado":
        return onChange("presence", [
          { path: "presence.enabled", value: false },
        ]);
      case "unifi":
        return onChange("presence.unifi", [
          { path: "presence.unifi.host", value: "" },
        ]);
      // The broker's login went with its keys, above.
      case "mqtt":
        return onChange("mqtt", [{ path: "mqtt.host", value: "" }]);
      // The pairing key is gone with it: it belongs to one TV, so a TV
      // added again pairs again.
      case "lg_tv":
        return onChange("tv", [{ path: "tv.host", value: "" }]);
      // The rooms each Sonos is placed in stay, for when it comes back.
      case "sonos":
        return onChange("speakers", [{ path: "speakers.host", value: "" }]);
      // Gone, not switched off — the card's own switch is for that, and
      // keeps the token. Removing it deletes the token too.
      case "claude_code":
        return onReset("integrations.claude_code", ["integrations.claude_code"]);
      // The sign-in went with its keys, above.
      case "chatgpt":
        return onReset("integrations.chatgpt", ["integrations.chatgpt"]);
      // Like Claude Code: gone from the config, and the login with it.
      case "nemlig":
        return onReset("integrations.nemlig", ["integrations.nemlig"]);
      case "spotify":
        return onReset("integrations.spotify", ["integrations.spotify"]);
      // The app's keys are deleted; everyone's own connections stay in
      // the database for when it comes back with new ones.
      case "microsoft":
        return onReset("integrations.microsoft", ["integrations.microsoft"]);
      case "google":
        return onReset("integrations.google", ["integrations.google"]);
      default:
        return onChange("providers", [
          {
            path: "providers",
            value: providerRows(integrations).filter(
              (p) => p.name !== integration.id,
            ),
          },
        ]);
    }
  }

  // Asked only when there is a choice to make. With one assistant that
  // one answers, and a choice of one is a setting with nothing to do.
  const bothAssistants = ["claude_code", "chatgpt"].every((id) =>
    added.some((i) => i.id === id),
  );

  function results(which: Tab) {
    const all = which === "connected" ? added : available;
    const found = all.filter((i) => matchesQuery(i, query));
    // Installed plugins are connected things; there is nothing of theirs
    // to offer under Available but the way to add one.
    const pluginsHere = which === "connected" ? plugins : [];
    const foundPlugins = pluginsHere.filter((p) => pluginMatchesQuery(p, query));
    const groups = groupsOf(found);
    const pluginRow = (plugin: PluginSummary) => (
      <PluginRow
        key={`plugin-${plugin.id}`}
        plugin={plugin}
        status={pluginStatusOf(plugin)}
        disabled={saving}
        onOpen={() => setOpenPlugin(plugin.id)}
      />
    );
    const other: Tab = which === "connected" ? "available" : "connected";
    const elsewhere = (which === "connected" ? available : added).filter((i) =>
      matchesQuery(i, query),
    );
    const searching = query.trim() !== "";

    return (
      <>
        {searching && found.length === 0 && foundPlugins.length === 0 ? (
          <p className="text-muted-foreground px-1 text-sm">
            Nothing matches “{query.trim()}”.
          </p>
        ) : all.length === 0 && pluginsHere.length === 0 ? (
          // Nothing at all is the catalogue still loading, not a house
          // with everything connected.
          integrations.length > 0 && (
            <p className="text-muted-foreground px-1 text-sm">
              {which === "connected"
                ? "Nothing connected yet. Start under Available: the ones marked Required are what Niles needs to reach your lights, hear you and answer."
                : "Everything Niles knows is connected."}
            </p>
          )
        ) : (
          groups.map(({ group, items }) => (
            <GroupList
              key={group.id}
              id={`${which}-group-${group.id}`}
              label={group.label}
              footer={
                which === "available"
                  ? group.requirement && (
                      <Requirement
                        text={group.requirement}
                        providers={integrations.filter(
                          (i) => i.group.id === group.id && i.kind === "provider",
                        )}
                      />
                    )
                  : group.id === "assistants" &&
                    bothAssistants && (
                      <AssistantChoice
                        value={assistant ?? "claude_code"}
                        disabled={saving}
                        onChoose={(value) =>
                          onChange("integrations.assistant", [
                            { path: "integrations.assistant", value },
                          ])
                        }
                      />
                    )
              }
            >
              {[...items]
                .sort((a, b) => RANK[a.importance] - RANK[b.importance])
                .map((integration) =>
                  which === "connected" ? (
                    <ConnectedRow
                      key={integration.id}
                      integration={integration}
                      status={status(integration)}
                      disabled={saving}
                      onOpen={() => setOpen(integration.id)}
                    />
                  ) : (
                    <AvailableRow
                      key={integration.id}
                      integration={integration}
                      required={showsRequired(integration, uncovered)}
                      disabled={saving}
                      onConnect={() => add(integration)}
                    />
                  ),
                )}
              {foundPlugins.filter((p) => p.group.id === group.id).map(pluginRow)}
            </GroupList>
          ))
        )}
        {/* A plugin naming a group with nothing else connected in it, or
            none Niles knows (Custom), gets a block of its own after the
            built-in ones. */}
        {pluginGroupsOf(
          foundPlugins.filter((p) => !groups.some(({ group }) => group.id === p.group.id)),
        ).map(({ group, items }) => (
          <GroupList key={group.id} id={`${which}-group-${group.id}`} label={group.label}>
            {items.map(pluginRow)}
          </GroupList>
        ))}
        {which === "available" && (
          <GroupList id="available-group-github" label="From GitHub">
            <div className="flex min-h-12 items-center gap-3 py-2 pr-1.5 pl-3">
              <span className="flex shrink-0">
                <BrandMark id="github" label="GitHub" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">Add from GitHub</span>
                <span className="text-muted-foreground block text-xs">
                  An integration somebody else wrote, for what Niles does not cover.
                </span>
              </span>
              <Button
                variant="link"
                size="sm"
                disabled={saving}
                aria-label="Add from GitHub"
                onClick={() => setAdding(true)}
              >
                Add
              </Button>
            </div>
          </GroupList>
        )}
        {searching && elsewhere.length > 0 && (
          // The tab is a filter too, and one search should not need two.
          <button
            type="button"
            onClick={() => setPicked(other)}
            className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex items-center gap-1 self-start rounded-md px-1 text-left text-sm focus-visible:outline-none"
          >
            {names(elsewhere)} {elsewhere.length === 1 ? "is" : "are"} under{" "}
            {TAB_LABELS[other]}
            <ChevronRight aria-hidden className="size-4 shrink-0" />
          </button>
        )}
      </>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Integrations</CardTitle>
          <CardDescription>The services Niles works through.</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="relative">
            <Search
              aria-hidden
              className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2"
            />
            {/* Not a login: with the setup panels' password fields on the
                page, Chrome took this for the username box and filled in the
                signed-in address whenever a panel closed. */}
            <Input
              type="search"
              name="integration-search"
              autoComplete="off"
              data-1p-ignore
              data-lpignore="true"
              aria-label="Search integrations"
              placeholder="Search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="pl-8"
            />
          </div>

          <Tabs value={tab} onValueChange={(value) => setPicked(value as Tab)}>
            <TabsList className="w-full sm:w-fit">
              {(["connected", "available"] as const).map((value) => (
                <TabsTrigger key={value} value={value} className="sm:px-3">
                  {TAB_LABELS[value]}{" "}
                  <span className="text-muted-foreground tabular-nums">
                    {value === "connected" ? connected : available.length}
                  </span>
                </TabsTrigger>
              ))}
            </TabsList>
            <TabsContent value="connected" className="flex flex-col gap-5 pt-2">
              {results("connected")}
            </TabsContent>
            <TabsContent value="available" className="flex flex-col gap-5 pt-2">
              {results("available")}
            </TabsContent>
          </Tabs>
        </CardContent>
      </Card>

      {error && <p className="text-destructive text-sm">{error}</p>}

      <AddPluginDialog
        open={adding}
        onOpenChange={setAdding}
        onInstalled={(plugin) => {
          setPicked("connected");
          setOpenPlugin(plugin.id);
          onPluginsChanged?.();
        }}
      />

      <Dialog
        open={currentPlugin !== undefined}
        onOpenChange={(next: boolean) => {
          if (!next) setOpenPlugin(null);
        }}
      >
        <DialogContent className="sm:max-w-lg">
          {currentPlugin && (
            <>
              <DialogHeader>
                <div className="flex min-w-0 flex-col gap-1">
                  <DialogTitle className="flex items-center gap-2">
                    <PluginLogo src={currentPlugin.logo} />
                    {currentPlugin.name}
                  </DialogTitle>
                  <DialogDescription>{currentPlugin.description}</DialogDescription>
                </div>
              </DialogHeader>
              <DialogBody>
                <PluginPanel
                  key={currentPlugin.id}
                  plugin={currentPlugin}
                  onChanged={() => onPluginsChanged?.()}
                  onRemoved={() => {
                    setOpenPlugin(null);
                    onPluginsChanged?.();
                    // Its saved keys went with it.
                    onSecretsChanged();
                  }}
                />
              </DialogBody>
            </>
          )}
        </DialogContent>
      </Dialog>

      <Dialog
        open={current !== undefined}
        onOpenChange={(next: boolean) => {
          if (!next) {
            setOpen(null);
            setRemoveError(null);
          }
        }}
      >
        <DialogContent className="sm:max-w-lg">
          {current && (
            <>
              <DialogHeader>
                <div className="flex min-w-0 flex-col gap-1">
                  <DialogTitle className="flex items-center gap-2">
                    <Logo integration={current} />
                    {current.label}
                    {status(current).ok && (
                      <Badge variant="secondary">configured</Badge>
                    )}
                  </DialogTitle>
                  {/* The rows show only a name, so how it works and
                      what it takes are said here, where it is set up. */}
                  <DialogDescription>{current.blurb}</DialogDescription>
                  <p className="text-muted-foreground text-sm">{current.needs}</p>
                </div>
              </DialogHeader>
              <DialogBody className="flex flex-col gap-4">
                {current.id === "tado" && tado && (
                  <TadoPanel
                    status={tado}
                    saving={saving}
                    onToggle={(on) =>
                      onChange("presence", [
                        { path: "presence.enabled", value: on },
                      ])
                    }
                    onChanged={onTadoChanged}
                    lights={presenceLights}
                    lightOptions={lightOptions}
                    onLightsChange={(entries) => onChange("presence", entries)}
                    zones={zones}
                    rooms={rooms}
                    onPair={(zoneId, room) =>
                      onChange("presence", [
                        {
                          path: `presence.tado.rooms.${zoneId}`,
                          value: room,
                        },
                      ])
                    }
                  />
                )}

                {current.id === "linear" && (
                  <LinearPanel
                    team={linear?.team ?? ""}
                    triggerLabel={linear?.trigger_label ?? ""}
                    secret={secretFor(current.secret_key)}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onChange={(entries) =>
                      onChange("integrations.linear", entries)
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "claude_code" && (
                  <ClaudeCodePanel
                    enabled={claudeCode?.enabled !== false}
                    model={claudeCode?.model ?? "sonnet"}
                    secret={secretFor(current.secret_key)}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.claude_code", [
                        { path: "integrations.claude_code.enabled", value: enabled },
                      ])
                    }
                    onModel={(model) =>
                      onChange("integrations.claude_code", [
                        { path: "integrations.claude_code.model", value: model },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "chatgpt" && (
                  <ChatGptPanel
                    enabled={chatgpt?.enabled !== false}
                    model={chatgpt?.model ?? "gpt-5.6-terra"}
                    status={chatgptStatus}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.chatgpt", [
                        { path: "integrations.chatgpt.enabled", value: enabled },
                      ])
                    }
                    onModel={(model) =>
                      onChange("integrations.chatgpt", [
                        { path: "integrations.chatgpt.model", value: model },
                      ])
                    }
                    onChanged={() => {
                      onSecretsChanged();
                      onChatGptChanged?.();
                    }}
                  />
                )}

                {current.id === "nemlig" && (
                  <NemligPanel
                    enabled={nemlig?.enabled !== false}
                    username={secretFor("integrations.nemlig.username")}
                    password={secretFor("integrations.nemlig.password")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.nemlig", [
                        { path: "integrations.nemlig.enabled", value: enabled },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "spotify" && (
                  <SpotifyPanel
                    enabled={spotify?.enabled !== false}
                    clientId={secretFor("integrations.spotify.client_id")}
                    clientSecret={secretFor("integrations.spotify.client_secret")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.spotify", [
                        { path: "integrations.spotify.enabled", value: enabled },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "microsoft" && (
                  <CalendarAppPanel
                    provider="microsoft"
                    enabled={microsoft?.enabled !== false}
                    clientId={secretFor("integrations.microsoft.client_id")}
                    clientSecret={secretFor("integrations.microsoft.client_secret")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.microsoft", [
                        { path: "integrations.microsoft.enabled", value: enabled },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "google" && (
                  <CalendarAppPanel
                    provider="google"
                    enabled={google?.enabled !== false}
                    clientId={secretFor("integrations.google.client_id")}
                    clientSecret={secretFor("integrations.google.client_secret")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("integrations.google", [
                        { path: "integrations.google.enabled", value: enabled },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "lg_tv" && (
                  <TvPanel
                    host={tvConfig?.host ?? ""}
                    room={tvConfig?.room ?? ""}
                    showAnnouncements={tvConfig?.show_announcements !== false}
                    tv={tv}
                    rooms={rooms ?? []}
                    saving={saving}
                    onChange={(entries) => onChange("tv", entries)}
                    onPaired={() => {
                      onSecretsChanged();
                      onTvChanged?.();
                    }}
                  />
                )}

                {current.id === "sonos" && (
                  <SonosPanel
                    enabled={sonos?.enabled !== false}
                    host={sonos?.host ?? ""}
                    found={sonosFound}
                    saving={saving}
                    onEnabled={(enabled) =>
                      onChange("speakers", [{ path: "speakers.enabled", value: enabled }])
                    }
                    onHost={(host) =>
                      onChange("speakers", [{ path: "speakers.host", value: host }])
                    }
                  />
                )}

                {current.id === "mqtt" && (
                  <MqttPanel
                    mqtt={mqtt ?? {}}
                    defaults={mqttDefaults ?? {}}
                    username={secretFor("mqtt.username")}
                    password={secretFor("mqtt.password")}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onChange={(entries) => onChange("mqtt", entries)}
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.id === "unifi" && (
                  <UnifiPanel
                    host={unifiHost ?? ""}
                    secret={secretFor(current.secret_key)}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onHost={(host) =>
                      onChange("presence.unifi", [
                        { path: "presence.unifi.host", value: host },
                      ])
                    }
                    onSecretsChanged={onSecretsChanged}
                  />
                )}

                {current.kind === "provider" && (
                  <>
                    <p className="text-muted-foreground text-sm">
                      Choose what it does under Speech &amp; language.
                    </p>
                    {secretFor(current.secret_key) && (
                      <SecretField
                        // The endpoint is named once below. Repeating it
                        // beside the field, inside this provider's own
                        // dialog, says the same thing twice.
                        secret={{
                          ...secretFor(current.secret_key)!,
                          hint: undefined,
                        }}
                        writable={secrets?.writable ?? false}
                        onChanged={onSecretsChanged}
                      />
                    )}
                    <p className="text-muted-foreground font-mono text-xs">
                      {current.base_url}
                    </p>
                  </>
                )}

                <div className="flex flex-col gap-1 border-t pt-3">
                  <div>
                    <Button
                      variant="ghost"
                      className="text-muted-foreground"
                      disabled={saving || removing}
                      onClick={() => remove(current)}
                    >
                      <Trash2 aria-hidden /> Remove {current.label}
                    </Button>
                  </div>
                  {/* Said before the click rather than after: there is
                      nowhere to get a deleted key back from. */}
                  {storedKeys(current).length > 0 && (
                    <p className="text-muted-foreground px-3 text-xs">
                      Its saved keys are deleted too.
                    </p>
                  )}
                  {removeError && (
                    <p className="text-destructive px-3 text-sm">{removeError}</p>
                  )}
                </div>
              </DialogBody>
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

type Tab = "connected" | "available";

/** `integrations.assistant`: which one answers when both are set up. */
export type Assistant = "claude_code" | "chatgpt";

const ASSISTANTS: { id: Assistant; label: string }[] = [
  { id: "claude_code", label: "Claude" },
  { id: "chatgpt", label: "ChatGPT" },
];

/**
 * Which assistant answers, under the two of them. Here rather than with
 * the voice settings: the assistant answers the chat, new rules and the
 * nightly review, and this is where both of them are set up.
 */
function AssistantChoice({
  value,
  disabled,
  onChoose,
}: {
  value: Assistant;
  disabled?: boolean;
  onChoose: (value: Assistant) => void;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-3 pt-1">
      <span className="min-w-0">
        <span id="assistant-choice" className="block text-sm font-medium">
          Which assistant answers
        </span>
        <span className="text-muted-foreground block text-xs">
          If it is switched off or not signed in, the other one answers.
        </span>
      </span>
      <div
        role="radiogroup"
        aria-labelledby="assistant-choice"
        className="bg-muted/60 flex gap-1 rounded-lg p-1"
      >
        {ASSISTANTS.map((a) => {
          const active = a.id === value;
          return (
            <button
              key={a.id}
              type="button"
              role="radio"
              aria-checked={active}
              disabled={disabled}
              onClick={() => !active && onChoose(a.id)}
              className={cn(
                "rounded-md px-3 py-1 text-sm transition-colors",
                "focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
                active
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {a.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

const TAB_LABELS: Record<Tab, string> = {
  connected: "Connected",
  available: "Available",
};

/** Plugins under their groups, in the order they first appear, with
    Custom last: it follows the built-in groups on the page. */
function pluginGroupsOf(
  plugins: PluginSummary[],
): { group: PluginSummary["group"]; items: PluginSummary[] }[] {
  const groups = new Map<string, { group: PluginSummary["group"]; items: PluginSummary[] }>();
  for (const plugin of plugins) {
    const entry = groups.get(plugin.group.id);
    if (entry) entry.items.push(plugin);
    else groups.set(plugin.group.id, { group: plugin.group, items: [plugin] });
  }
  return [...groups.values()].sort(
    (a, b) => Number(a.group.id === "custom") - Number(b.group.id === "custom"),
  );
}

/** "Groq", "Groq and Cerebras", "Groq, Cerebras and UniFi", "4 more". */
function names(integrations: Integration[]): string {
  if (integrations.length > 3) return `${integrations.length} more`;
  const labels = integrations.map((i) => i.label);
  if (labels.length === 1) return labels[0];
  return `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
}

/**
 * The icon names the catalogue uses. A table of icons rather than of
 * integrations: a new entry that reuses one needs nothing here, and one
 * that names an icon missing from it still gets a plug.
 */
const ICONS: Record<string, LucideIcon> = {
  network: Network,
  sparkles: Sparkles,
  "audio-lines": AudioLines,
  brain: Brain,
  speaker: Speaker,
  music: Music,
  tv: Tv,
  wifi: Wifi,
  thermometer: Thermometer,
  calendar: Calendar,
  "calendar-days": CalendarDays,
  "shopping-basket": ShoppingBasket,
  bot: Bot,
  "list-todo": ListTodo,
};

/** The service's own mark; the catalogue's icon only where there is
    none (the broker), which says more than its initial would. */
function Logo({ integration }: { integration: Integration }) {
  if (hasBrandMark(integration.id)) {
    return (
      <span data-logo="brand" className="flex shrink-0">
        <BrandMark id={integration.id} label={integration.label} />
      </span>
    );
  }
  const Icon = ICONS[integration.icon] ?? Plug;
  return (
    <span data-logo="icon" className="flex shrink-0">
      <Icon aria-hidden className="size-6" />
    </span>
  );
}

/** One group: its name above, its rows in one rounded block. */
function GroupList({
  id,
  label,
  footer,
  children,
}: {
  id: string;
  label: string;
  footer?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={id} className="flex flex-col gap-1.5">
      <h3
        id={id}
        className="text-muted-foreground px-3 text-xs font-medium tracking-wide uppercase"
      >
        {label}
      </h3>
      <div className="bg-muted/50 divide-border/60 divide-y overflow-hidden rounded-2xl">
        {children}
      </div>
      {footer}
    </section>
  );
}

/** The group's "one of these", and how far along it is. */
function Requirement({
  text,
  providers,
}: {
  text: string;
  providers: Integration[];
}) {
  return (
    <div className="text-muted-foreground px-3 text-xs">
      <p>{text}</p>
      {providers.length > 0 && (
        <ul className="mt-1 flex flex-wrap gap-x-4 gap-y-1">
          {(["stt", "llm"] as const).map((role) => {
            const by = providers.filter((p) => p.added && p.serves.includes(role));
            return (
              <li key={role} className="flex items-center gap-1">
                {by.length > 0 && <Check aria-hidden className="text-lit size-3" />}
                {ROLE_LABELS[role]}:{" "}
                {by.length > 0 ? by.map((p) => p.label).join(", ") : "not yet"}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

const ROLE_LABELS: Record<"stt" | "llm", string> = {
  stt: "Speech-to-text",
  llm: "AI",
};

/** Something not set up yet: its name, and a way to start. */
function AvailableRow({
  integration,
  required,
  disabled,
  onConnect,
}: {
  integration: Integration;
  required: boolean;
  disabled?: boolean;
  onConnect: () => void;
}) {
  return (
    <div className="flex min-h-12 items-center gap-3 py-2 pr-1.5 pl-3">
      <Logo integration={integration} />
      <span className="flex min-w-0 flex-1 items-center gap-2">
        <span className="truncate text-sm font-medium">{integration.label}</span>
        {required && <Badge>Required</Badge>}
      </span>
      <Button
        variant="link"
        size="sm"
        disabled={disabled}
        aria-label={`Connect ${integration.label}`}
        onClick={onConnect}
      >
        Connect
      </Button>
    </div>
  );
}

/** Something set up: whether it works, and the way into its settings. */
function ConnectedRow({
  integration,
  status,
  disabled,
  onOpen,
}: {
  integration: Integration;
  status: Status;
  disabled?: boolean;
  onOpen: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      aria-label={`Configure ${integration.label}`}
      onClick={onOpen}
      className="hover:bg-muted focus-visible:bg-muted flex min-h-12 w-full items-center gap-3 px-3 py-2 text-left focus-visible:outline-none disabled:opacity-50"
    >
      <Logo integration={integration} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {integration.label}
        </span>
        <span className="text-muted-foreground flex items-center gap-1.5 text-xs">
          <span
            aria-hidden
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              status.ok ? "bg-ok" : "bg-muted-foreground/40",
            )}
          />
          <span className="truncate">{status.text}</span>
        </span>
      </span>
      <ChevronRight aria-hidden className="text-muted-foreground size-4 shrink-0" />
    </button>
  );
}

/** The `[[providers]]` array as it stands, ready to be written back. */
function providerRows(integrations: Integration[]) {
  return integrations
    .filter((i) => i.added && i.kind === "provider")
    .map(asRow);
}

function asRow(integration: Integration) {
  return {
    name: integration.id,
    base_url: integration.base_url,
    serves: integration.serves,
  };
}
