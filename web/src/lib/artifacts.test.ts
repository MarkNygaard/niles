import { describe, expect, it } from "vitest";
import type { Artifact } from "@/lib/api";
import { arrange, kindOf, workingLine } from "@/lib/artifacts";

function artifact(id: number, title: string, rest: Partial<Artifact> = {}): Artifact {
  return {
    id,
    kind: "document",
    title,
    summary: "",
    pinned: false,
    archived: false,
    notify: true,
    status: "ready",
    status_text: null,
    error: null,
    created_at: "2026-10-16T10:00:00Z",
    updated_at: "2026-10-16T10:00:00Z",
    current_revision: 1,
    preview: { lines: [] },
    ...rest,
  };
}

describe("workingLine", () => {
  it("is the newest job's status, with its kind's emoji", () => {
    expect(
      workingLine([
        artifact(1, "Packing list", {
          status: "working",
          status_text: "Writing your packing list…",
          updated_at: "2026-10-16T10:00:00Z",
        }),
        artifact(2, "Thailand travel plan", {
          status: "working",
          status_text: "Writing your Thailand travel plan…",
          updated_at: "2026-10-16T11:00:00Z",
        }),
        artifact(3, "Notes", { updated_at: "2026-10-16T12:00:00Z" }),
      ]),
    ).toBe("✍️ Writing your Thailand travel plan…");
  });

  it("is nothing when nothing is being written", () => {
    expect(workingLine([artifact(1, "Notes")])).toBeUndefined();
    expect(workingLine(undefined)).toBeUndefined();
  });
});

describe("arrange", () => {
  it("puts pinned first, then the rest newest first, and the archived apart", () => {
    const groups = arrange([
      artifact(1, "Old", { updated_at: "2026-10-01T00:00:00Z" }),
      artifact(2, "New", { updated_at: "2026-10-15T00:00:00Z" }),
      artifact(3, "Pinned", { pinned: true, updated_at: "2026-09-01T00:00:00Z" }),
      artifact(4, "Gone", { archived: true, pinned: true }),
    ]);
    expect(groups.pinned.map((a) => a.title)).toEqual(["Pinned"]);
    expect(groups.recent.map((a) => a.title)).toEqual(["New", "Old"]);
    expect(groups.archived.map((a) => a.title)).toEqual(["Gone"]);
  });
});

describe("kindOf", () => {
  it("copies a document as its markdown", () => {
    expect(kindOf("document").text({ markdown: "- a" })).toBe("- a");
  });

  it("has an answer for a kind this app does not know yet", () => {
    expect(kindOf("spreadsheet").label).toBe("Saved");
  });
});
