import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PersonalAddOns, personalAddOns, returningTo } from "@/components/PersonalAddOns";
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

function page(hasVoice = true) {
  vi.spyOn(api, "calendars").mockResolvedValue(ONE_CALENDAR);
  vi.spyOn(api, "strava").mockResolvedValue(NO_STRAVA);
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <PersonalAddOns hasVoice={hasVoice} />
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

describe("PersonalAddOns", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    window.history.replaceState(null, "", "/");
  });

  it("is one line on the page, naming what is connected", async () => {
    page();
    const line = await screen.findByRole("button", { name: /Add-ons\s*Calendars/ });
    expect(line).toBeInTheDocument();
    expect(screen.queryByText("Connect a calendar")).toBeNull();
  });

  it("lists yours like Settings lists the house's, and opens each", async () => {
    page();
    fireEvent.click(await screen.findByRole("button", { name: /Add-ons\s*Calendars/ }));
    const connected = screen.getByRole("region", { name: "Connected" });
    expect(within(connected).getByText("Private")).toBeInTheDocument();
    const available = screen.getByRole("region", { name: "Available" });
    fireEvent.click(within(available).getByRole("button", { name: "Connect Strava" }));
    expect(await screen.findByText(/everyone makes their own/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back to your add-ons" }));
    expect(screen.getByRole("region", { name: "Connected" })).toBeInTheDocument();
  });

  it("opens on the add-on a sign-in has just come back for", async () => {
    window.history.replaceState(null, "", "/?strava_error=denied");
    page();
    expect(await screen.findByRole("heading", { name: "Strava" })).toBeInTheDocument();
  });
});
