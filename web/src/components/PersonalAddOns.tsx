import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { Activity, CalendarDays, ChevronLeft, ChevronRight, Puzzle } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CALENDARS, CalendarsPanel } from "@/components/CalendarsPanel";
import { STRAVA, StravaPanel } from "@/components/StravaPanel";
import { api } from "@/lib/api";
import type { CalendarsView, StravaStatus } from "@/lib/api";
import { cn } from "@/lib/utils";

export type PersonalId = "calendars" | "strava";

export interface PersonalAddOn {
  id: PersonalId;
  label: string;
  /** What it is, under its name in the popup. */
  description: string;
  added: boolean;
  status: { ok: boolean; text: string };
}

const ICONS: Record<PersonalId, ReactNode> = {
  calendars: <CalendarDays aria-hidden className="size-6" />,
  strava: <Activity aria-hidden className="size-6" />,
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

/**
 * The add-on a sign-in has just come back for, from the address.
 *
 * Microsoft, Google and Strava send the browser back here with how it
 * went; the panel that says so has to be open to say it.
 */
export function returningTo(search: string): PersonalId | null {
  const query = new URLSearchParams(search);
  if (query.has("calendar") || query.has("calendar_error")) return "calendars";
  if (query.has("strava") || query.has("strava_error")) return "strava";
  return null;
}

/**
 * Your own add-ons, as one line on the profile and a list behind it,
 * the way Settings keeps the house's.
 *
 * Each grew into a card of its own, and the page became a stack of
 * them. Most people set each up once and then only want to know it
 * works, which is what the line and the list say.
 */
export function PersonalAddOns({ hasVoice }: { hasVoice: boolean }) {
  const calendars = useQuery({
    queryKey: CALENDARS,
    queryFn: api.calendars,
    retry: false,
    enabled: hasVoice,
  });
  const strava = useQuery({ queryKey: STRAVA, queryFn: api.strava, retry: false });
  const addOns = personalAddOns(hasVoice, calendars.data, strava.data);
  const back = returningTo(window.location.search);
  const [open, setOpen] = useState(back !== null);
  const [showing, setShowing] = useState<PersonalId | null>(back);

  const connected = addOns.filter((a) => a.added);
  const available = addOns.filter((a) => !a.added);
  const current = addOns.find((a) => a.id === showing);

  return (
    <>
      <button
        type="button"
        onClick={() => {
          setShowing(null);
          setOpen(true);
        }}
        className="bg-card ring-foreground/10 hover:bg-muted focus-visible:bg-muted flex min-h-14 w-full items-center gap-3 rounded-xl px-4 py-3 text-left ring-1 focus-visible:outline-none"
      >
        <Puzzle aria-hidden className="size-5 shrink-0" />
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium">Add-ons</span>
          <span className="text-muted-foreground block truncate text-xs">
            {connected.length > 0
              ? connected.map((a) => a.label).join(", ")
              : "Your calendars and workouts, connected to you"}
          </span>
        </span>
        <ChevronRight aria-hidden className="text-muted-foreground size-4 shrink-0" />
      </button>

      <Dialog open={open} onOpenChange={(next: boolean) => setOpen(next)}>
        <DialogContent className="sm:w-[calc(100vw-2rem)] sm:max-w-2xl">
          {current ? (
            <>
              <DialogHeader>
                <div className="flex min-w-0 items-start gap-2">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Back to your add-ons"
                    onClick={() => setShowing(null)}
                  >
                    <ChevronLeft aria-hidden />
                  </Button>
                  <div className="flex min-w-0 flex-col gap-1">
                    <DialogTitle>{current.label}</DialogTitle>
                    <DialogDescription>{current.description}</DialogDescription>
                  </div>
                </div>
              </DialogHeader>
              <DialogBody>
                {current.id === "calendars" && <CalendarsPanel />}
                {current.id === "strava" && <StravaPanel />}
              </DialogBody>
            </>
          ) : (
            <>
              <DialogHeader>
                <div className="flex min-w-0 flex-col gap-1">
                  <DialogTitle>Your add-ons</DialogTitle>
                  <DialogDescription>
                    Yours alone: Niles reads them only for you.
                  </DialogDescription>
                </div>
              </DialogHeader>
              <DialogBody className="flex flex-col gap-5">
                {connected.length > 0 && (
                  <Group id="personal-connected" label="Connected">
                    {connected.map((a) => (
                      <Row
                        key={a.id}
                        addOn={a}
                        aria-label={`Configure ${a.label}`}
                        onOpen={() => setShowing(a.id)}
                      >
                        <span className="text-muted-foreground flex items-center gap-1.5 text-xs">
                          <span
                            aria-hidden
                            className={cn(
                              "size-1.5 shrink-0 rounded-full",
                              a.status.ok ? "bg-ok" : "bg-muted-foreground/40",
                            )}
                          />
                          <span className="truncate">{a.status.text}</span>
                        </span>
                      </Row>
                    ))}
                  </Group>
                )}
                {available.length > 0 && (
                  <Group id="personal-available" label="Available">
                    {available.map((a) => (
                      <Row
                        key={a.id}
                        addOn={a}
                        aria-label={`Connect ${a.label}`}
                        onOpen={() => setShowing(a.id)}
                      />
                    ))}
                  </Group>
                )}
              </DialogBody>
            </>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}

/** One group: its name above, its rows in one rounded block. */
function Group({ id, label, children }: { id: string; label: string; children: ReactNode }) {
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
    </section>
  );
}

function Row({
  addOn,
  onOpen,
  children,
  "aria-label": label,
}: {
  addOn: PersonalAddOn;
  onOpen: () => void;
  children?: ReactNode;
  "aria-label": string;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onOpen}
      className="hover:bg-muted focus-visible:bg-muted flex min-h-12 w-full items-center gap-3 px-3 py-2 text-left focus-visible:outline-none"
    >
      <span className="flex shrink-0">{ICONS[addOn.id]}</span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{addOn.label}</span>
        {children}
      </span>
      <ChevronRight aria-hidden className="text-muted-foreground size-4 shrink-0" />
    </button>
  );
}
