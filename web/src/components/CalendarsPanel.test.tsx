import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { CalendarsPanel, offered, outcomeOf } from "./CalendarsPanel";
import { redirectUri } from "./CalendarAppPanel";
import { api } from "@/lib/api";
import type { CalendarsView } from "@/lib/api";

const ALL_AVAILABLE: CalendarsView["providers"] = [
  { id: "microsoft", available: true, reason: null },
  { id: "google", available: true, reason: null },
  { id: "icloud", available: true, reason: null },
];

const MARK: CalendarsView = {
  providers: ALL_AVAILABLE,
  connections: [
    {
      id: "cal-1",
      provider: "microsoft",
      name: "Private",
      account: "mark@hotmail.com",
      calendar: true,
      access: "write",
      mail: null,
      enforced_by_provider: true,
    },
    {
      id: "cal-2",
      provider: "microsoft",
      name: "Work",
      account: "mark@work.example",
      calendar: true,
      access: "read",
      mail: null,
      enforced_by_provider: true,
    },
    {
      id: "cal-3",
      provider: "icloud",
      name: "Phone",
      account: "mark@icloud.com",
      calendar: true,
      access: "read",
      mail: null,
      enforced_by_provider: false,
    },
  ],
};

function card(view: CalendarsView = MARK) {
  vi.spyOn(api, "calendars").mockResolvedValue(view);
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <CalendarsPanel />
    </QueryClientProvider>,
  );
}

describe("CalendarsPanel", () => {
  afterEach(() => vi.restoreAllMocks());

  it("says how the sign-in went", () => {
    expect(outcomeOf("?calendar=connected&name=Work")).toEqual({
      ok: true,
      text: "Work is connected.",
    });
    expect(outcomeOf("?calendar_error=AADSTS65001%3A%20admin%20approval")?.ok).toBe(false);
    expect(outcomeOf("")).toBeNull();
  });

  it("names the redirect each app registration needs", () => {
    expect(redirectUri("microsoft", "https://niles.mnygaard.io")).toBe(
      "https://niles.mnygaard.io/calendars/microsoft/callback",
    );
    expect(redirectUri("google", "https://niles.mnygaard.io")).toBe(
      "https://niles.mnygaard.io/calendars/google/callback",
    );
  });

  it("offers what can be connected first", () => {
    expect(
      offered({
        providers: [
          { id: "microsoft", available: false, reason: "no keys" },
          { id: "icloud", available: true, reason: null },
        ],
        connections: [],
      }).map((p) => p.id),
    ).toEqual(["icloud", "microsoft"]);
  });

  it("shows each calendar, and says when read only is Niles's rule", async () => {
    card();
    expect(await screen.findByText(/mark@hotmail\.com/)).toBeInTheDocument();
    expect(screen.getByText(/read only is kept by Niles/)).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Allow writing" })).toHaveLength(2);
  });

  it("switches an iCloud calendar's access in place", async () => {
    card();
    const set = vi.spyOn(api, "setAccountServices").mockResolvedValue(MARK.connections[2]);
    const allow = await screen.findAllByRole("button", { name: "Allow writing" });
    fireEvent.click(allow[1]);
    await waitFor(() =>
      expect(set).toHaveBeenCalledWith("cal-3", { calendar: "write", mail: "none" }),
    );
  });

  it("offers an account connected for mail, and leaves it out of the calendars", async () => {
    const mailOnly = {
      ...MARK.connections[2],
      id: "acc-9",
      account: "mark@icloud.com",
      calendar: false,
      mail: "read" as const,
    };
    card({ providers: ALL_AVAILABLE, connections: [mailOnly] });
    const set = vi.spyOn(api, "setAccountServices").mockResolvedValue(mailOnly);
    const use = await screen.findByRole("button", {
      name: /Use mark@icloud\.com, already connected for your mail/,
    });
    expect(screen.queryByRole("button", { name: "Allow writing" })).toBeNull();
    fireEvent.click(use);
    await waitFor(() =>
      expect(set).toHaveBeenCalledWith("acc-9", { calendar: "read", mail: "read" }),
    );
  });

  it("asks before disconnecting", async () => {
    card();
    const remove = vi.spyOn(api, "removeCalendar").mockResolvedValue(undefined);
    fireEvent.click((await screen.findAllByRole("button", { name: "Disconnect" }))[1]);
    expect(remove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Disconnect Work" }));
    await waitFor(() => expect(remove.mock.calls[0]?.[0]).toBe("cal-2"));
  });

  it("connects iCloud with an app-specific password", async () => {
    card({ providers: ALL_AVAILABLE, connections: [] });
    const connect = vi.spyOn(api, "connectIcloud").mockResolvedValue(MARK.connections[2]);
    fireEvent.click(await screen.findByRole("radio", { name: "iCloud" }));
    expect(screen.getByText(/Apple has no read-only password/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Calendar name"), { target: { value: "Phone" } });
    fireEvent.change(screen.getByLabelText("Apple ID"), {
      target: { value: "majse@icloud.com" },
    });
    fireEvent.change(screen.getByLabelText("App-specific password"), {
      target: { value: "abcd-efgh-ijkl-mnop" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect iCloud" }));
    await waitFor(() =>
      expect(connect.mock.calls[0]?.[0]).toEqual({
        name: "Phone",
        calendar: "read",
        mail: "none",
        apple_id: "majse@icloud.com",
        password: "abcd-efgh-ijkl-mnop",
      }),
    );
  });

  it("says why a provider cannot be connected yet", async () => {
    card({
      providers: [
        { id: "microsoft", available: false, reason: "the Microsoft app's keys are missing" },
        { id: "google", available: false, reason: "Google calendars are not switched on" },
        { id: "icloud", available: true, reason: null },
      ],
      connections: [],
    });
    expect(await screen.findByRole("radio", { name: "Microsoft" })).toBeDisabled();
    expect(screen.getByRole("radio", { name: "iCloud" })).toHaveAttribute("aria-checked", "true");
  });
});
