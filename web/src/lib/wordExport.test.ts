// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import JSZip from "jszip";
import { Paragraph, Table } from "docx";
import type { ArtifactImage } from "@/lib/api";
import { fileName } from "@/lib/downloads";
import { Lists, fit, loadPictures, markdownBody, picturesOf, wordFile } from "@/lib/wordExport";
import type { Picture } from "@/lib/wordExport";

const PALACE: ArtifactImage = {
  id: 12,
  mime: "image/png",
  width: 1600,
  height: 900,
  size: 100,
  alt: "The Grand Palace",
  credit: "J. Doe",
  source: "https://www.example.com/palace",
  created_at: "2026-10-01T00:00:00Z",
};

/** The smallest PNG there is: one transparent pixel. */
const PNG = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  ),
  (c) => c.charCodeAt(0),
);

const DOC = [
  "# Packing",
  "",
  "Read **this** and [the guide](https://example.com/guide).",
  "",
  "- Passport",
  "- Charger",
  "",
  "1. Book",
  "2. Pay",
  "",
  "- [ ] Sunscreen",
  "- [x] Towel",
  "",
  "| Day | What |",
  "|---|---|",
  "| Mon | Beach |",
  "",
  "![The palace](niles-image:12)",
  "",
  "![A tracker](https://evil.example.com/pixel.png)",
].join("\n");

async function documentXml(blob: Blob): Promise<string> {
  const zip = await JSZip.loadAsync(await blob.arrayBuffer());
  return zip.file("word/document.xml")!.async("string");
}

async function coreXml(blob: Blob): Promise<string> {
  const zip = await JSZip.loadAsync(await blob.arrayBuffer());
  return zip.file("docProps/core.xml")!.async("string");
}

const picture = (): Picture => ({ image: PALACE, type: "png", data: PNG });

describe("a document as Word", () => {
  afterEach(() => vi.restoreAllMocks());

  it("maps each block of the markdown to a paragraph or a table", () => {
    const ctx = { pictures: new Map([[12, picture()]]), lists: new Lists() };
    const body = markdownBody(DOC, ctx);
    const shape = body.map((c) =>
      c instanceof Table ? "table" : c instanceof Paragraph ? "p" : "?",
    );
    expect(shape).toEqual([
      "p", // heading
      "p", // paragraph
      "p", // bullet
      "p", // bullet
      "p", // number
      "p", // number
      "p", // unticked
      "p", // ticked
      "table",
      "p", // the picture
      "p", // its credit
      "p", // the tracker, as its alt
    ]);
    // The bullets, and one numbering of its own for the numbered list.
    expect(ctx.lists.config.map((c) => c.reference)).toEqual(["bullets", "numbered-1"]);
  });

  it("writes headings, lists, ticks, a table, links and the kept picture", async () => {
    const loadImage = vi.fn(async () => new Blob([PNG], { type: "image/png" }));
    const fetched = vi.spyOn(globalThis, "fetch");
    const blob = await wordFile({
      kind: "document",
      title: "Cabin trip",
      content: { markdown: DOC },
      images: [PALACE],
      loadImage,
    });
    const xml = await documentXml(blob);
    expect(xml).toContain('w:val="Title"');
    expect(xml).toContain('w:val="Heading1"');
    expect(xml).toContain("<w:numPr>");
    expect(xml).toContain("☐ ");
    expect(xml).toContain("☑ ");
    expect(xml).toContain("<w:tbl>");
    expect(xml).toContain("Beach");
    expect(xml).toContain("<w:hyperlink");
    expect(xml).toContain("<w:b/>");
    expect(xml).toContain("<w:drawing>");
    expect(xml).toContain('descr="The palace"');
    expect(xml).toContain("J. Doe · example.com");
    expect(xml).toContain("A tracker");
    expect(await coreXml(blob)).toContain("<dc:title>Cabin trip</dc:title>");

    // Pictures come from Niles alone: the tracker is never asked for.
    expect(loadImage).toHaveBeenCalledTimes(1);
    expect(loadImage).toHaveBeenCalledWith(12);
    expect(fetched).not.toHaveBeenCalled();
  });

  it("leaves out a picture that does not load", async () => {
    const blob = await wordFile({
      kind: "document",
      title: "Cabin",
      content: { markdown: "Before\n\n![The palace](niles-image:12)\n\nAfter" },
      images: [PALACE],
      loadImage: async () => {
        throw new Error("404");
      },
    });
    const xml = await documentXml(blob);
    expect(xml).toContain("Before");
    expect(xml).toContain("After");
    expect(xml).not.toContain("<w:drawing>");
  });

  it("asks only for the pictures the artifact keeps", async () => {
    const loadImage = vi.fn(async () => new Blob([PNG]));
    const got = await loadPictures(
      ["niles-image:12", "niles-image:12", "niles-image:99", "https://example.com/a.png"],
      [PALACE],
      loadImage,
    );
    expect([...got.keys()]).toEqual([12]);
    expect(loadImage).toHaveBeenCalledTimes(1);
  });

  it("scales a picture down to the page, never up", () => {
    expect(fit({ width: 1600, height: 900 }, 600, 800)).toEqual({ width: 600, height: 338 });
    expect(fit({ width: 300, height: 200 }, 600, 800)).toEqual({ width: 300, height: 200 });
    expect(fit({ width: 1000, height: 4000 }, 600, 800)).toEqual({ width: 200, height: 800 });
  });
});

const PLAN = {
  goal: { race: "Copenhagen Half", distance_km: 21.1, date: "2026-09-13", target_time: "1:45" },
  weeks: [
    {
      starts: "2026-08-31",
      focus: "Base",
      sessions: [
        {
          date: "2026-09-01",
          type: "easy",
          title: "Easy run",
          distance_km: 6,
          status: "done",
          actual: { distance_km: 6.2, pace: "5:41" },
        },
        {
          date: "2026-09-03",
          type: "intervals",
          title: "Track 6×800",
          details: "Warm up first",
          status: "planned",
        },
      ],
    },
    { starts: "2026-09-07", sessions: [] },
  ],
  notes: "Sleep well.",
  updates: [
    { date: "2026-09-02", note: "Nice and easy." },
    { date: "2026-08-30", note: "An older note." },
  ],
};

const TRIP = {
  destination: "Thailand",
  start: "2027-02-01",
  end: "2027-02-14",
  travellers: ["Mark", "Majse"],
  image: "niles-image:12",
  days: [
    {
      date: "2027-02-01",
      place: "Bangkok",
      title: "Arrival",
      items: [
        {
          time: "09:00",
          type: "travel",
          title: "Flight in",
          booked: true,
          cost: { amount: 5400, currency: "DKK" },
        },
        {
          type: "activity",
          title: "Grand Palace",
          location: "Grand Palace, Bangkok",
          cost: { amount: 500, currency: "THB" },
        },
      ],
    },
  ],
  stays: [
    {
      name: "Riva Surya",
      place: "Bangkok",
      check_in: "2027-02-01",
      check_out: "2027-02-04",
      booked: true,
      confirmation: "AB12",
    },
  ],
  packing: [
    { item: "Sunscreen", group: "Toiletries", packed: true },
    { item: "Passport", packed: false },
  ],
  budget: { currency: "THB", notes: "About 2,000 a day" },
  notes: "Get a SIM at the airport.",
};

describe("a plan and a trip as Word", () => {
  it("writes a plan: the race, the coach's newest note, the weeks and their sessions", async () => {
    const blob = await wordFile({
      kind: "training_plan",
      title: "Half plan",
      content: PLAN,
      loadImage: async () => new Blob(),
    });
    expect(blob.size).toBeGreaterThan(0);
    const xml = await documentXml(blob);
    for (const words of [
      "Copenhagen Half",
      "Sun 13 Sep · 21.1 km · goal 1:45",
      "From your coach",
      "Nice and easy.",
      "Week 1",
      "Week 2",
      "Nothing planned.",
      "Track 6×800",
      "Warm up first",
      "Intervals",
      "Done ✓ 6.2 km at 5:41",
      "Planned",
      "Sleep well.",
    ]) {
      expect(xml).toContain(words);
    }
    expect(xml).not.toContain("An older note.");
    expect(xml).toContain("<w:tbl>");
  });

  it("writes a trip: dates, travellers, stays, days, the packing list with its ticks, budget, notes and pictures", async () => {
    const loadImage = vi.fn(async () => new Blob([PNG]));
    const blob = await wordFile({
      kind: "trip",
      title: "Thailand",
      content: TRIP,
      images: [PALACE],
      loadImage,
    });
    expect(blob.size).toBeGreaterThan(0);
    const xml = await documentXml(blob);
    for (const words of [
      "1–14 Feb · 14 days",
      "Travellers: Mark &amp; Majse",
      "Riva Surya",
      "Ref AB12",
      "Mon 1 Feb · Bangkok",
      "Arrival",
      "09:00",
      "Flight in",
      "5,400 DKK",
      "≈ 500 THB",
      "Grand Palace, Bangkok",
      "Packing · 1 of 2",
      "Toiletries",
      "☑ ",
      "☐ ",
      "Budget · THB",
      "About 2,000 a day",
      "Get a SIM at the airport.",
    ]) {
      expect(xml).toContain(words);
    }
    expect(xml).toContain("<w:drawing>");
    expect(loadImage).toHaveBeenCalledWith(12);
  });

  it("finds a trip's pictures wherever they are", () => {
    expect(
      picturesOf("trip", {
        ...TRIP,
        days: [
          {
            ...TRIP.days[0],
            image: "niles-image:3",
            items: [{ type: "food", title: "x", image: "niles-image:4" }],
          },
        ],
        notes: "![map](niles-image:5)",
      }),
    ).toEqual(["niles-image:12", "niles-image:3", "niles-image:4", "niles-image:5"]);
  });
});

describe("file names", () => {
  it("are the title, made safe, and say which version", () => {
    expect(fileName("Thailand trip", "docx")).toBe("Thailand trip.docx");
    expect(fileName('a/b\\c:d*e?f"g<h>i|j', "md")).toBe("a b c d e f g h i j.md");
    expect(fileName("Thailand trip", "docx", 3)).toBe("Thailand trip (version 3).docx");
    expect(fileName("  ", "md")).toBe("Untitled.md");
  });
});
