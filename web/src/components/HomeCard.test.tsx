import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HomeCard } from "@/components/HomeCard";
import type { HomeValues } from "@/components/HomeCard";
import { api } from "@/lib/api";

// Leaflet draws into a box with a size, and jsdom's boxes have none.
vi.mock("@/components/HomeMap", () => ({ default: () => null }));

const AARHUS = {
  label: "Aarhus, Central Denmark Region, Denmark",
  latitude: 56.1572,
  longitude: 10.2107,
  timezone: "Europe/Copenhagen",
  address: null,
  country_code: "DK",
};

const STREET = {
  label: "Vestergade 12, 8000 Aarhus, Denmark",
  latitude: 56.1567,
  longitude: 10.2039,
  timezone: "Europe/Copenhagen",
  address: "Vestergade 12, 8000 Aarhus",
  country_code: "DK",
};

function setup(values: HomeValues = {}) {
  const onChange = vi.fn();
  const onClear = vi.fn();
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <HomeCard
        values={values}
        timezones={["UTC", "Europe/Copenhagen"]}
        onChange={onChange}
        onClear={onClear}
      />
    </QueryClientProvider>,
  );
  return { onChange, onClear };
}

async function search(term: string) {
  fireEvent.change(screen.getByLabelText("Address or town"), {
    target: { value: term },
  });
  fireEvent.click(screen.getByRole("button", { name: /Find it/ }));
}

describe("HomeCard", () => {
  it("fills the location and the clock from one search", async () => {
    // The point of this service over a plain geocoder: the timezone is
    // the setting that shifts the whole lighting curve, and asking two
    // services the same question is two chances to disagree.
    vi.spyOn(api, "places").mockResolvedValue([AARHUS]);
    const { onChange } = setup();
    await search("Aarhus");
    fireEvent.click(await screen.findByText(AARHUS.label));
    expect(onChange).toHaveBeenCalledWith([
      { path: "home.latitude", value: 56.1572 },
      { path: "home.longitude", value: 10.2107 },
      { path: "home.timezone", value: "Europe/Copenhagen" },
      { path: "home.country", value: "DK" },
    ]);
  });

  it("keeps the address when the search found a street", async () => {
    vi.spyOn(api, "places").mockResolvedValue([STREET]);
    const { onChange } = setup();
    await search("Vestergade 12, Aarhus");
    fireEvent.click(await screen.findByText(STREET.label));
    expect(onChange).toHaveBeenCalledWith([
      { path: "home.latitude", value: 56.1567 },
      { path: "home.longitude", value: 10.2039 },
      { path: "home.timezone", value: "Europe/Copenhagen" },
      { path: "home.country", value: "DK" },
      { path: "home.address", value: "Vestergade 12, 8000 Aarhus" },
    ]);
  });

  it("leaves the timezone alone when it could not be looked up", async () => {
    vi.spyOn(api, "places").mockResolvedValue([{ ...STREET, timezone: null }]);
    const { onChange } = setup();
    await search("Vestergade 12, Aarhus");
    fireEvent.click(await screen.findByText(STREET.label));
    const paths = onChange.mock.calls[0][0].map((e: { path: string }) => e.path);
    expect(paths).not.toContain("home.timezone");
  });

  it("clears the address rather than writing an empty one", () => {
    const { onClear } = setup({ address: "Vestergade 12, 8000 Aarhus" });
    const field = screen.getByLabelText("Address");
    fireEvent.change(field, { target: { value: "  " } });
    fireEvent.blur(field);
    expect(onClear).toHaveBeenCalledWith("home.address");
  });

  it("says nothing matched rather than looking broken", async () => {
    vi.spyOn(api, "places").mockResolvedValue([]);
    setup();
    await search("Nowhereshire");
    expect(await screen.findByText(/Nothing matched/)).toBeInTheDocument();
  });

  it("offers the numbers by hand when the index cannot be reached", async () => {
    vi.spyOn(api, "places").mockRejectedValue(new Error("offline"));
    setup();
    await search("Aarhus");
    await waitFor(() =>
      expect(screen.getByText(/type the numbers in below/)).toBeInTheDocument(),
    );
    expect(screen.getByLabelText("Latitude")).toBeInTheDocument();
  });

  it("says what an unset location costs", () => {
    setup();
    expect(screen.getByText(/Gulf of Guinea/)).toBeInTheDocument();
  });

  it("writes a coordinate when the field is left, not per keystroke", () => {
    const { onChange } = setup({ latitude: 0, longitude: 0 });
    const field = screen.getByLabelText("Latitude");
    fireEvent.change(field, { target: { value: "56.1572" } });
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith([
      { path: "home.latitude", value: 56.1572 },
    ]);
  });

  it("refuses to write a latitude that is not a number", () => {
    const { onChange } = setup({ latitude: 56.1572 });
    const field = screen.getByLabelText("Latitude");
    fireEvent.change(field, { target: { value: "north a bit" } });
    fireEvent.blur(field);
    expect(onChange).not.toHaveBeenCalled();
    expect(field).toHaveValue("56.1572");
  });

  it("clears units rather than writing a null TOML cannot hold", () => {
    const { onChange, onClear } = setup({ units: "metric" });
    fireEvent.click(screen.getByRole("radio", { name: "Metric" }));
    expect(onClear).toHaveBeenCalledWith("home.units");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("picks units that were not set", () => {
    const { onChange } = setup();
    fireEvent.click(screen.getByRole("radio", { name: "Imperial" }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "home.units", value: "imperial" },
    ]);
  });
});
