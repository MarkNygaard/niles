import { describe, expect, it } from "vitest";
import type { Artifact } from "@/lib/api";
import {
  NO_LONGER_HERE,
  canEdit,
  changedNote,
  choicesFor,
  isOwner,
  levelOf,
  personLabel,
  sharingNote,
} from "@/lib/sharing";

function artifact(rest: Partial<Artifact> = {}): Artifact {
  return {
    id: 1,
    kind: "trip",
    title: "Thailand trip",
    summary: "",
    pinned: false,
    archived: false,
    notify: true,
    status: "ready",
    status_text: null,
    error: null,
    created_at: "",
    updated_at: "",
    current_revision: 1,
    preview: { lines: [] },
    ...rest,
  };
}

describe("sharing", () => {
  it("lets the owner and an editor change it, and a reader only read it", () => {
    expect([isOwner(artifact()), canEdit(artifact())]).toEqual([true, true]);
    expect([isOwner(artifact({ access: "edit" })), canEdit(artifact({ access: "edit" }))]).toEqual(
      [false, true],
    );
    expect(canEdit(artifact({ access: "view" }))).toBe(false);
  });

  it("offers everyone in the household every level, and someone gone only a stop", () => {
    expect(choicesFor({ person: "person:majse", name: "Majse", level: null })).toEqual([
      "none",
      "view",
      "edit",
    ]);
    const gone = { person: "person:gone", name: null, level: "edit" as const };
    expect(choicesFor(gone)).toEqual(["none", "edit"]);
    expect(personLabel(gone)).toBe(NO_LONGER_HERE);
    expect(levelOf("none")).toBeNull();
    expect(levelOf("view")).toBe("view");
  });

  it("says whose it is, and who changed it", () => {
    expect(sharingNote(artifact())).toBeUndefined();
    expect(sharingNote(artifact({ access: "owner", shared_with: 2 }))).toBe("Shared with 2");
    expect(sharingNote(artifact({ access: "edit", owner: "Mark" }))).toBe("Shared by Mark");
    expect(sharingNote(artifact({ access: "view", owner: "Mark" }))).toBe(
      "Shared by Mark · read only",
    );
    expect(sharingNote(artifact({ access: "edit", owner: null }))).toBe("Shared with you");
    expect(changedNote(artifact({ changed_by: "Majse" }))).toBe("changed by Majse");
    expect(changedNote(artifact())).toBeUndefined();
  });
});
