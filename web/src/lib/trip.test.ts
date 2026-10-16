import { describe, expect, it } from "vitest";
import {
  costTotals,
  countdown,
  mapsLink,
  money,
  nights,
  openDay,
  packingGroups,
  packingKey,
  packingReason,
  span,
  travellers,
  tripOf,
  tripText,
  unsaved,
  withTicks,
} from "@/lib/trip";
import type { Trip } from "@/lib/trip";

const TRIP: Trip = {
  destination: "Thailand",
  start: "2027-01-03",
  end: "2027-01-23",
  travellers: ["Mark", "Majse"],
  days: [
    {
      date: "2027-01-03",
      place: "Bangkok",
      title: "Arrive",
      items: [
        {
          time: "07:05",
          type: "travel",
          title: "Land at BKK",
          booked: true,
          cost: { amount: 5400.5, currency: "DKK" },
        },
        {
          time: "19:30",
          type: "food",
          title: "Dinner at Jay Fai",
          location: "327 Maha Chai Rd, Bangkok",
          cost: { amount: 1500, currency: "THB" },
        },
      ],
    },
    {
      date: "2027-01-04",
      place: "Bangkok",
      items: [
        {
          time: "09:00",
          type: "activity",
          title: "Grand Palace",
          link: "https://www.royalgrandpalace.th/",
          cost: { amount: 1000, currency: "THB" },
        },
      ],
    },
    { date: "2027-01-06", place: "Chiang Mai", items: [] },
  ],
  stays: [
    {
      name: "Riva Surya",
      place: "Bangkok",
      check_in: "2027-01-03",
      check_out: "2027-01-05",
      booked: true,
      confirmation: "AB123",
    },
  ],
  packing: [
    { item: "Sunscreen", group: "Toiletries", packed: false },
    { item: "Passport", group: "Documents", packed: true },
    { item: "Toothbrush", group: "Toiletries", packed: false },
  ],
  budget: { currency: "THB", notes: "About 3,000 a day for two." },
  notes: "Assumed you fly in to Bangkok.",
};

describe("countdown", () => {
  it("counts down to the first day, in weeks and then days", () => {
    expect(countdown(TRIP, "2026-10-16")).toBe("In 11 weeks");
    expect(countdown(TRIP, "2026-12-25")).toBe("In 9 days");
    expect(countdown(TRIP, "2027-01-02")).toBe("Tomorrow");
  });

  it("says which day of the trip it is, and when it is over", () => {
    expect(countdown(TRIP, "2027-01-03")).toBe("Day 1 of 21");
    expect(countdown(TRIP, "2027-01-06")).toBe("Day 4 of 21");
    expect(countdown(TRIP, "2027-01-23")).toBe("Day 21 of 21");
    expect(countdown(TRIP, "2027-01-24")).toBe("Done");
  });
});

describe("openDay", () => {
  it("is today's day during the trip, or the one before it on a day with nothing", () => {
    expect(openDay(TRIP, "2027-01-04")).toBe(1);
    expect(openDay(TRIP, "2027-01-05")).toBe(1);
    expect(openDay(TRIP, "2027-01-20")).toBe(2);
  });

  it("is the first before the trip and none after it", () => {
    expect(openDay(TRIP, "2026-10-16")).toBe(0);
    expect(openDay(TRIP, "2027-02-01")).toBe(-1);
  });
});

describe("labels", () => {
  it("writes a span of days within a month and across one", () => {
    expect(span("2027-01-03", "2027-01-23")).toBe("3–23 Jan");
    expect(span("2027-01-30", "2027-02-02")).toBe("30 Jan – 2 Feb");
    expect(span("2026-12-28", "2027-01-04")).toBe("28 Dec – 4 Jan");
    expect(span("2027-01-03", "2027-01-03")).toBe("3 Jan");
  });

  it("counts a stay's nights and names the travellers", () => {
    expect(nights(TRIP.stays![0])).toBe("2 nights");
    expect(travellers(["Mark"])).toBe("Mark");
    expect(travellers(["Mark", "Majse", "Ida"])).toBe("Mark, Majse & Ida");
  });

  it("writes money with its currency, and decimals only when there are some", () => {
    expect(money(1500, "THB")).toBe("1,500 THB");
    expect(money(5400.5, "DKK")).toBe("5,400.50 DKK");
  });

  it("searches a map for a location", () => {
    expect(mapsLink("327 Maha Chai Rd, Bangkok")).toBe(
      "https://www.google.com/maps/search/?api=1&query=327%20Maha%20Chai%20Rd%2C%20Bangkok",
    );
  });
});

describe("costTotals", () => {
  it("adds the costs up per currency, the budget's first", () => {
    expect(costTotals(TRIP)).toEqual([
      { currency: "THB", amount: 2500 },
      { currency: "DKK", amount: 5400.5 },
    ]);
  });
});

describe("packing", () => {
  it("keeps the groups in the order they first appear", () => {
    const groups = packingGroups(TRIP.packing!).map(([g, items]) => [g, items.map((p) => p.item)]);
    expect(groups).toEqual([
      ["Toiletries", ["Sunscreen", "Toothbrush"]],
      ["Documents", ["Passport"]],
    ]);
  });

  it("puts ticks on the list, and knows which of them are not saved yet", () => {
    const ticks = new Map([
      [packingKey({ item: "Sunscreen", group: "Toiletries" }), true],
      [packingKey({ item: "passport", group: "documents" }), true],
    ]);
    expect(withTicks(TRIP, ticks).packing!.map((p) => p.packed)).toEqual([true, true, false]);
    expect(unsaved(TRIP, ticks).map((p) => p.item)).toEqual(["Sunscreen"]);
  });

  it("says what a save of ticks did", () => {
    const p = (item: string, packed: boolean) => ({ item, packed });
    expect(packingReason([p("Sunscreen", true)])).toBe("Packed “Sunscreen”");
    expect(packingReason([p("Hat", false)])).toBe("Unpacked “Hat”");
    expect(packingReason([p("a", true), p("b", true)])).toBe("Packed 2 things");
    expect(packingReason([p("a", true), p("b", false), p("c", true)])).toBe(
      "Packing list: 2 packed, 1 unpacked",
    );
  });
});

describe("tripOf", () => {
  it("reads a trip and refuses what is not one", () => {
    expect(tripOf(TRIP)?.destination).toBe("Thailand");
    expect(tripOf({ markdown: "- a" })).toBeUndefined();
    expect(tripOf(null)).toBeUndefined();
  });
});

describe("tripText", () => {
  it("is the whole trip as text, day by day", () => {
    const text = tripText(TRIP);
    expect(text).toContain("Thailand · 3–23 Jan · 21 days\nMark & Majse");
    expect(text).toContain("- Riva Surya, Bangkok: 3–5 Jan (2 nights), booked, ref AB123");
    expect(text).toContain("Sun 3 Jan · Bangkok · Arrive\n- 07:05 Land at BKK (5,400.50 DKK, booked)");
    expect(text).toContain("- 09:00 Grand Palace (1,000 THB)\n  https://www.royalgrandpalace.th/");
    expect(text).toContain("Documents:\n- [x] Passport");
  });
});
