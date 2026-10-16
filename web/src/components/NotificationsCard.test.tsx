import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { NotificationsCard, testedLine } from "@/components/NotificationsCard";
import type { NotificationsCardProps } from "@/components/NotificationsCard";
import type { PushDevice } from "@/lib/api";

const IPHONE: PushDevice = {
  endpoint: "https://web.push.apple.com/abc",
  device: "iPhone",
  created_at: new Date(Date.now() - 2 * 86_400_000).toISOString(),
  last_ok_at: null,
};
const LAPTOP: PushDevice = {
  endpoint: "https://fcm.googleapis.com/fcm/send/xyz",
  device: "Chrome on Mac",
  created_at: new Date(Date.now() - 86_400_000).toISOString(),
  last_ok_at: new Date().toISOString(),
};

function card(rest: Partial<NotificationsCardProps> = {}) {
  const props: NotificationsCardProps = {
    support: "available",
    denied: "Turn them on in Settings → Notifications → Niles.",
    on: false,
    canAsk: true,
    devices: [],
    onChange: vi.fn(),
    onRemove: vi.fn(),
    onTest: vi.fn(),
    ...rest,
  };
  render(<NotificationsCard {...props} />);
  return props;
}

describe("NotificationsCard", () => {
  it("turns this device on from its switch", () => {
    const props = card();
    fireEvent.click(screen.getByRole("switch"));
    expect(props.onChange).toHaveBeenCalledWith(true);
  });

  it("turns this device off from its switch", () => {
    const props = card({ on: true, devices: [IPHONE], here: IPHONE.endpoint });
    expect(screen.getByRole("switch")).toBeChecked();
    fireEvent.click(screen.getByRole("switch"));
    expect(props.onChange).toHaveBeenCalledWith(false);
  });

  it("waits for the key before the switch can ask", () => {
    card({ canAsk: false });
    expect(screen.getByRole("switch")).toHaveAttribute("aria-disabled", "true");
  });

  it("explains the Home Screen on an iPhone in Safari, with no switch", () => {
    card({ support: "needs-home-screen" });
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    expect(screen.getByText(/Add to Home Screen/)).toBeInTheDocument();
    expect(screen.getByText(/iOS 16.4/)).toBeInTheDocument();
  });

  it("says where to allow them again after no", () => {
    card({ support: "denied" });
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    expect(screen.getByText(/Settings → Notifications → Niles/)).toBeInTheDocument();
  });

  it("says why when the server cannot send", () => {
    card({ unavailable: "notifications need Niles's database", devices: undefined });
    expect(screen.getByText(/need Niles's database/)).toBeInTheDocument();
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
  });

  it("lists your devices, marks this one, and removes one", () => {
    const props = card({ on: true, devices: [LAPTOP, IPHONE], here: IPHONE.endpoint });
    expect(screen.getByText(/· this device/)).toBeInTheDocument();
    expect(screen.getByText(/last sent/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove Chrome on Mac" }));
    expect(props.onRemove).toHaveBeenCalledWith(LAPTOP.endpoint);
  });

  it("sends a test and says how it went", () => {
    const props = card({ devices: [IPHONE], tested: { sent: 1, gone: 0, failed: 0 } });
    fireEvent.click(screen.getByRole("button", { name: "Send a test" }));
    expect(props.onTest).toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent("Sent to 1 device.");
  });

  it("offers no test with no device to send to", () => {
    card({ devices: [] });
    expect(screen.queryByRole("button", { name: "Send a test" })).not.toBeInTheDocument();
  });
});

describe("testedLine", () => {
  it("counts what was sent and what was not", () => {
    expect(testedLine({ sent: 2, gone: 0, failed: 1 })).toBe("Sent to 2 devices. 1 didn't take it.");
  });

  it("says when every device had gone", () => {
    expect(testedLine({ sent: 0, gone: 2, failed: 0 })).toMatch(/removed/);
  });
});
