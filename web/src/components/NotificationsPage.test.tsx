import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { NotificationsPage } from "@/components/NotificationsPage";
import { ApiError, api } from "@/lib/api";
import type { AppNotification, DeviceView, NotificationList } from "@/lib/api";
import type { PlanToday } from "@/lib/trainingPlan";

vi.mock("@/hooks/useDeviceStream", () => ({ useDeviceStream: () => {} }));

const COUNTED: AppNotification = {
  id: 12,
  kind: "workout",
  title: "Hellerup 10K plan",
  body: "Your 8.2 km run on Saturday was counted as the long run.",
  url: "/#/chat?artifact=7",
  created_at: new Date().toISOString(),
  read_at: null,
};

const WAITING: AppNotification = {
  id: 9,
  kind: "approval",
  title: "Let Linear create issue?",
  body: "Title: WLED presets",
  url: "/#/chat?panel=approvals",
  created_at: new Date(Date.now() - 3 * 3_600_000).toISOString(),
  read_at: new Date().toISOString(),
};

const TODAY: PlanToday = {
  id: 7,
  title: "Hellerup 10K plan",
  race: { race: "Hellerup 10K", distance_km: 10, date: "2026-10-25" },
  today: "2026-10-16",
  days_to_go: 9,
  sessions: [
    {
      date: "2026-10-16",
      type: "intervals",
      title: "6 × 400 m",
      distance_km: 7,
      status: "planned",
    },
  ],
  next: null,
  week: { focus: "Build", planned_km: 25, run_km: 6.4, sessions_left: 2 },
  update: null,
};

const UNPAIRED: DeviceView = {
  available: true,
  on_home_network: true,
  mac: "aa:bb:cc:dd:ee:ff",
  name: "Mark's iPhone",
  paired: false,
  has_phone: false,
  signed_in: true,
};

function listed(notifications: AppNotification[], rest: Partial<NotificationList> = {}) {
  const unread = notifications.filter((n) => !n.read_at).length;
  return { notifications, unread, next: null, kept: true, ...rest };
}

function show() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <NotificationsPage />
    </QueryClientProvider>,
  );
}

describe("NotificationsPage", () => {
  beforeEach(() => {
    vi.spyOn(api, "trainingToday").mockResolvedValue({ plan: null });
    vi.spyOn(api, "nemligNext").mockResolvedValue(null);
    vi.spyOn(api, "deviceStatus").mockRejectedValue(new ApiError("no console", 501));
  });
  afterEach(() => vi.restoreAllMocks());

  it("lists what Niles said, newest first, each linking where it is about", async () => {
    vi.spyOn(api, "notifications").mockResolvedValue(listed([COUNTED, WAITING]));
    vi.spyOn(api, "readNotifications").mockResolvedValue({ unread: 0 });
    show();

    const run = await screen.findByRole("link", { name: /counted as the long run/ });
    expect(run).toHaveAttribute("href", "#/chat?artifact=7");
    expect(run).toHaveTextContent("just now");
    const asked = screen.getByRole("link", { name: /Let Linear create issue/ });
    expect(asked).toHaveAttribute("href", "#/chat?panel=approvals");
    expect(asked).toHaveTextContent("3 hours ago");
    const links = screen.getAllByRole("link");
    expect(links.indexOf(run)).toBeLessThan(links.indexOf(asked));
  });

  it("reads what is there when it opens, and keeps the dot on what was new", async () => {
    vi.spyOn(api, "notifications")
      .mockResolvedValueOnce(listed([COUNTED, WAITING]))
      .mockResolvedValue(listed([{ ...COUNTED, read_at: new Date().toISOString() }, WAITING]));
    const read = vi.spyOn(api, "readNotifications").mockResolvedValue({ unread: 0 });
    show();

    await waitFor(() => expect(read).toHaveBeenCalledWith(12));
    expect(read).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("img", { name: "New" })).toBeInTheDocument();
    expect(screen.getAllByRole("img", { name: "New" })).toHaveLength(1);
  });

  it("asks nothing to be read when nothing is unread", async () => {
    vi.spyOn(api, "notifications").mockResolvedValue(listed([WAITING]));
    const read = vi.spyOn(api, "readNotifications").mockResolvedValue({ unread: 0 });
    show();
    await screen.findByRole("link", { name: /Let Linear create issue/ });
    expect(read).not.toHaveBeenCalled();
  });

  it("says so when Niles keeps no history", async () => {
    vi.spyOn(api, "notifications").mockResolvedValue(listed([], { kept: false }));
    vi.spyOn(api, "trainingToday").mockResolvedValue({ plan: TODAY });
    show();
    expect(await screen.findByText(/no database/)).toBeInTheDocument();
    expect(await screen.findByText("6 × 400 m")).toBeInTheDocument();
  });

  it("puts today's run, the groceries and pairing the phone under Now", async () => {
    vi.spyOn(api, "notifications").mockResolvedValue(listed([]));
    vi.spyOn(api, "trainingToday").mockResolvedValue({ plan: TODAY });
    const today = new Date().toLocaleDateString("sv-SE");
    vi.spyOn(api, "nemligNext").mockResolvedValue({
      id: 1,
      status: 1,
      total: 512,
      delivery_start: `${today}T17:00:00`,
      delivery_end: `${today}T18:00:00`,
    });
    vi.spyOn(api, "deviceStatus").mockResolvedValue(UNPAIRED);
    show();

    expect(await screen.findByRole("heading", { name: "Now" })).toBeInTheDocument();
    expect(await screen.findByText("6 × 400 m")).toBeInTheDocument();
    expect(await screen.findByText(/Groceries arrive today/)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "This is my phone" })).toBeInTheDocument();
  });

  it("has no Now with nothing for now", async () => {
    vi.spyOn(api, "notifications").mockResolvedValue(listed([]));
    show();
    expect(await screen.findByText(/Nothing yet/)).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Now" })).toBeNull();
  });
});
