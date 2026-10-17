import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  PersonalAddOnPage,
  addOnHint,
  personalAddOns,
  returningTo,
} from "@/components/PersonalAddOns";
import type { PersonalAddOn } from "@/components/PersonalAddOns";
import { api } from "@/lib/api";
import type { CalendarsView, StravaStatus } from "@/lib/api";

const NO_STRAVA: StravaStatus = {
  client_id: null,
  secret_set: false,
  connected: false,
  athlete: null,
  last_sync: null,
  last_error: null,
  syncing: false,
  paused_until: null,
  activities: 0,
  runs: 0,
};

const ONE_CALENDAR: CalendarsView = {
  providers: [{ id: "icloud", available: true, reason: null }],
  connections: [
    {
      id: "c1",
      provider: "icloud",
      name: "Private",
      account: "mark@icloud.com",
      calendar: true,
      access: "read",
      mail: null,
      enforced_by_provider: false,
    },
  ],
};

function page(addOn: PersonalAddOn) {
  vi.spyOn(api, "calendars").mockResolvedValue(ONE_CALENDAR);
  vi.spyOn(api, "strava").mockResolvedValue(NO_STRAVA);
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <PersonalAddOnPage addOn={addOn} />
    </QueryClientProvider>,
  );
}

describe("personalAddOns", () => {
  it("leaves calendars and mail out for somebody Niles cannot hear", () => {
    expect(personalAddOns(false, ONE_CALENDAR, NO_STRAVA).map((a) => a.id)).toEqual(["strava"]);
    expect(personalAddOns(true, ONE_CALENDAR, NO_STRAVA).map((a) => a.id)).toEqual([
      "calendars",
      "mail",
      "strava",
    ]);
  });

  it("counts one account's calendar and mail separately", () => {
    const both: CalendarsView = {
      ...ONE_CALENDAR,
      connections: [{ ...ONE_CALENDAR.connections[0], mail: "send" }],
    };
    const [calendars, mail] = personalAddOns(true, both, NO_STRAVA);
    expect(calendars.added).toBe(true);
    expect(mail.added).toBe(true);
    expect(mail.status.text).toBe("Private (can send)");
    const [, none] = personalAddOns(true, ONE_CALENDAR, NO_STRAVA);
    expect(none.added).toBe(false);
  });

  it("counts Strava as added once there is an app, connected or not", () => {
    const [strava] = personalAddOns(false, undefined, { ...NO_STRAVA, client_id: "123" });
    expect(strava.added).toBe(true);
    expect(strava.status).toEqual({ ok: false, text: "Not connected yet" });
    const [working] = personalAddOns(false, undefined, {
      ...NO_STRAVA,
      client_id: "123",
      connected: true,
      athlete: "Majse H",
    });
    expect(working.status).toEqual({ ok: true, text: "Connected as Majse H" });
  });

  it("says what went wrong before anything else", () => {
    const [strava] = personalAddOns(false, undefined, {
      ...NO_STRAVA,
      connected: true,
      last_error: "Strava refused the token",
    });
    expect(strava.status).toEqual({ ok: false, text: "Strava refused the token" });
  });
});

describe("returningTo", () => {
  it("knows which sign-in the browser came back from", () => {
    expect(returningTo("?calendar=connected&name=Work")).toBe("calendars");
    expect(returningTo("?calendar_error=denied")).toBe("calendars");
    expect(returningTo("?strava=connected")).toBe("strava");
    expect(returningTo("?mail=connected&name=Private")).toBe("mail");
    expect(returningTo("?mail_error=denied")).toBe("mail");
    expect(returningTo("")).toBeNull();
  });
});

describe("addOnHint", () => {
  it("says what is connected, or that nothing is yet", () => {
    const [calendars, mail, strava] = personalAddOns(true, ONE_CALENDAR, NO_STRAVA);
    expect(addOnHint(calendars)).toBe("Private");
    expect(addOnHint(mail)).toBe("Not connected yet");
    expect(addOnHint(strava)).toBe("Not connected yet");
  });
});

describe("PersonalAddOnPage", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("is a page of its own, not a dialog", async () => {
    const [, , strava] = personalAddOns(true, ONE_CALENDAR, NO_STRAVA);
    page(strava);
    expect(screen.getByText("Strava")).toBeInTheDocument();
    expect(screen.getByText(/Yours alone/)).toBeInTheDocument();
    expect(await screen.findByText(/everyone makes their own/)).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows a calendar's accounts in place", async () => {
    const [calendars] = personalAddOns(true, ONE_CALENDAR, NO_STRAVA);
    page(calendars);
    expect(await screen.findByText("Private")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
