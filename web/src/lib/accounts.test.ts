import { describe, expect, it } from "vitest";
import { change, connectUrl, outcomeOf, ticksOf } from "@/lib/accounts";
import type { CalendarConnection } from "@/lib/api";

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

describe("accounts", () => {
  it("starts a sign-in for exactly the ticks asked for", () => {
    expect(connectUrl("microsoft", " Work ", { calendar: "read", mail: "none" })).toBe(
      "/calendars/microsoft/start?name=Work&calendar=read&mail=none",
    );
    expect(
      connectUrl("google", "Private", { calendar: "none", mail: "send" }, "acc-1", "mail"),
    ).toBe("/calendars/google/start?name=Private&calendar=none&mail=send&replace=acc-1&from=mail");
  });

  it("reads what an account serves", () => {
    expect(ticksOf(GMAIL)).toEqual({ calendar: "write", mail: "read" });
    expect(ticksOf({ ...GMAIL, calendar: false, mail: null })).toEqual({
      calendar: "none",
      mail: "none",
    });
  });

  it("signs in again to change what Google grants, keeping the other service", () => {
    const how = change(GMAIL, { calendar: "none", mail: "read" }, "calendar");
    expect(how).toEqual({
      kind: "sign_in",
      url: "/calendars/google/start?name=Private&calendar=none&mail=read&replace=acc-1",
    });
  });

  it("changes an iCloud account's ticks in place, and removes one with none left", () => {
    const phone = { ...GMAIL, provider: "icloud" as const };
    expect(change(phone, { calendar: "read", mail: "read" }, "mail")).toEqual({
      kind: "in_place",
      ticks: { calendar: "read", mail: "read" },
    });
    expect(change(GMAIL, { calendar: "none", mail: "none" }, "mail")).toEqual({ kind: "remove" });
  });

  it("hears a sign-in's outcome on the panel it began on", () => {
    expect(outcomeOf("?mail=connected&name=Private", "mail")).toEqual({
      ok: true,
      text: "Private is connected.",
    });
    expect(outcomeOf("?mail=connected&name=Private", "calendar")).toBeNull();
    expect(outcomeOf("?calendar_error=denied", "calendar")).toEqual({ ok: false, text: "denied" });
  });
});
