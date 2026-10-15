import type { CalendarConnection } from "@/lib/api";

/**
 * A person's accounts: one sign-in at Microsoft or Google, or an iCloud
 * app-specific password, serving Calendar, Mail or both. The Calendars
 * and Mail panels each show the accounts that serve them; underneath,
 * one Gmail is one account.
 */

export type CalendarLevel = "none" | "read" | "write";
export type MailLevel = "none" | "read" | "send";

/** What an account serves, each with its access. */
export interface Ticks {
  calendar: CalendarLevel;
  mail: MailLevel;
}

/** The panel a sign-in starts from and comes back to. */
export type Panel = "calendar" | "mail";

/** The query key every panel reads the accounts by. */
export const ACCOUNTS = ["calendars"];

export function ticksOf(account: CalendarConnection): Ticks {
  return {
    calendar: account.calendar ? account.access : "none",
    mail: account.mail ?? "none",
  };
}

export function none(ticks: Ticks): boolean {
  return ticks.calendar === "none" && ticks.mail === "none";
}

/** The link that starts a provider's sign-in for these ticks: followed,
    not fetched, because it ends at Microsoft or Google. */
export function connectUrl(
  provider: "microsoft" | "google",
  name: string,
  ticks: Ticks,
  replace?: string,
  from: Panel = "calendar",
): string {
  const query = new URLSearchParams({
    name: name.trim(),
    calendar: ticks.calendar,
    mail: ticks.mail,
  });
  if (replace) query.set("replace", replace);
  if (from !== "calendar") query.set("from", from);
  return `/calendars/${provider}/start?${query}`;
}

/** What a sign-in that began on `panel` came back saying. */
export function outcomeOf(search: string, panel: Panel): { ok: boolean; text: string } | null {
  const query = new URLSearchParams(search);
  const error = query.get(`${panel}_error`);
  if (error) return { ok: false, text: error };
  if (query.get(panel) === "connected") {
    const name = query.get("name");
    return { ok: true, text: name ? `${name} is connected.` : "Connected." };
  }
  return null;
}

/** How a change of ticks happens: removed when nothing is left; in
    place for iCloud, whose ticks Niles keeps; a sign-in for Microsoft
    and Google, which grant exactly what is asked and no more. */
export type Change =
  | { kind: "remove" }
  | { kind: "in_place"; ticks: Ticks }
  | { kind: "sign_in"; url: string };

export function change(account: CalendarConnection, ticks: Ticks, from: Panel): Change {
  if (none(ticks)) return { kind: "remove" };
  if (account.provider === "icloud") return { kind: "in_place", ticks };
  return {
    kind: "sign_in",
    url: connectUrl(account.provider, account.name, ticks, account.id, from),
  };
}
