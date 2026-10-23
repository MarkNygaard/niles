/**
 * Files the app hands the person: an artifact as Word or Markdown, made
 * here from its content, never from its page.
 */

/** The kinds a Word copy is made of: the ones written as text. */
export const WORD_KINDS: ReadonlySet<string> = new Set(["document", "training_plan", "trip"]);

/** Characters no file system takes in a name, and control codes. */
const UNSAFE = /[/\\:*?"<>|\u0000-\u001f]/g;

/**
 * A file name from an artifact's title: "Thailand trip.docx", or
 * "Thailand trip (version 3).docx" for a version read in place of the
 * current one.
 */
export function fileName(title: string, extension: string, version?: number): string {
  const base =
    title
      .replace(UNSAFE, " ")
      .replace(/\s+/g, " ")
      .trim()
      .replace(/^\.+/, "")
      .slice(0, 100)
      .trim() || "Untitled";
  return `${version === undefined ? base : `${base} (version ${version})`}.${extension}`;
}

/** Hand `blob` to the browser to save as `name`. */
export function saveFile(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.rel = "noopener";
  document.body.append(link);
  link.click();
  link.remove();
  // Revoked later, not at once: Safari reads the blob after the click
  // returns.
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
