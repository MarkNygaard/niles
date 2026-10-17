import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MyProfile, profileSection, profileSections } from "@/components/MyProfile";
import { personalAddOns } from "@/components/PersonalAddOns";
import { api } from "@/lib/api";
import type { DeviceView, Me, PendingApproval, StravaStatus } from "@/lib/api";

const ME: Me = {
  email: "mark@example.com",
  speaker: "mark",
  display_name: "Mark",
  spoken_as: null,
  address_as: "Sir",
  notes: "- Takes tea, not coffee",
  birthday: "10-03",
  phone: null,
};

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

const HOME: DeviceView = {
  available: true,
  on_home_network: true,
  mac: "aa:bb:cc:dd:ee:ff",
  name: "Mark's iPhone",
  paired: false,
  has_phone: false,
  signed_in: true,
};

const WAITING: PendingApproval = {
  id: "a1",
  title: "Let Linear create issue?",
  subtitle: "Linear",
  icon: "linear",
  fields: [{ label: "Title", value: "WLED presets" }],
  body: null,
  allow_conversation: true,
  status: "pending",
  outcome: null,
  tool: "Linear",
  summary: "Title: WLED presets",
  conversation_id: 4,
  asked_at: new Date().toISOString(),
  expires_at: new Date(Date.now() + 3_600_000).toISOString(),
  via: "app",
};

class FakeSocket {
  onopen: (() => void) | null = null;
  onmessage: (() => void) | null = null;
  onclose: (() => void) | null = null;
  close() {}
}

function viewport(phone: boolean) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches: query.includes("max-width") && phone,
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

function show({ phone = false, me = ME }: { phone?: boolean; me?: Me } = {}) {
  viewport(phone);
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.spyOn(api, "me").mockResolvedValue(me);
  vi.spyOn(api, "deviceStatus").mockResolvedValue(HOME);
  vi.spyOn(api, "calendars").mockResolvedValue({ providers: [], connections: [] });
  vi.spyOn(api, "strava").mockResolvedValue(NO_STRAVA);
  vi.spyOn(api, "approvals").mockResolvedValue({ approvals: [WAITING] });
  vi.spyOn(api, "approvalHistory").mockResolvedValue({ entries: [], next: null, kept: true });
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MyProfile />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  window.history.replaceState(null, "", "/");
});

describe("profileSections", () => {
  it("groups you, each of your add-ons, and approvals", () => {
    const sections = profileSections(personalAddOns(true, undefined, NO_STRAVA));
    expect(sections.map((g) => g.group)).toEqual(["You", "Add-ons", "Approvals"]);
    expect(sections[0].items.map((s) => s.label)).toEqual([
      "About you",
      "My phone",
      "Notifications",
    ]);
    expect(sections[1].items.map((s) => s.id)).toEqual(["calendars", "mail", "strava"]);
  });

  it("has no calendar for somebody Niles cannot hear", () => {
    const sections = profileSections(personalAddOns(false, undefined, NO_STRAVA));
    expect(sections[1].items.map((s) => s.id)).toEqual(["strava"]);
    expect(profileSection("/me/profile/calendars", sections)).toBeNull();
    expect(profileSection("/me/profile/strava", sections)).toBe("strava");
    expect(profileSection("/me/profile", sections)).toBeNull();
  });
});

describe("MyProfile", () => {
  it("opens on About you beside the list on a desk", async () => {
    show();
    const nav = await screen.findByRole("navigation");
    expect(within(nav).getByRole("button", { name: /About you/ })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByText("What Niles knows about you")).toBeInTheDocument();
  });

  it("switches section from the list, an add-on as a page and not a dialog", async () => {
    show();
    const nav = await screen.findByRole("navigation");
    fireEvent.click(within(nav).getByRole("button", { name: /Strava/ }));
    await waitFor(() => expect(window.location.hash).toBe("#/me/profile/strava"));
    expect(await screen.findByText(/everyone makes their own/)).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText("What Niles knows about you")).toBeNull();
  });

  it("counts what waits beside Approvals, and shows it there", async () => {
    show();
    const nav = await screen.findByRole("navigation");
    const approvals = within(nav).getByRole("button", { name: /Approvals/ });
    await waitFor(() => expect(approvals).toHaveTextContent("1 waiting"));
    fireEvent.click(approvals);
    expect(await screen.findByRole("region", { name: "Let Linear create issue?" })).toBeInTheDocument();
    expect(await screen.findByText("Nothing decided yet.")).toBeInTheDocument();
  });

  it("is the list on a phone, and a section pushes in with a way back", async () => {
    show({ phone: true });
    const nav = await screen.findByRole("navigation");
    expect(screen.queryByText("What Niles knows about you")).toBeNull();
    fireEvent.click(within(nav).getByRole("button", { name: /My phone/ }));
    expect(await screen.findByText(/Pair Mark's iPhone/)).toBeInTheDocument();
    expect(screen.queryByRole("navigation")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "My profile" }));
    expect(await screen.findByRole("navigation")).toBeInTheDocument();
  });

  it("lands on the add-on a sign-in has just come back for", async () => {
    window.history.replaceState(null, "", "/?strava_error=denied#/me");
    show();
    await waitFor(() => expect(window.location.hash).toBe("#/me/profile/strava"));
    expect(await screen.findByText(/everyone makes their own/)).toBeInTheDocument();
  });
});
