import { Activity, CalendarDays, Mail, type LucideIcon } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { CalendarsPanel } from "@/components/CalendarsPanel";
import { MailPanel } from "@/components/MailPanel";
import { StravaPanel } from "@/components/StravaPanel";
import type { CalendarsView, StravaStatus } from "@/lib/api";

export type PersonalId = "calendars" | "mail" | "strava";

export interface PersonalAddOn {
  id: PersonalId;
  label: string;
  /** What it is, at the top of its page. */
  description: string;
  added: boolean;
  status: { ok: boolean; text: string };
}

export const PERSONAL_ICONS: Record<PersonalId, LucideIcon> = {
  calendars: CalendarDays,
  mail: Mail,
  strava: Activity,
};

/**
 * Yours, and how each is doing.
 *
 * Calendars only with a voice: a calendar is read when Niles hears its
 * owner, and somebody it cannot recognise has no calendar to read.
 */
export function personalAddOns(
  hasVoice: boolean,
  calendars?: CalendarsView,
  strava?: StravaStatus,
): PersonalAddOn[] {
  const out: PersonalAddOn[] = [];
  if (hasVoice) {
    const names = (calendars?.connections ?? []).filter((c) => c.calendar).map((c) => c.name);
    out.push({
      id: "calendars",
      label: "Calendars",
      description:
        "Read by Niles only when it hears you, or when you ask here. Nobody else can ask for yours.",
      added: names.length > 0,
      status: { ok: true, text: names.join(", ") },
    });
    // Mail, like calendars, is read only for the voice it belongs to.
    const mail = (calendars?.connections ?? []).filter((c) => c.mail);
    out.push({
      id: "mail",
      label: "Mail",
      description:
        "Read by Niles only when it hears you, or when you ask here. A mail it writes waits for you to send it.",
      added: mail.length > 0,
      status: {
        ok: true,
        text: mail.map((c) => (c.mail === "send" ? `${c.name} (can send)` : c.name)).join(", "),
      },
    });
  }
  out.push({
    id: "strava",
    label: "Strava",
    description: "Your workouts, from your own Strava. Yours alone: Niles reads them only for you.",
    added: Boolean(strava?.connected || strava?.client_id),
    status: stravaStatus(strava),
  });
  return out;
}

function stravaStatus(s?: StravaStatus): { ok: boolean; text: string } {
  if (s?.last_error) return { ok: false, text: s.last_error };
  if (s?.connected) return { ok: true, text: `Connected as ${s.athlete ?? "you"}` };
  return { ok: false, text: "Not connected yet" };
}

/** The line under an add-on's name in My profile's list. */
export function addOnHint(addOn: PersonalAddOn): string {
  if (!addOn.added) return "Not connected yet";
  return addOn.status.text || "Connected";
}

/**
 * The add-on a sign-in has just come back for, from the address.
 *
 * Microsoft, Google and Strava send the browser back here with how it
 * went; the page that says so has to be open to say it.
 */
export function returningTo(search: string): PersonalId | null {
  const query = new URLSearchParams(search);
  if (query.has("calendar") || query.has("calendar_error")) return "calendars";
  if (query.has("mail") || query.has("mail_error")) return "mail";
  if (query.has("strava") || query.has("strava_error")) return "strava";
  return null;
}

/**
 * One of your add-ons as a page of My profile, the way Settings shows a
 * section beside its list: what it is, and everything to set it up.
 *
 * A page rather than the dialog it used to be. A dialog over a list is
 * the house's add-ons, where there are dozens; yours are three, each in
 * the profile's own list, and a sign-in that comes back lands on its
 * page with nothing to reopen.
 */
export function PersonalAddOnPage({ addOn }: { addOn: PersonalAddOn }) {
  const Icon = PERSONAL_ICONS[addOn.id];
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Icon aria-hidden className="size-5" />
          {addOn.label}
        </CardTitle>
        <CardDescription>{addOn.description}</CardDescription>
      </CardHeader>
      <CardContent>
        {addOn.id === "calendars" && <CalendarsPanel />}
        {addOn.id === "mail" && <MailPanel />}
        {addOn.id === "strava" && <StravaPanel />}
      </CardContent>
    </Card>
  );
}
