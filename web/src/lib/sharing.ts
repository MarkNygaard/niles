import type { Artifact, ArtifactAccess, ArtifactSharee, ShareLevel } from "@/lib/api";

/** What the share dialog offers each person: not shared, or one of the
    two levels. */
export type ShareChoice = ShareLevel | "none";

export const CHOICE_LABELS: Record<ShareChoice, string> = {
  none: "None",
  view: "View",
  edit: "Edit",
};

/** What a Niles older than sharing sends nothing for: everything is
    the signed-in person's own. */
function accessOf(a: { access?: ArtifactAccess }): ArtifactAccess {
  return a.access ?? "owner";
}

export function isOwner(a: { access?: ArtifactAccess }): boolean {
  return accessOf(a) === "owner";
}

/** Whether they may change it: theirs, or shared with them to edit. */
export function canEdit(a: { access?: ArtifactAccess }): boolean {
  return accessOf(a) !== "view";
}

export const NO_LONGER_HERE = "Someone no longer in the household";

/** The name the dialog shows for someone. */
export function personLabel(s: ArtifactSharee): string {
  return s.name ?? NO_LONGER_HERE;
}

/**
 * What the dialog offers for one person. Someone no longer in the
 * household cannot be shared with again, only let go: they keep what
 * they have until the owner stops it.
 */
export function choicesFor(s: ArtifactSharee): ShareChoice[] {
  if (s.name === null) return s.level ? ["none", s.level] : ["none"];
  return ["none", "view", "edit"];
}

/** What a choice sends: a level, or null to stop. */
export function levelOf(choice: ShareChoice): ShareLevel | null {
  return choice === "none" ? null : choice;
}

/**
 * The line a row in Saved, or an open artifact, says about whose it is:
 * "Shared by Mark", "Shared by Mark · read only", or for its owner
 * "Shared with 2". Nothing for one of their own nobody else has.
 */
export function sharingNote(a: Artifact): string | undefined {
  const access = accessOf(a);
  if (access === "owner") {
    const n = a.shared_with ?? 0;
    return n > 0 ? `Shared with ${n}` : undefined;
  }
  const by = a.owner ? `Shared by ${a.owner}` : "Shared with you";
  return access === "view" ? `${by} · read only` : by;
}

/** "changed by Majse", when someone other than them saved it last. */
export function changedNote(a: Artifact): string | undefined {
  return a.changed_by ? `changed by ${a.changed_by}` : undefined;
}
