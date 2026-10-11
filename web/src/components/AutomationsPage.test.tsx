import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AutomationsPage } from "./AutomationsPage";
import { api } from "@/lib/api";
import type { AutomationView, Rule } from "@/lib/api";

const garden: Rule = {
  id: "4f1c",
  description: "Garden door",
  enabled: true,
  trigger: { type: "contact", device: "office/door", open: true },
  conditions: [],
  actions: [{ do: "set_room", room: "office", on: false }],
};

const stored: AutomationView = {
  rule: garden,
  source: "stored",
  sentence: "When the office door opens, turn off the office lights.",
  error: null,
};

const fromConfig: AutomationView = {
  rule: {
    id: "config-0",
    description: "",
    enabled: true,
    trigger: { type: "presence", state: "away" },
    conditions: [],
    actions: [{ do: "set_room", room: "hall", on: false }],
  },
  source: "config",
  sentence: "When everybody leaves, turn off the hall lights.",
  error: null,
};

function show(rules: AutomationView[]) {
  vi.spyOn(api, "automations").mockResolvedValue({ rules });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <AutomationsPage />
    </QueryClientProvider>,
  );
}

describe("AutomationsPage", () => {
  afterEach(() => vi.restoreAllMocks());

  it("lists each rule as its sentence, under its name", async () => {
    show([stored]);
    expect(await screen.findByText(stored.sentence)).toBeInTheDocument();
    expect(screen.getByText("Garden door")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /office door opens/ })).toHaveAttribute(
      "href",
      "#/automations/4f1c",
    );
  });

  it("pauses a rule with its switch", async () => {
    show([stored]);
    // Never answers: what the switch shows meanwhile is the point.
    const update = vi.spyOn(api, "updateAutomation").mockReturnValue(new Promise(() => {}));
    const toggle = await screen.findByRole("switch", { name: "Run Garden door" });
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    await waitFor(() => expect(update).toHaveBeenCalledWith({ ...garden, enabled: false }));
    expect(screen.getByRole("switch", { name: "Run Garden door" })).not.toBeChecked();
  });

  it("marks a rule from the config, and offers it no switch or link", async () => {
    show([fromConfig]);
    expect(await screen.findByText(fromConfig.sentence)).toBeInTheDocument();
    expect(screen.getByText("From config")).toBeInTheDocument();
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.queryByRole("link", { name: /everybody leaves/ })).toBeNull();
  });

  it("says why a rule is not running", async () => {
    show([{ ...stored, error: "office/door is not a door or window" }]);
    expect(await screen.findByText("office/door is not a door or window")).toBeInTheDocument();
  });

  it("says a paused rule is paused", async () => {
    show([{ ...stored, rule: { ...garden, enabled: false } }]);
    expect(await screen.findByText("Paused")).toBeInTheDocument();
  });

  it("explains, with nothing yet, that Niles can be asked", async () => {
    show([]);
    expect(await screen.findByText(/Niles, when the office door opens/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "New automation" })).toHaveAttribute(
      "href",
      "#/automations/new",
    );
  });
});
