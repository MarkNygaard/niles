import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { CalendarsCard, connectUrl, outcomeOf } from "./CalendarsCard";
import { redirectUri } from "./MicrosoftPanel";
import { api } from "@/lib/api";
import type { CalendarsView } from "@/lib/api";

const MARK: CalendarsView = {
  available: true,
  reason: null,
  connections: [
    { id: "cal-1", name: "Private", account: "mark@hotmail.com", access: "write" },
    { id: "cal-2", name: "Work", account: "mark@work.example", access: "read" },
  ],
};

function card(view: CalendarsView = MARK) {
  vi.spyOn(api, "calendars").mockResolvedValue(view);
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <CalendarsCard />
    </QueryClientProvider>,
  );
}

describe("CalendarsCard", () => {
  afterEach(() => vi.restoreAllMocks());

  it("starts Microsoft's sign-in with the name and access chosen", () => {
    expect(connectUrl(" Work ", "read")).toBe("/calendars/microsoft/start?name=Work&access=read");
    expect(connectUrl("Work", "write", "cal-2")).toBe(
      "/calendars/microsoft/start?name=Work&access=write&replace=cal-2",
    );
  });

  it("says how the sign-in went", () => {
    expect(outcomeOf("?calendar=connected&name=Work")).toEqual({
      ok: true,
      text: "Work is connected.",
    });
    expect(outcomeOf("?calendar_error=AADSTS65001%3A%20admin%20approval")?.ok).toBe(false);
    expect(outcomeOf("")).toBeNull();
  });

  it("names the redirect the app registration needs", () => {
    expect(redirectUri("https://niles.mnygaard.io")).toBe(
      "https://niles.mnygaard.io/calendars/microsoft/callback",
    );
  });

  it("shows each calendar with what Niles may do with it", async () => {
    card();
    expect(await screen.findByText("mark@hotmail.com")).toBeInTheDocument();
    // The badge, and the choice offered when connecting another.
    expect(screen.getAllByText("Read and write")).toHaveLength(2);
    expect(screen.getAllByText("Read only")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Allow writing" })).toBeInTheDocument();
  });

  it("asks before disconnecting", async () => {
    card();
    const remove = vi.spyOn(api, "removeCalendar").mockResolvedValue(undefined);
    fireEvent.click((await screen.findAllByRole("button", { name: "Disconnect" }))[1]);
    expect(remove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Disconnect Work" }));
    await waitFor(() => expect(remove.mock.calls[0]?.[0]).toBe("cal-2"));
  });

  it("says why nothing can be connected yet", async () => {
    card({ available: false, reason: "the Microsoft app's keys are missing", connections: [] });
    expect(await screen.findByText("the Microsoft app's keys are missing")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect with Microsoft" })).toBeNull();
  });
});
