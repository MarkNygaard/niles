import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { SAVE_TICKS_AFTER_MS, TripView } from "@/components/TripView";
import { ArtifactImagesProvider } from "@/lib/artifactImages";
import { kindOf } from "@/lib/artifacts";
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
          cost: { amount: 5400, currency: "DKK" },
        },
      ],
    },
    {
      date: "2027-01-04",
      place: "Bangkok",
      title: "Temples",
      items: [
        {
          time: "09:00",
          type: "activity",
          title: "Grand Palace",
          link: "https://www.royalgrandpalace.th/",
          location: "Na Phra Lan Rd, Bangkok",
          details: "Cover shoulders and knees.",
          cost: { amount: 500, currency: "THB" },
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
  budget: { currency: "THB" },
  notes: "Assumed you **fly** in to Bangkok.",
};

function days() {
  return screen.getByRole("list", { name: "Days" });
}

describe("TripView", () => {
  it("heads the trip with where, when, who and how long until it starts", () => {
    render(<TripView content={TRIP} today="2026-10-16" />);
    expect(screen.getByText("Thailand")).toBeInTheDocument();
    expect(screen.getByText("In 11 weeks")).toBeInTheDocument();
    expect(screen.getByText("3–23 Jan · 21 days")).toBeInTheDocument();
    expect(screen.getByText("Mark & Majse")).toBeInTheDocument();
  });

  it("lists the stays with their nights, booking and reference", () => {
    render(<TripView content={TRIP} today="2026-10-16" />);
    const stays = screen.getByRole("list", { name: "Stays" });
    expect(stays).toHaveTextContent("Riva Surya");
    expect(stays).toHaveTextContent("Bangkok · 3–5 Jan · 2 nights");
    expect(stays).toHaveTextContent("Booked");
    expect(stays).toHaveTextContent("Ref AB123");
  });

  it("opens today's day during the trip and folds the others", () => {
    render(<TripView content={TRIP} today="2027-01-04" />);
    expect(screen.getByText("Day 2 of 21")).toBeInTheDocument();
    const [first, second, third] = within(days()).getAllByRole("button");
    expect(first).toHaveAttribute("aria-expanded", "false");
    expect(second).toHaveAttribute("aria-expanded", "true");
    expect(third).toHaveAttribute("aria-expanded", "false");
    expect(days().querySelector('[aria-current="date"]')).toHaveTextContent("Today");
    expect(screen.queryByText("Land at BKK")).not.toBeInTheDocument();

    fireEvent.click(first);
    expect(screen.getByText("Land at BKK")).toBeInTheDocument();
  });

  it("shows an item's time, kind, link, place on a map, and its cost as an estimate", () => {
    render(<TripView content={TRIP} today="2027-01-04" />);
    const day = screen.getByRole("list", { name: "Mon 4 Jan" });
    expect(within(day).getByText("09:00")).toBeInTheDocument();
    expect(within(day).getByRole("img", { name: "Activity" })).toBeInTheDocument();
    expect(within(day).getByRole("link", { name: /Grand Palace/ })).toHaveAttribute(
      "href",
      "https://www.royalgrandpalace.th/",
    );
    expect(within(day).getByRole("link", { name: /Na Phra Lan Rd/ })).toHaveAttribute(
      "href",
      "https://www.google.com/maps/search/?api=1&query=Na%20Phra%20Lan%20Rd%2C%20Bangkok",
    );
    expect(within(day).getByText("≈ 500 THB")).toBeInTheDocument();
    expect(within(day).getByText("Cover shoulders and knees.")).toBeInTheDocument();
  });

  it("marks what is booked, and gives a booked cost as it is", () => {
    render(<TripView content={TRIP} today="2027-01-03" />);
    const day = screen.getByRole("list", { name: "Sun 3 Jan" });
    expect(within(day).getByText("Booked")).toBeInTheDocument();
    expect(within(day).getByText("5,400 DKK")).toBeInTheDocument();
  });

  it("folds every day once the trip is over", () => {
    render(<TripView content={TRIP} today="2027-02-01" />);
    expect(screen.getByText("Done")).toBeInTheDocument();
    for (const b of within(days()).getAllByRole("button")) {
      expect(b).toHaveAttribute("aria-expanded", "false");
    }
  });

  it("keeps the notes at the end, as markdown, and the costs under the budget", () => {
    render(<TripView content={TRIP} today="2026-10-16" />);
    expect(screen.getByText("fly").tagName).toBe("STRONG");
    expect(screen.getByText("500 THB · 5,400 DKK")).toBeInTheDocument();
  });

  it("says so when the content is not a trip", () => {
    render(<TripView content={{ markdown: "- a" }} today="2026-10-16" />);
    expect(screen.getByText("This trip cannot be read.")).toBeInTheDocument();
  });

  it("is the trip kind's view, copied as text", () => {
    const kind = kindOf("trip");
    expect(kind.label).toBe("Trip");
    expect(kind.text(TRIP)).toContain("Mon 4 Jan · Bangkok · Temples");
  });
});

describe("TripView's packing list", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const box = (name: string) => screen.getByRole("checkbox", { name });

  it("is read only without a way to save", () => {
    render(<TripView content={TRIP} today="2026-10-16" />);
    expect(box("Passport")).toBeChecked();
    expect(box("Sunscreen")).toBeDisabled();
  });

  it("saves a run of ticks as one version, a moment after the last", () => {
    const onChange = vi.fn();
    render(<TripView content={TRIP} onChange={onChange} today="2026-10-16" />);
    expect(screen.getByText("Packing · 1 of 3")).toBeInTheDocument();

    fireEvent.click(box("Sunscreen"));
    fireEvent.click(box("Toothbrush"));
    fireEvent.click(box("Passport"));
    expect(box("Sunscreen")).toBeChecked();
    expect(box("Passport")).not.toBeChecked();
    expect(screen.getByText("Packing · 2 of 3")).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(SAVE_TICKS_AFTER_MS - 1));
    expect(onChange).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(1));
    expect(onChange).toHaveBeenCalledTimes(1);
    const [content, reason] = onChange.mock.calls[0];
    expect((content as Trip).packing!.map((p) => p.packed)).toEqual([true, false, true]);
    expect((content as Trip).days).toEqual(TRIP.days);
    expect(reason).toBe("Packing list: 2 packed, 1 unpacked");
  });

  it("saves nothing when a tick is taken back before it is saved", () => {
    const onChange = vi.fn();
    render(<TripView content={TRIP} onChange={onChange} today="2026-10-16" />);
    fireEvent.click(box("Sunscreen"));
    fireEvent.click(box("Sunscreen"));
    act(() => vi.advanceTimersByTime(SAVE_TICKS_AFTER_MS));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("saves at once when the view goes, rather than losing the ticks", () => {
    const onChange = vi.fn();
    const { unmount } = render(<TripView content={TRIP} onChange={onChange} today="2026-10-16" />);
    fireEvent.click(box("Sunscreen"));
    unmount();
    expect(onChange).toHaveBeenCalledWith(expect.anything(), "Packed “Sunscreen”");
  });

  it("waits for a save on its way before the next", () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <TripView content={TRIP} onChange={onChange} busy today="2026-10-16" />,
    );
    fireEvent.click(box("Sunscreen"));
    act(() => vi.advanceTimersByTime(SAVE_TICKS_AFTER_MS));
    expect(onChange).not.toHaveBeenCalled();
    rerender(<TripView content={TRIP} onChange={onChange} today="2026-10-16" />);
    act(() => vi.advanceTimersByTime(SAVE_TICKS_AFTER_MS));
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("keeps a tick shown until the version it was saved in comes back", () => {
    const onChange = vi.fn();
    const { rerender } = render(<TripView content={TRIP} onChange={onChange} today="2026-10-16" />);
    fireEvent.click(box("Sunscreen"));
    act(() => vi.advanceTimersByTime(SAVE_TICKS_AFTER_MS));
    expect(box("Sunscreen")).toBeChecked();

    const saved = onChange.mock.calls[0][0] as Trip;
    rerender(<TripView content={saved} onChange={onChange} today="2026-10-16" />);
    expect(box("Sunscreen")).toBeChecked();
    // Unticked elsewhere since: the newer version is what shows.
    const elsewhere = {
      ...saved,
      packing: saved.packing!.map((p) => ({ ...p, packed: false })),
    };
    rerender(<TripView content={elsewhere} onChange={onChange} today="2026-10-16" />);
    expect(box("Sunscreen")).not.toBeChecked();
    expect(onChange).toHaveBeenCalledTimes(1);
  });
});

describe("TripView's pictures", () => {
  const picture = (id: number, alt: string, credit: string | null = null) => ({
    id,
    mime: "image/jpeg",
    width: 1600,
    height: 900,
    size: 200_000,
    alt,
    credit,
    source: null,
    created_at: "2026-10-16T10:00:00Z",
  });

  it("shows the trip's, the open day's and an item's, each from Niles", () => {
    const trip: Trip = {
      ...TRIP,
      image: "niles-image:1",
      days: [
        { ...TRIP.days[0], image: "niles-image:2", items: [{ ...TRIP.days[0].items[0], image: "niles-image:3" }] },
        { ...TRIP.days[1], image: "https://tracker.example/day.jpg" },
        TRIP.days[2],
      ],
    };
    const { container } = render(
      <ArtifactImagesProvider
        artifact={5}
        images={[picture(1, "Thailand's coast", "Photo: A. B."), picture(2, "Bangkok at night"), picture(3, "BKK")]}
      >
        <TripView content={trip} today="2026-10-16" />
      </ArtifactImagesProvider>,
    );
    const srcs = [...container.querySelectorAll("img")].map((i) => [
      i.getAttribute("alt"),
      i.getAttribute("src"),
    ]);
    expect(srcs).toEqual([
      ["Thailand's coast", "/me/artifacts/5/images/1"],
      ["Bangkok at night", "/me/artifacts/5/images/2"],
      ["BKK", "/me/artifacts/5/images/3"],
    ]);
    expect(screen.getByText("Photo: A. B.")).toBeInTheDocument();
    fireEvent.click(within(days()).getByRole("button", { name: /Temples/ }));
    expect(document.body.innerHTML).not.toContain("tracker.example");
  });

  it("shows none without the artifact keeping them", () => {
    const { container } = render(
      <TripView content={{ ...TRIP, image: "niles-image:1" }} today="2026-10-16" />,
    );
    expect(container.querySelector("img")).toBeNull();
  });
});
