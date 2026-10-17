import { describe, expect, it } from "vitest";
import { ACCEPT, accept, filesFrom, isImage, MAX_BYTES, MAX_FILES, readable } from "./attachments";

/** A File of a size and type, since jsdom has no picker to make one. */
function file(name: string, type: string, size = 1024, lastModified = 1): File {
  return new File([new Uint8Array(size)], name, { type, lastModified });
}

describe("isImage", () => {
  it("is any image MIME type", () => {
    expect(isImage({ type: "image/jpeg" })).toBe(true);
    expect(isImage({ type: "image/heic" })).toBe(true);
    expect(isImage({ type: "application/pdf" })).toBe(false);
    expect(isImage({ type: "text/plain" })).toBe(false);
    expect(isImage({ type: "" })).toBe(false);
  });
});

describe("readable", () => {
  it("sizes in bytes, kilobytes and megabytes", () => {
    expect(readable(512)).toBe("512 B");
    expect(readable(2048)).toBe("2.0 KB");
    expect(readable(10 * 1024 * 1024)).toBe("10.0 MB");
  });
});

describe("filesFrom", () => {
  it("keeps images and PDFs out of a drop", () => {
    const data = {
      files: [
        file("p.jpg", "image/jpeg"),
        file("t.pdf", "application/pdf"),
        file("n.txt", "text/plain"),
      ],
    } as unknown as DataTransfer;
    const got = filesFrom(data);
    expect(got.map((f) => f.name)).toEqual(["p.jpg", "t.pdf"]);
  });

  it("is nothing without data", () => {
    expect(filesFrom(null)).toEqual([]);
    expect(filesFrom(undefined)).toEqual([]);
  });
});

describe("accept", () => {
  it("takes a JPEG and a PDF", () => {
    const { files, error } = accept(
      [file("p.jpg", "image/jpeg", 2 * 1024 * 1024), file("t.pdf", "application/pdf")],
      [],
    );
    expect(files.map((f) => f.name)).toEqual(["p.jpg", "t.pdf"]);
    expect(error).toBeUndefined();
  });

  it("refuses the sixth file", () => {
    const five = Array.from({ length: MAX_FILES }, (_, i) =>
      file(`p${i}.jpg`, "image/jpeg"),
    );
    const { files, error } = accept([file("sixth.jpg", "image/jpeg")], five);
    expect(files).toHaveLength(MAX_FILES);
    expect(error).toBe("Five files at most in one message.");
  });

  it("refuses a file over 10 MB, naming its size", () => {
    const big = file("big.jpg", "image/jpeg", MAX_BYTES + 1);
    const { files, error } = accept([big], []);
    expect(files).toEqual([]);
    expect(error).toContain("big.jpg");
    expect(error).toContain("10.0 MB");
    expect(error).toContain("10 MB");
  });

  it("refuses a file that is not a photo or a PDF", () => {
    const { files, error } = accept([file("notes.txt", "text/plain")], []);
    expect(files).toEqual([]);
    expect(error).toBe("notes.txt is not a photo or a PDF.");
  });

  it("keeps the good ones when one in the batch is bad", () => {
    const good = file("p.jpg", "image/jpeg");
    const { files, error } = accept([good, file("notes.txt", "text/plain")], []);
    expect(files).toEqual([good]);
    expect(error).toBe("notes.txt is not a photo or a PDF.");
  });

  it("reports the first reason when several are wrong", () => {
    const { error } = accept(
      [file("notes.txt", "text/plain"), file("big.jpg", "image/jpeg", MAX_BYTES + 1)],
      [],
    );
    expect(error).toBe("notes.txt is not a photo or a PDF.");
  });

  it("de-duplicates the same file chosen twice", () => {
    const same = file("p.jpg", "image/jpeg", 1024, 42);
    const again = file("p.jpg", "image/jpeg", 1024, 42);
    const { files } = accept([same, again], []);
    expect(files).toEqual([same]);
  });

  it("keeps what was already there, in order", () => {
    const old = file("old.jpg", "image/jpeg");
    const next = file("new.jpg", "image/jpeg");
    const { files } = accept([next], [old]);
    expect(files).toEqual([old, next]);
  });

  it("lets a HEIC through for the server to refuse", () => {
    // The client does not second-guess what the server can decode:
    // its refusal names the iPhone setting to change.
    const heic = file("photo.heic", "image/heic");
    const { files, error } = accept([heic], []);
    expect(files).toEqual([heic]);
    expect(error).toBeUndefined();
  });
});

describe("ACCEPT", () => {
  it("offers photos and PDFs, including HEIC so an iPhone shows them", () => {
    for (const t of ["image/jpeg", "image/png", "image/webp", "image/gif", "image/heic", "application/pdf"]) {
      expect(ACCEPT).toContain(t);
    }
  });
});
