/** Every rule the chat's attachment UI applies, as plain functions the
    tests drive directly. What the picker offers, what a drop or a paste
    may add, and how a refusal reads. */

export const MAX_FILES = 5;
export const MAX_BYTES = 10 * 1024 * 1024;
/** What the picker offers. HEIC is left out on purpose: an iPhone
    converts a photo to a type the list names, so its photos arrive as
    JPEG, which the server can read, rather than HEIC, which it cannot.
    The photo library and the camera are still offered. A HEIC dropped
    on a desktop passes `accept` below and is refused by the server with
    the setting to change; the client does not second-guess what the
    server can decode. */
export const ACCEPT = "image/jpeg,image/png,image/webp,image/gif,application/pdf";

/** What the accept list takes, by MIME type. */
export function isImage(file: { type: string }): boolean {
  return file.type.startsWith("image/");
}

/** "2.4 MB" — one decimal over a kilobyte, bytes below. */
export function readable(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

/** The files in a drop or a paste, images and PDFs only. */
export function filesFrom(data: DataTransfer | null | undefined): File[] {
  if (!data) return [];
  return Array.from(data.files).filter((f) => isImage(f) || f.type === "application/pdf");
}

/** What may be added to `already`, and why the rest may not. The first
    reason wins; the files that are fine are kept either way. */
export function accept(
  chosen: File[],
  already: File[],
): { files: File[]; error?: string } {
  const files = [...already];
  let error: string | undefined;
  for (const file of chosen) {
    if (files.some((f) => f.name === file.name && f.size === file.size && f.lastModified === file.lastModified)) {
      continue;
    }
    if (files.length >= MAX_FILES) {
      error ??= "Five files at most in one message.";
      continue;
    }
    if (file.size > MAX_BYTES) {
      error ??= `${file.name} is ${readable(file.size)} — the most a file can be is 10 MB.`;
      continue;
    }
    if (!isImage(file) && file.type !== "application/pdf") {
      error ??= `${file.name} is not a photo or a PDF.`;
      continue;
    }
    files.push(file);
  }
  return { files, error };
}
