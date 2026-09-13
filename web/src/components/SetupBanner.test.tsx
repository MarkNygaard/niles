import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { SetupBanner } from "@/components/SetupBanner";
import type { SetupGap } from "@/lib/api";

function gap(partial: Partial<SetupGap> = {}): SetupGap {
  return {
    path: "mqtt.host",
    severity: "blocking",
    consequence: "Niles cannot reach any lights.",
    ...partial,
  };
}

describe("SetupBanner", () => {
  it("says nothing when there is nothing to say", () => {
    // A permanent "all good" banner is one people stop reading, and
    // then it is not there when it changes.
    const { container } = render(
      <SetupBanner report={{ set_up: true, gaps: [] }} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("leads with what stops the house working", () => {
    render(
      <SetupBanner report={{ set_up: false, gaps: [gap()] }} />,
    );
    expect(screen.getByText(/cannot reach any lights/)).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: /missing something it needs/ }),
    ).toBeInTheDocument();
  });

  it("keeps a feature being off apart from a house that does not work", () => {
    // Run together, neither reads as urgent. The severity is the whole
    // reason the server sends one.
    render(
      <SetupBanner
        report={{
          set_up: false,
          gaps: [
            gap(),
            gap({
              path: "llm.api_key_env",
              severity: "degraded",
              consequence: "Anything a regex cannot answer goes unanswered.",
            }),
          ],
        }}
      />,
    );
    expect(
      screen.getByRole("heading", { name: /missing something it needs/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: /One thing is not set up yet/ }),
    ).toBeInTheDocument();
  });

  it("counts rather than repeating itself", () => {
    render(
      <SetupBanner
        report={{
          set_up: false,
          gaps: [gap(), gap({ path: "mqtt.username_env" })],
        }}
      />,
    );
    expect(
      screen.getByRole("heading", { name: /missing 2 things/ }),
    ).toBeInTheDocument();
  });
});
