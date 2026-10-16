import { afterEach, describe, expect, it, vi } from "vitest";
import { useRef, useState } from "react";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { History, Sparkles } from "lucide-react";
import { NilesPanel } from "@/components/NilesPanel";
import type { PanelTab } from "@/components/NilesPanel";

/** The viewport, as the panel asks it: a phone or not, and whether
    movement is turned down. jsdom answers no to both (see test-setup). */
function viewport({ phone = false, still = false }) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches:
        (query.includes("max-width") && phone) || (query.includes("reduced-motion") && still),
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

const CONVERSATIONS: PanelTab = {
  id: "conversations",
  label: "Conversations",
  icon: History,
  render: () => <p>The conversations</p>,
};

const SAVED: PanelTab = {
  id: "saved",
  label: "Saved",
  icon: Sparkles,
  render: () => <p>What Niles saved</p>,
};

function Harness({ tabs = [CONVERSATIONS] }: { tabs?: PanelTab[] }) {
  const [open, setOpen] = useState(false);
  const avatar = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button ref={avatar} type="button" onClick={() => setOpen(true)}>
        Open Niles
      </button>
      <NilesPanel
        open={open}
        onOpenChange={setOpen}
        anchor={avatar}
        status="Last talked 5 minutes ago"
        tabs={tabs}
      />
    </>
  );
}

async function openPanel(tabs?: PanelTab[]) {
  render(<Harness tabs={tabs} />);
  fireEvent.click(screen.getByRole("button", { name: "Open Niles" }));
  return screen.findByRole("dialog", { name: "Niles" });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("NilesPanel", () => {
  it("opens as a full-screen drawer on a phone", async () => {
    viewport({ phone: true });
    const panel = await openPanel();
    expect(panel).toHaveAttribute("data-slot", "drawer-popup");
    expect(panel.className).toMatch(/\bh-dvh\b/);
  });

  it("opens as a panel under the avatar on a desktop", async () => {
    viewport({ phone: false });
    const panel = await openPanel();
    expect(panel).toHaveAttribute("data-slot", "niles-panel");
    expect(panel.className).toContain("origin-(--transform-origin)");
  });

  it("carries the name and the status line", async () => {
    const panel = within(await openPanel());
    expect(panel.getByText("Niles")).toBeInTheDocument();
    expect(panel.getByRole("status")).toHaveTextContent("Last talked 5 minutes ago");
  });

  it("closes with the X, and gives focus back to the avatar", async () => {
    const panel = await openPanel();
    fireEvent.click(within(panel).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("button", { name: "Open Niles" })).toHaveFocus();
  });

  it("closes on Escape, and gives focus back to the avatar", async () => {
    const panel = await openPanel();
    fireEvent.keyDown(panel, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("button", { name: "Open Niles" })).toHaveFocus();
  });

  it("closes on Escape on a phone too", async () => {
    viewport({ phone: true });
    const panel = await openPanel();
    fireEvent.keyDown(panel, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("button", { name: "Open Niles" })).toHaveFocus();
  });

  it("grows out of the avatar", async () => {
    viewport({ still: false });
    const moving = await openPanel();
    expect(moving.className).toMatch(/data-starting-style:scale-/);
  });

  it("only fades when movement is turned down, on a desk", async () => {
    viewport({ still: true });
    const panel = await openPanel();
    expect(panel.className).not.toMatch(/scale-|translate/);
    expect(panel.className).toContain("data-starting-style:opacity-0");
  });

  it("only fades when movement is turned down, in a hand", async () => {
    viewport({ phone: true, still: true });
    const panel = await openPanel();
    // The drawer's closed position is where the open one is, so there
    // is nothing for it to slide across.
    expect(panel.className).toContain("[--closed-transform:translate3d(0,0,0)]");
    expect(panel.className).not.toContain("translate3d(0,calc(100%");
    expect(panel.className).toContain("data-starting-style:opacity-0");
  });

  describe("the avatar's flight on a phone", () => {
    // jsdom lays nothing out and has no Web Animations, so both are
    // given: every box 40 square, and an animation that never ends.
    function measurable() {
      vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
        new DOMRect(10, 10, 40, 40),
      );
      const animate = vi.fn(() => ({ cancel: vi.fn() }) as unknown as Animation);
      Object.defineProperty(HTMLElement.prototype, "animate", {
        value: animate,
        configurable: true,
      });
      return animate;
    }
    afterEach(() => {
      vi.restoreAllMocks();
      delete (HTMLElement.prototype as Partial<HTMLElement>).animate;
    });

    it("flies a copy of the avatar into the header, once", async () => {
      viewport({ phone: true });
      const animate = measurable();
      await openPanel();
      expect(document.querySelectorAll("[data-slot=avatar-flight]")).toHaveLength(1);
      expect(animate).toHaveBeenCalledTimes(1);
      // The avatar in the chat is the one that left.
      expect(screen.getByText("Open Niles").style.visibility).toBe("hidden");
    });

    it("stays put when movement is turned down", async () => {
      viewport({ phone: true, still: true });
      const animate = measurable();
      await openPanel();
      expect(document.querySelector("[data-slot=avatar-flight]")).toBeNull();
      expect(animate).not.toHaveBeenCalled();
    });
  });

  it("names its one tab as a heading rather than a bar of one", async () => {
    const panel = within(await openPanel());
    expect(panel.queryByRole("tablist")).toBeNull();
    expect(panel.getByRole("heading", { name: "Conversations" })).toBeInTheDocument();
    expect(panel.getByText("The conversations")).toBeInTheDocument();
  });

  it("shows a bar of icon tabs once there is a second, one entry each", async () => {
    const panel = within(await openPanel([CONVERSATIONS, SAVED]));
    const tabs = within(panel.getByRole("tablist")).getAllByRole("tab");
    expect(tabs.map((t) => t.getAttribute("aria-label"))).toEqual(["Conversations", "Saved"]);
    expect(tabs[1]).toHaveAttribute("title", "Saved");
    expect(panel.getByText("The conversations")).toBeInTheDocument();

    fireEvent.click(panel.getByRole("tab", { name: "Saved" }));
    expect(await panel.findByText("What Niles saved")).toBeInTheDocument();
    expect(panel.queryByText("The conversations")).toBeNull();
  });
});
