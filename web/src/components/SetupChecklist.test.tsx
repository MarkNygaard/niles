import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { SetupChecklist } from "@/components/SetupChecklist";
import { gapsFor, pageOfGap, todoByPage } from "@/lib/setup";
import type { SetupGap, SetupReport } from "@/lib/api";

function gap(partial: Partial<SetupGap> = {}): SetupGap {
  return {
    path: "mqtt.host",
    severity: "blocking",
    consequence: "Niles can't reach any lights yet: it has no broker to talk to.",
    ...partial,
  };
}

const HEARING = gap({
  path: "stt.api_key",
  severity: "degraded",
  consequence:
    "Niles can't hear yet: choose and set up a hearing provider. ElevenLabs needs an API key.",
});

const NEW_INSTALL: SetupReport = {
  set_up: false,
  gaps: [
    gap(),
    gap({ path: "mqtt.username", consequence: "The broker will refuse the connection." }),
    HEARING,
    gap({ path: "llm.api_key", severity: "degraded", consequence: "Niles can't think yet." }),
    gap({ path: "home.timezone", severity: "degraded", consequence: "UTC." }),
  ],
};

describe("SetupChecklist", () => {
  it("says nothing when there is nothing to say", () => {
    // A permanent "all good" is one people stop reading, and then it
    // is not there when it changes.
    const { container } = render(<SetupChecklist gaps={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("leads with what stops the house working", () => {
    render(<SetupChecklist gaps={[gap()]} />);
    expect(screen.getByText(/can't reach any lights yet/)).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: /missing something it needs/ }),
    ).toBeInTheDocument();
  });

  it("is quieter about a feature that is off", () => {
    render(<SetupChecklist gaps={[HEARING]} />);
    expect(
      screen.getByRole("heading", { name: "One thing is not set up yet" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/can't hear yet/)).toBeInTheDocument();
  });

  it("counts rather than repeating itself", () => {
    render(<SetupChecklist gaps={[HEARING, { ...HEARING, path: "llm.api_key" }]} />);
    expect(
      screen.getByRole("heading", { name: "2 things are not set up yet" }),
    ).toBeInTheDocument();
  });
});

describe("which page answers a gap", () => {
  it("sends the broker to Devices and the voice to Voice & AI", () => {
    expect(pageOfGap("mqtt.host")).toBe("devices");
    expect(pageOfGap("mqtt.password")).toBe("devices");
    expect(pageOfGap("stt.api_key")).toBe("voice");
    expect(pageOfGap("stt.provider")).toBe("voice");
    expect(pageOfGap("llm.tier2")).toBe("voice");
    expect(pageOfGap("home.latitude")).toBe("home");
  });

  it("shows each page only its own", () => {
    expect(gapsFor(NEW_INSTALL, "devices").map((g) => g.path)).toEqual([
      "mqtt.host",
      "mqtt.username",
    ]);
    expect(gapsFor(NEW_INSTALL, "voice").map((g) => g.path)).toEqual([
      "stt.api_key",
      "llm.api_key",
    ]);
    expect(gapsFor(undefined, "voice")).toEqual([]);
  });

  it("counts them per page for the list beside", () => {
    expect(todoByPage(NEW_INSTALL)).toEqual({ devices: 2, voice: 2, home: 1 });
    expect(todoByPage({ set_up: true, gaps: [] })).toEqual({});
  });
});
