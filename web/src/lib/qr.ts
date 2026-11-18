import jsQR from "jsqr";

/** What a QR code read for the authenticator field: the link to keep,
    or why it can't be used. */
export type QrRead = { link: string } | { refused: string };

/** An authenticator's setup QR holds an `otpauth://totp/...` link. Only
    that is kept; anything else says why not. */
export function authenticatorFrom(text: string): QrRead {
  const read = text.trim();
  const lower = read.toLowerCase();
  if (lower.startsWith("otpauth-migration://")) {
    return {
      refused:
        "That's an export from an authenticator app, which Niles can't read yet. Scan the site's own setup QR code instead.",
    };
  }
  if (lower.startsWith("otpauth://hotp/")) {
    return {
      refused: "That code counts sign-ins instead of time, which Niles doesn't support.",
    };
  }
  if (lower.startsWith("otpauth://totp/")) return { link: read };
  return { refused: "That QR code isn't an authenticator setup code." };
}

/** The text of the QR code in a picture's pixels, or null. */
export function decodePixels(data: Uint8ClampedArray, width: number, height: number) {
  return jsQR(data, width, height, { inversionAttempts: "attemptBoth" })?.data ?? null;
}

/** The text of the QR code in an image file or a pasted screenshot, read
    on this device: the picture goes nowhere. Null when none is found. */
export async function decodeImage(file: Blob): Promise<string | null> {
  const bitmap = await createImageBitmap(file);
  // A screenshot of a whole screen is large; a QR code reads at a
  // fraction of that, and smaller is faster.
  const scale = Math.min(1, 1600 / Math.max(bitmap.width, bitmap.height));
  const width = Math.max(1, Math.round(bitmap.width * scale));
  const height = Math.max(1, Math.round(bitmap.height * scale));
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return null;
  context.drawImage(bitmap, 0, 0, width, height);
  bitmap.close();
  return decodePixels(context.getImageData(0, 0, width, height).data, width, height);
}
