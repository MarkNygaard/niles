import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bell, ChevronLeft, ShieldCheck, Smartphone, UserRound } from "lucide-react";
import { Approvals } from "@/components/ApprovalsList";
import { CALENDARS } from "@/components/CalendarsPanel";
import { AboutYou, PhoneCard } from "@/components/MeCard";
import { Notifications } from "@/components/NotificationsCard";
import {
  PERSONAL_ICONS,
  PersonalAddOnPage,
  addOnHint,
  personalAddOns,
  returningTo,
} from "@/components/PersonalAddOns";
import type { PersonalAddOn } from "@/components/PersonalAddOns";
import { SettingsNav } from "@/components/SettingsNav";
import type { Section } from "@/components/SettingsNav";
import { STRAVA } from "@/components/StravaPanel";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useDeviceStream } from "@/hooks/useDeviceStream";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { api } from "@/lib/api";
import { PENDING } from "@/lib/approvals";
import { useRoute } from "@/lib/route";
import { cn } from "@/lib/utils";

/** Where the page opens on a desk, with nothing picked yet. */
const FIRST = "about";

/**
 * My profile's list, grouped the way Settings groups the house's:
 * you, then what is connected to you one add-on at a time, then what
 * waits for your yes.
 */
export function profileSections(addOns: PersonalAddOn[]): { group: string; items: Section[] }[] {
  return [
    {
      group: "You",
      items: [
        {
          id: "about",
          label: "About you",
          hint: "What Niles knows, and how it speaks to you",
          icon: UserRound,
        },
        {
          id: "phone",
          label: "My phone",
          hint: "How Niles knows you are home",
          icon: Smartphone,
        },
        {
          id: "notifications",
          label: "Notifications",
          hint: "Which of your devices Niles tells things to",
          icon: Bell,
        },
      ],
    },
    {
      group: "Add-ons",
      items: addOns.map((a) => ({
        id: a.id,
        label: a.label,
        hint: addOnHint(a),
        icon: PERSONAL_ICONS[a.id],
      })),
    },
    {
      group: "Approvals",
      items: [
        {
          id: "approvals",
          label: "Approvals",
          hint: "What waits for your yes, and what you decided",
          icon: ShieldCheck,
        },
      ],
    },
  ];
}

/** The section a route names, if the list has it: `/me/profile/strava`. */
export function profileSection(route: string, sections: { items: Section[] }[]): string | null {
  const id = /^\/me\/profile\/([^/]+)/.exec(route)?.[1] ?? null;
  return id && sections.some((g) => g.items.some((s) => s.id === id)) ? id : null;
}

/**
 * Your own page, beside Settings on the Me tab rather than inside it.
 *
 * Settings is the house — lights, rooms, add-ons — and most of the
 * household never needs it. Everyone has a birthday and a way they like
 * to be spoken to, so that sits where a phone app keeps your profile.
 *
 * Laid out as Settings is, a list and the section it opened, because
 * the page outgrew a stack of cards: on a phone the list is the screen
 * and a section pushes in; from `sm` they sit side by side.
 */
export function MyProfile() {
  const queryClient = useQueryClient();
  // Answers 401 to the API token, which is nobody in particular.
  const me = useQuery({ queryKey: ["me"], queryFn: api.me, retry: false });
  // The dashboard's key, so pairing here takes its offer away there.
  const phone = useQuery({
    queryKey: ["presence-device"],
    queryFn: api.deviceStatus,
    retry: false,
  });
  const hasVoice = Boolean(me.data?.speaker);
  const calendars = useQuery({
    queryKey: CALENDARS,
    queryFn: api.calendars,
    retry: false,
    enabled: hasVoice,
  });
  const strava = useQuery({ queryKey: STRAVA, queryFn: api.strava, retry: false });
  const pending = useQuery({ queryKey: PENDING, queryFn: api.approvals, retry: false });
  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["me"] });
    queryClient.invalidateQueries({ queryKey: ["presence-device"] });
    queryClient.invalidateQueries({ queryKey: ["voices"] });
  };
  const update = useMutation({ mutationFn: api.updateMe, onSuccess: refresh });
  const pair = useMutation({ mutationFn: api.pairDevice, onSuccess: refresh });
  const unpair = useMutation({ mutationFn: api.unpairPhone, onSuccess: refresh });

  const addOns = personalAddOns(hasVoice, calendars.data, strava.data);
  const sections = profileSections(addOns);
  const phoneScreen = useMediaQuery("(max-width: 639px)");
  const route = useRoute();
  const section = profileSection(route, sections);
  const setSection = (id: string | null) => {
    window.location.hash = id ? `#/me/profile/${id}` : "#/me/profile";
  };

  // Back from a sign-in, on the page of the add-on it was for. The
  // search stays: that page reads how it went from it.
  const [landing] = useState(() => returningTo(window.location.search));
  useEffect(() => {
    if (!landing) return;
    window.history.replaceState(
      null,
      "",
      `${window.location.pathname}${window.location.search}#/me/profile/${landing}`,
    );
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  }, [landing]);

  if (me.isError) {
    return (
      <p className="text-muted-foreground text-sm">
        This page belongs to whoever is signed in, and nobody is.
      </p>
    );
  }
  if (!me.data) return <Skeleton className="h-64 w-full" />;

  const shown = section ?? (phoneScreen ? null : FIRST);
  const saving = update.isPending || pair.isPending || unpair.isPending;
  const waiting = pending.data?.approvals.length ?? 0;

  return (
    <div className="flex flex-col gap-5 sm:flex-row sm:gap-6">
      {(!phoneScreen || shown === null) && (
        <SettingsNav
          sections={sections}
          current={phoneScreen ? null : shown}
          onPick={setSection}
          todo={{ approvals: waiting }}
          counting="waiting"
        />
      )}
      <div className={cn("flex min-w-0 flex-1 flex-col gap-4", shown === null && "hidden")}>
        {phoneScreen && (
          <button
            type="button"
            onClick={() => setSection(null)}
            className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 -ml-1 flex items-center gap-1 self-start rounded-lg py-1 pr-2 text-sm focus-visible:outline-none"
          >
            <ChevronLeft aria-hidden className="size-4" />
            My profile
          </button>
        )}
        {shown === "about" && (
          <AboutYou
            me={me.data}
            saving={saving}
            error={update.error?.message}
            onSave={(change) => update.mutate(change)}
          />
        )}
        {shown === "phone" && (
          <>
            <PhoneCard
              device={phone.data}
              phone={me.data.phone}
              saving={saving}
              onPair={() => pair.mutate()}
              onUnpair={() => unpair.mutate()}
            />
            {(pair.error ?? unpair.error) && (
              <p className="text-destructive text-sm">
                {(pair.error ?? unpair.error)?.message}
              </p>
            )}
          </>
        )}
        {shown === "notifications" && <Notifications />}
        {addOns
          .filter((a) => a.id === shown)
          .map((a) => (
            <PersonalAddOnPage key={a.id} addOn={a} />
          ))}
        {shown === "approvals" && <ProfileApprovals />}
      </div>
    </div>
  );
}

/** Approvals on the profile, kept live: the chat's stream is not open here. */
function ProfileApprovals() {
  useDeviceStream();
  return (
    <Card>
      <CardHeader>
        <CardTitle>Approvals</CardTitle>
        <CardDescription>
          What Niles would not do on its own: a mail to send, a tool set to Ask first. Only
          you see yours.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Approvals />
      </CardContent>
    </Card>
  );
}
