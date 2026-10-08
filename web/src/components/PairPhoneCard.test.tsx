import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { PairPhoneCard, shouldOffer } from "@/components/PairPhoneCard";
import type { DeviceView } from "@/lib/api";

const UNPAIRED: DeviceView = {
  available: true,
  on_home_network: true,
  mac: "aa:bb:cc:dd:ee:ff",
  name: "Mark's iPhone",
  paired: false,
  has_phone: false,
  signed_in: true,
};

describe("shouldOffer", () => {
  it("offers a phone on the home network that is not yet paired", () => {
    expect(shouldOffer(UNPAIRED)).toBe(true);
  });

  it("offers nothing before the answer arrives", () => {
    // The rule that was asked for: a dashboard that shows this and then
    // withdraws it a moment later has misreported the house, and this is
    // the one thing on the page that appears unprompted.
    expect(shouldOffer(undefined)).toBe(false);
  });

  it("disappears once the phone is paired", () => {
    expect(shouldOffer({ ...UNPAIRED, paired: true })).toBe(false);
  });

  it("is not offered through the tunnel", () => {
    // From outside, the request's address belongs to Cloudflare, and
    // pairing would bind somebody else's device.
    expect(shouldOffer({ ...UNPAIRED, on_home_network: false })).toBe(false);
  });

  it("is not offered when there is no console to ask", () => {
    expect(shouldOffer({ ...UNPAIRED, available: false })).toBe(false);
  });

  it("is not offered when the console cannot see this device", () => {
    expect(shouldOffer({ ...UNPAIRED, mac: null })).toBe(false);
  });

  it("is not offered to nobody", () => {
    expect(shouldOffer({ ...UNPAIRED, signed_in: false })).toBe(false);
  });

  it("is not offered to somebody who already has a phone", () => {
    // From a laptop, or a new phone: one per person, and changing it is
    // done from My profile by unpairing the old one first.
    expect(shouldOffer({ ...UNPAIRED, has_phone: true })).toBe(false);
  });

  it("comes back once the phone is unpaired", () => {
    expect(shouldOffer({ ...UNPAIRED, mac: "11:22:33:44:55:66", has_phone: false })).toBe(true);
  });
});

describe("PairPhoneCard", () => {
  it("renders nothing at all while it does not know", () => {
    const { container } = render(<PairPhoneCard onPair={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("names the phone the console sees", () => {
    render(<PairPhoneCard device={UNPAIRED} onPair={vi.fn()} />);
    expect(screen.getByText(/Mark's iPhone/)).toBeInTheDocument();
  });

  it("pairs on one press", () => {
    const onPair = vi.fn();
    render(<PairPhoneCard device={UNPAIRED} onPair={onPair} />);
    fireEvent.click(screen.getByRole("button", { name: "This is my phone" }));
    expect(onPair).toHaveBeenCalled();
  });
});
