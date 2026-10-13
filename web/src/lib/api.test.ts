import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";

function answer(status: number, body: string | null) {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(body, { status })));
}

const rule = {
  description: "",
  enabled: true,
  triggers: [{ type: "contact" as const, device: "office/door", open: true }],
  conditions: [],
  actions: [],
};

describe("request", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("passes on a refusal given as plain text", async () => {
    answer(400, "an automation needs at least one action");
    await expect(api.createAutomation(rule)).rejects.toThrow(
      "an automation needs at least one action",
    );
  });

  it("passes on a refusal given as {error}", async () => {
    answer(400, JSON.stringify({ error: "brightness must be 0..=100" }));
    await expect(api.createAutomation(rule)).rejects.toThrow("brightness must be 0..=100");
  });

  it("passes on a 501 from an instance with no skill store", async () => {
    answer(501, "this instance has no skill store");
    await expect(api.skills()).rejects.toThrow("this instance has no skill store");
  });

  it("takes an empty 204 as done", async () => {
    answer(204, null);
    await expect(api.updateSkill("evening-wind-down", { pinned: true })).resolves.toBeNull();
  });

  it("says only the status for a proxy's error page", async () => {
    answer(502, "<html><body>Bad gateway</body></html>");
    await expect(api.automations()).rejects.toThrow("request failed (502)");
  });
});
