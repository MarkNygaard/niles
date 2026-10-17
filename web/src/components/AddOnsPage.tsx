import { useEffect, useState, type ReactNode } from "react";
import {
  Bot,
  Calendar,
  CalendarDays,
  ChevronRight,
  ListTodo,
  Music,
  Plug,
  Search,
  ShoppingBasket,
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
import { TadoPanel } from "@/components/TadoPanel";
import { ClaudeCodePanel } from "@/components/ClaudeCodePanel";
import { ChatGptPanel } from "@/components/ChatGptPanel";
import { NemligPanel } from "@/components/NemligPanel";
import { ToolServerPanel } from "@/components/ToolServerPanel";
import type { ToolMode } from "@/components/ToolServerPanel";
import { SonosPanel } from "@/components/SonosPanel";
import { CalendarAppPanel, redirectUri } from "@/components/CalendarAppPanel";
import { SpotifyPanel } from "@/components/SpotifyPanel";
import { TvPanel } from "@/components/TvPanel";
import { UnifiPanel } from "@/components/UnifiPanel";
import { WebSearchPanel } from "@/components/WebSearchPanel";
import { AddPluginDialog } from "@/components/AddPluginDialog";
import { PluginPanel, PluginRow } from "@/components/PluginPanel";
import { PluginLogo } from "@/components/PluginPreview";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import {
  LINEAR_KEYWORDS,
  LINEAR_URL,
  RANK,
  groupsOf,
  matchesQuery,
  pluginMatchesQuery,
  pluginStatusOf,
  showsRequired,
  statusOf,
} from "@/lib/integrations";
import type { Status } from "@/lib/integrations";
import { connectOutcome, pluginGroupsOf } from "@/lib/plugins";
import type {
  ChatGptStatus,
  Integration,
  PluginSummary,
  Secret,
  SecretsReport,
  SpeakersReport,
  TadoStatus,
  ToolServer,
  TvInfo,
  Zone,
} from "@/lib/api";

export interface AddOnsPageProps {
  /** The whole catalogue; the base system in it is left out here. */
  integrations: Integration[];
  secrets?: SecretsReport;
  tado?: TadoStatus;
  /** tado's heating zones, and the rooms they can be paired with. */
  zones?: Zone[];
  rooms?: string[];
  /** What presence does to the lights, and the lights to choose from. */
  presenceLights?: { offWhenAway: boolean; onWhenHome: string[] };
  lightOptions?: DeviceOption[];
  /** Linear's tool-server section as the config has it, if added. */
  linear?: { notes?: string; tools?: Record<string, ToolMode>; hand_off?: boolean };
  /** The remote tool servers, as Niles is connected to them. */
  toolServers?: ToolServer[];
  /** Something about a tool server changed: ask for them again. */
  onToolServersChanged?: () => void;
  /** The Claude Code section as the config has it, if there is one. */
  claudeCode?: { enabled?: boolean; model?: string };
  /** The ChatGPT section as the config has it, if there is one. */
  chatgpt?: { enabled?: boolean; model?: string };
  /** Whether ChatGPT is signed in, and as whom. */
  chatgptStatus?: ChatGptStatus;
  onChatGptChanged?: () => void;
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
  /** The SearXNG address, empty until one is typed in. */
  webSearchAddress?: string;
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
  /** Third-party add-ons installed from GitHub. */
  plugins?: PluginSummary[];
  /** Something about the plugins changed: ask for them again. */
  onPluginsChanged?: () => void;
  /** The config as it is, for where each plugin's widgets are. */
  effective?: unknown;
}

/**
 * The optional extras: what Niles can do beyond lights and voice, its
 * own and other people's.
 *
 * The base system — the broker, who hears and who thinks — is not here:
 * it is under System, set up by the job it does, because a list of
 * optional things is the wrong place to find what Niles cannot work
 * without.
 *
 * Rows rather than cards. A key and an address are setup detail — true,
 * and worth nothing at a glance. What this page is for is seeing which
 * add-ons are connected and getting into the one you came to change,
 * and both of those fit on a row; the rest is in the dialog a row opens.
 *
 * Connected and Available are two tabs over the same groups, so an
 * add-on is found in the same place whether or not it is set up yet.
 */
export function AddOnsPage({
  integrations: catalogue,
  secrets,
  tado,
  zones,
  rooms,
  presenceLights,
  lightOptions,
  linear,
  toolServers,
  onToolServersChanged,
  claudeCode,
  chatgpt,
  chatgptStatus,
  onChatGptChanged,
  nemlig,
  spotify,
  microsoft,
  google,
  unifiHost,
  webSearchAddress,
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
  plugins = [],
  onPluginsChanged,
  effective,
}: AddOnsPageProps) {
  const [open, setOpen] = useState<string | null>(null);
  // Back from a plugin's sign-in page, its panel opens saying how it went.
  const [returned] = useState(() => connectOutcome(window.location.search));
  const [openPlugin, setOpenPlugin] = useState<string | null>(returned?.plugin ?? null);
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Tab | null>(null);

  const integrations = catalogue.filter((i) => !i.system);
  const added = integrations.filter((i) => i.added);
  const available = integrations.filter((i) => !i.added);
  const current = integrations.find((i) => i.id === open);
  const currentPlugin = plugins.find((p) => p.id === openPlugin);

  // Said once, then taken off the address so a refresh does not say it
  // again.
  useEffect(() => {
    if (!returned) return;
    window.history.replaceState(null, "", window.location.pathname + window.location.hash);
  }, [returned]);
  // Worked out rather than set once: the list arrives after the first
  // render, and a house with nothing set up yet should land on what it
  // can add rather than on an empty tab.
  const connected = added.length + plugins.length;
  const tab: Tab = picked ?? (connected > 0 ? "connected" : "available");

  function secretFor(key: string | null): Secret | undefined {
    if (!key) return undefined;
    return secrets?.secrets.find((s) => s.key === key);
  }

  function status(integration: Integration): Status {
    return statusOf(integration, {
      secrets,
      tado,
      toolServers,
      claudeCode,
      chatgpt,
      nemlig,
      spotify,
      microsoft,
      google,
      unifiHost,
      webSearchAddress,
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
        return onChange("tool_servers.linear", [
          {
            path: "tool_servers.linear",
            value: { url: LINEAR_URL, name: "Linear", keywords: LINEAR_KEYWORDS },
          },
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
      default:
        // UniFi, Sonos, the TV, web search: nothing to write yet. The
        // address is what makes it added, and that is the first thing
        // the dialog asks for.
        return;
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
    // Its keys go with it. A key shows only on its add-on's card, so
    // one left behind is one nobody can see or delete. Cleared first,
    // so a key that cannot be deleted stops the removal rather than
    // being stranded by it.
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
      // The pairing key is gone with it: it belongs to one TV, so a TV
      // added again pairs again.
      case "lg_tv":
        return onChange("tv", [{ path: "tv.host", value: "" }]);
      // The rooms each Sonos is placed in stay, for when it comes back.
      case "sonos":
        return onChange("speakers", [{ path: "speakers.host", value: "" }]);
      // Written empty rather than reset, so an address a ConfigMap
      // still names is set aside too. Claude goes back to its own
      // search with the next message.
      case "web_search":
        return onChange("web_search", [{ path: "web_search.base_url", value: "" }]);
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
      case "linear":
        return onReset("tool_servers.linear", ["tool_servers.linear"]);
      // The app's keys are deleted; everyone's own connections stay in
      // the database for when it comes back with new ones.
      case "microsoft":
        return onReset("integrations.microsoft", ["integrations.microsoft"]);
      case "google":
        return onReset("integrations.google", ["integrations.google"]);
    }
  }

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
                ? "No add-ons yet. Everything Niles needs to work is under System; these are extras, for when you want them."
                : "Every add-on Niles ships is connected."}
            </p>
          )
        ) : (
          groups.map(({ group, items }) => (
            <GroupList
              key={group.id}
              id={`${which}-group-${group.id}`}
              label={group.label}
              footer={
                // Which one answers is chosen under Voice & AI, beside
                // Niles's own model; here is only how each is set up.
                which === "connected" &&
                group.id === "assistants" && (
                  <p className="text-muted-foreground px-3 text-xs">
                    Which assistant answers is chosen under System, in Voice &amp; AI.
                  </p>
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
                      required={showsRequired(integration)}
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
                <span className="block truncate text-sm font-medium">Add-on from GitHub</span>
                <span className="text-muted-foreground block text-xs">
                  A third-party add-on somebody else wrote, for what Niles does not cover.
                </span>
              </span>
              <Button
                variant="link"
                size="sm"
                disabled={saving}
                aria-label="Add-on from GitHub"
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
          <CardTitle>Add-ons</CardTitle>
          <CardDescription>
            Optional extras: music, presence, calendars, shopping and
            assistants, and add-ons other people wrote.
          </CardDescription>
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
              name="addon-search"
              autoComplete="off"
              data-1p-ignore
              data-lpignore="true"
              aria-label="Search add-ons"
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
        <DialogContent className="sm:w-[calc(100vw-2rem)] sm:max-w-2xl">
          {currentPlugin && (
            <>
              <DialogHeader>
                <div className="flex min-w-0 flex-col gap-1">
                  <DialogTitle className="flex items-center gap-2">
                    <PluginLogo plugin={currentPlugin} />
                    {currentPlugin.name}
                  </DialogTitle>
                  <DialogDescription>{currentPlugin.description}</DialogDescription>
                </div>
              </DialogHeader>
              <DialogBody>
                <PluginPanel
                  key={currentPlugin.id}
                  plugin={currentPlugin}
                  outcome={returned?.plugin === currentPlugin.id ? returned : null}
                  rooms={rooms}
                  secrets={secrets}
                  effective={effective}
                  onChanged={() => onPluginsChanged?.()}
                  onSecretsChanged={onSecretsChanged}
                  onPlaceWidget={(widget, place) =>
                    onChange(`plugins.${currentPlugin.id}.widgets`, [
                      { path: `menu.widgets.${currentPlugin.id}.${widget}`, value: place },
                    ])
                  }
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
        <DialogContent className="sm:w-[calc(100vw-2rem)] sm:max-w-2xl">
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
                  <ToolServerPanel
                    id="linear"
                    server={toolServers?.find((t) => t.id === "linear")}
                    modes={linear?.tools ?? {}}
                    notes={linear?.notes ?? ""}
                    handOff={linear?.hand_off !== false}
                    notesPlaceholder="For example: a coding task goes to the team Niles, in Todo, with the label AI Eligible."
                    secret={secretFor(current.secret_key)}
                    writable={secrets?.writable ?? false}
                    saving={saving}
                    onChange={(entries) => onChange("tool_servers.linear", entries)}
                    onSecretsChanged={onSecretsChanged}
                    onRefreshed={() => onToolServersChanged?.()}
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

                {current.id === "web_search" && (
                  <WebSearchPanel
                    address={webSearchAddress ?? ""}
                    saving={saving}
                    onAddress={(address) =>
                      onChange("web_search", [{ path: "web_search.base_url", value: address }])
                    }
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

                <div className="flex flex-col gap-1 border-t pt-3">
                  <div>
                    <Button
                      variant="ghost"
                      className="text-muted-foreground"
                      disabled={saving || removing}
                      onClick={() => remove(current)}
                    >
                      <Trash2 aria-hidden /> Remove add-on
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

const TAB_LABELS: Record<Tab, string> = {
  connected: "Connected",
  available: "Available",
};

/** "UniFi", "UniFi and tado°", "UniFi, tado° and Sonos", "4 more". */
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
  search: Search,
};

/** The service's own mark; the catalogue's icon only where there is
    none, which says more than its initial would. */
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
