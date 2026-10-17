import type {
  Importance,
  Integration,
  IntegrationGroup,
  PluginSummary,
  SecretsReport,
  TadoStatus,
  ToolServer,
  TvInfo,
} from "@/lib/api";

/** Where Linear's own MCP server is. */
export const LINEAR_URL = "https://mcp.linear.app/mcp";

/** Where the Linear webhook's signing secret is kept. */
export const LINEAR_WEBHOOK_SECRET = "linear.webhook_secret";

/** What a request has to mention for Linear's tools to go with it. */
export const LINEAR_KEYWORDS = ["linear", "issue", "issues", "task", "tasks", "ticket", "tickets"];

export const RANK: Record<Importance, number> = {
  required: 0,
  recommended: 1,
  optional: 2,
};

/**
 * The add-ons under their groups, groups in the order they first
 * appear. The server sends them in page order, so this is the page's
 * order without the page knowing what the groups are.
 */
export function groupsOf(
  integrations: Integration[],
): { group: IntegrationGroup; items: Integration[] }[] {
  const groups = new Map<string, { group: IntegrationGroup; items: Integration[] }>();
  for (const integration of integrations) {
    const entry = groups.get(integration.group.id);
    if (entry) entry.items.push(integration);
    else groups.set(integration.group.id, { group: integration.group, items: [integration] });
  }
  return [...groups.values()];
}

/**
 * Whether its row says Required.
 *
 * None of the add-ons Niles ships is — what it cannot work without is
 * under System — but the catalogue can say so, and the row says it
 * when it does.
 */
export function showsRequired(integration: Integration): boolean {
  return !integration.added && integration.importance === "required";
}

/** Whether a search finds it: by name, by what it does, or by its group. */
export function matchesQuery(integration: Integration, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [integration.label, integration.blurb, integration.group.label].some(
    (text) => text.toLowerCase().includes(q),
  );
}

/** What a status line is worked out from: the config, the keys, and
    what the services themselves report. */
export interface StatusInputs {
  secrets?: SecretsReport;
  tado?: TadoStatus;
  toolServers?: ToolServer[];
  claudeCode?: { enabled?: boolean };
  chatgpt?: { enabled?: boolean };
  nemlig?: { enabled?: boolean };
  spotify?: { enabled?: boolean };
  microsoft?: { enabled?: boolean };
  google?: { enabled?: boolean };
  unifiHost?: string;
  webSearchAddress?: string;
  sonos?: { enabled?: boolean; host?: string };
  tv?: TvInfo;
}

export interface Status {
  ok: boolean;
  text: string;
}

const WORKING: Status = { ok: true, text: "Working" };

/**
 * Whether it is finished, not merely added, and if not, what is left.
 *
 * An add-on with no key is a row that looks done and does nothing;
 * naming the missing piece saves opening it to find out.
 */
export function statusOf(integration: Integration, inputs: StatusInputs): Status {
  // A key the report does not list is one this page cannot judge, so
  // it is not held against the add-on.
  const missing = (...keys: (string | null)[]) =>
    keys.some(
      (key) =>
        key !== null &&
        inputs.secrets?.secrets.find((s) => s.key === key)?.source === "unset",
    );
  const needs = (text: string): Status => ({ ok: false, text });
  const off = needs("Switched off");

  switch (integration.id) {
    case "tado":
      return inputs.tado?.authorised ? WORKING : needs("Not signed in to tado° yet");
    // Working only once Linear has answered with its tools: a key that
    // is merely present proved nothing.
    case "linear": {
      if (missing(integration.secret_key)) return needs("Needs its API key");
      const server = inputs.toolServers?.find((s) => s.id === "linear");
      if (!server || server.status === "connecting") return needs("Connecting…");
      if (server.status === "needs_key") return needs("Needs its API key");
      if (server.status === "failed") return needs(server.message ?? "Not connected");
      return WORKING;
    }
    case "nemlig":
      if (inputs.nemlig?.enabled === false) return off;
      return missing("integrations.nemlig.username", "integrations.nemlig.password")
        ? needs("Needs its email and password")
        : WORKING;
    case "claude_code":
      if (inputs.claudeCode?.enabled === false) return off;
      return missing(integration.secret_key) ? needs("Needs its token") : WORKING;
    // Its key is the sign-in's access token, which only signing in writes.
    case "chatgpt":
      if (inputs.chatgpt?.enabled === false) return off;
      return missing(integration.secret_key) ? needs("Not signed in") : WORKING;
    case "spotify":
    case "microsoft":
    case "google": {
      if (inputs[integration.id]?.enabled === false) return off;
      const prefix = `integrations.${integration.id}`;
      return missing(`${prefix}.client_id`, `${prefix}.client_secret`)
        ? needs("Needs its client ID and secret")
        : WORKING;
    }
    case "lg_tv":
      return inputs.tv?.paired ? WORKING : needs("TV not paired yet");
    case "sonos":
      if (inputs.sonos?.enabled === false) return off;
      return inputs.sonos?.host?.trim() ? WORKING : needs("Needs its address");
    case "unifi":
      if (!inputs.unifiHost?.trim()) return needs("Needs its address");
      return missing(integration.secret_key) ? needs("Needs its API key") : WORKING;
    // Whether it answers is the panel's to say, from a real search.
    case "web_search":
      return inputs.webSearchAddress?.trim()
        ? { ok: true, text: "In use" }
        : needs("Needs its address");
    default:
      return missing(integration.secret_key) ? needs("Needs its API key") : WORKING;
  }
}

/**
 * A plugin's line on its row: how it is doing, and, when it is working,
 * that a newer release is waiting. A plugin that is not working says
 * that first: an update is not the fix until somebody decides it is.
 */
export function pluginStatusOf(plugin: PluginSummary): Status {
  switch (plugin.status) {
    case "disabled":
      return { ok: false, text: "Off" };
    case "running":
      return plugin.update
        ? { ok: true, text: `Update available: v${plugin.update.version}` }
        : WORKING;
    default:
      return { ok: false, text: plugin.status_line };
  }
}

/** Whether a search finds it: by name, by what it does, or by its group. */
export function pluginMatchesQuery(plugin: PluginSummary, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [plugin.name, plugin.description, plugin.group.label].some((text) =>
    text.toLowerCase().includes(q),
  );
}
