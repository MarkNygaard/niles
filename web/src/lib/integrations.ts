import type {
  Importance,
  Integration,
  IntegrationGroup,
  PluginSummary,
  SecretsReport,
  TadoStatus,
  TvInfo,
} from "@/lib/api";

export const RANK: Record<Importance, number> = {
  required: 0,
  recommended: 1,
  optional: 2,
};

/**
 * The integrations under their groups, groups in the order they first
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

/** The roles no added provider serves yet. */
export function uncoveredRoles(integrations: Integration[]): ("stt" | "llm")[] {
  const added = integrations.filter((i) => i.added && i.kind === "provider");
  return (["stt", "llm"] as const).filter(
    (role) => !added.some((p) => p.serves.includes(role)),
  );
}

/**
 * Whether its row says Required.
 *
 * A provider is required only as "one of" for a role: once something
 * serves that role, the others are a choice rather than a gap, and a
 * badge saying otherwise sends somebody off to make accounts they do
 * not need.
 */
export function showsRequired(
  integration: Integration,
  uncovered: ("stt" | "llm")[],
): boolean {
  if (integration.added || integration.importance !== "required") return false;
  if (integration.kind !== "provider") return true;
  return integration.serves.some((role) => uncovered.includes(role));
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
  linear?: { team?: string };
  claudeCode?: { enabled?: boolean };
  chatgpt?: { enabled?: boolean };
  nemlig?: { enabled?: boolean };
  spotify?: { enabled?: boolean };
  microsoft?: { enabled?: boolean };
  google?: { enabled?: boolean };
  unifiHost?: string;
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
 * A provider with no key is a row that looks done and answers nothing;
 * naming the missing piece saves opening it to find out.
 */
export function statusOf(integration: Integration, inputs: StatusInputs): Status {
  // A key the report does not list is one this page cannot judge, so
  // it is not held against the integration.
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
    case "linear":
      return inputs.linear?.team?.trim() ? WORKING : needs("Needs a team");
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
