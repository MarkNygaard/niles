import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MailPanel, mailProviders } from "@/components/MailPanel";
import { api } from "@/lib/api";
import type { CalendarConnection, CalendarsView } from "@/lib/api";

const PROVIDERS: CalendarsView["providers"] = [
  { id: "microsoft", available: true, reason: null },
  { id: "google", available: true, reason: null },
  { id: "icloud", available: true, reason: null },
];

const GMAIL: CalendarConnection = {
  id: "acc-1",
  provider: "google",
  name: "Private",
  account: "mark@gmail.com",
  calendar: true,
  access: "write",
  mail: "read",
  enforced_by_provider: true,
};

const PHONE: CalendarConnection = {
  ...GMAIL,
  id: "acc-2",
  provider: "icloud",
  name: "Phone",
  account: "mark@icloud.com",
  mail: null,
  enforced_by_provider: false,
};

function panel(connections: CalendarConnection[]) {
  vi.spyOn(api, "calendars").mockResolvedValue({ providers: PROVIDERS, connections });
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MailPanel />
    </QueryClientProvider>,
  );
}

describe("MailPanel", () => {
  afterEach(() => vi.restoreAllMocks());

  it("shows the accounts that serve mail, read only by default", async () => {
    panel([GMAIL, PHONE]);
    expect(await screen.findByText("Private")).toBeInTheDocument();
    // Connected read only: the way to change it is to allow sending.
    expect(screen.getByRole("button", { name: "Allow sending" })).toBeInTheDocument();
    expect(screen.queryByText("Phone")).toBeNull();
    expect(screen.getByRole("radio", { name: /Read only/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  it("does not offer iCloud for mail yet", () => {
    expect(mailProviders({ providers: PROVIDERS, connections: [] }).map((p) => p.id)).toEqual([
      "microsoft",
      "google",
    ]);
  });

  it("offers a calendar account for its mail too", async () => {
    panel([{ ...GMAIL, mail: null }]);
    expect(
      await screen.findByRole("button", {
        name: /Use mark@gmail\.com, already connected for your calendar/,
      }),
    ).toBeInTheDocument();
  });

  it("asks before disconnecting, and keeps the calendar of an account that has one", async () => {
    panel([GMAIL]);
    fireEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
    // A Google account keeping its calendar is signed in again with
    // less; that is a link followed, not a call, so nothing is removed.
    const remove = vi.spyOn(api, "removeCalendar").mockResolvedValue(undefined);
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, assign, search: "" });
    fireEvent.click(screen.getByRole("button", { name: "Disconnect Private" }));
    await waitFor(() =>
      expect(assign).toHaveBeenCalledWith(
        "/calendars/google/start?name=Private&calendar=write&mail=none&replace=acc-1&from=mail",
      ),
    );
    expect(remove).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
