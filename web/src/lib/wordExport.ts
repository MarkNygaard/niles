import {
  AlignmentType,
  BorderStyle,
  Document,
  ExternalHyperlink,
  HeadingLevel,
  ImageRun,
  LevelFormat,
  Packer,
  Paragraph,
  ShadingType,
  Table,
  TableCell,
  TableRow,
  TextRun,
  WidthType,
} from "docx";
import type { FileChild, ILevelsOptions, INumberingOptions, ParagraphChild } from "docx";
import type { ArtifactImage } from "@/lib/api";
import { imageId, sourceHost } from "@/lib/artifactImages";
import { markdownOf } from "@/lib/artifacts";
import { parseInline, parseMarkdown, safeHref } from "@/lib/markdown";
import type { Block, Inline } from "@/lib/markdown";
import {
  TYPE_LABELS,
  dayLabel as planDay,
  km,
  measures,
  outcome,
  planOf,
  weekRange,
  weekTotals,
} from "@/lib/trainingPlan";
import type { Plan, Session } from "@/lib/trainingPlan";
import {
  ITEM_LABELS,
  costTotals,
  dayLabel,
  days,
  mapsLink,
  money,
  nights,
  packingGroups,
  span,
  travellers,
  tripLength,
  tripOf,
} from "@/lib/trip";
import type { Item, Stay, Trip } from "@/lib/trip";

/**
 * An artifact as a Word document, made from its content the way its
 * kind's view reads it: never from its page. Loaded only when someone
 * downloads one, so `docx` stays out of the app's first load.
 *
 * Pictures are the artifact's own, fetched from Niles by `loadImage`; a
 * picture by its web address is never fetched (see `lib/markdown.tsx`),
 * and one that does not load is left out.
 */
export interface WordSource {
  kind: string;
  title: string;
  content: unknown;
  /** The pictures the artifact keeps. */
  images?: ArtifactImage[];
  loadImage: (image: number) => Promise<Blob>;
}

/** A kept picture, fetched. */
export interface Picture {
  data: Uint8Array;
  type: "jpg" | "png" | "gif";
  image: ArtifactImage;
}

const TYPES: Record<string, Picture["type"]> = {
  "image/jpeg": "jpg",
  "image/png": "png",
  "image/gif": "gif",
};

/** In pixels at 96 per inch: inside the margins of both A4 and Letter. */
const PAGE_WIDTH = 600;
const PAGE_HEIGHT = 800;
/** A picture within a line, as small as the view draws it. */
const INLINE_HEIGHT = 160;
/** A table across the page, in twentieths of a point. */
const TABLE_WIDTH = 9000;
const MONO = "Courier New";
const GREY = "666666";

const HEADINGS = [
  HeadingLevel.HEADING_1,
  HeadingLevel.HEADING_2,
  HeadingLevel.HEADING_3,
  HeadingLevel.HEADING_4,
  HeadingLevel.HEADING_5,
  HeadingLevel.HEADING_6,
];

const LIST_INDENT = { left: 720, hanging: 360 };

function level(format: ILevelsOptions["format"], text: string, start = 1): ILevelsOptions {
  return {
    level: 0,
    format,
    text,
    start,
    alignment: AlignmentType.LEFT,
    style: { paragraph: { indent: LIST_INDENT } },
  };
}

/** The lists of one document: one bullet style, and a numbering of its
    own for each numbered list, so each counts from its own start. */
export class Lists {
  readonly config: { reference: string; levels: ILevelsOptions[] }[] = [
    { reference: "bullets", levels: [level(LevelFormat.BULLET, "•")] },
  ];

  numbered(start: number): string {
    const reference = `numbered-${this.config.length}`;
    this.config.push({ reference, levels: [level(LevelFormat.DECIMAL, "%1.", start)] });
    return reference;
  }

  get numbering(): INumberingOptions {
    return { config: this.config };
  }
}

export interface Context {
  pictures: ReadonlyMap<number, Picture>;
  lists: Lists;
}

interface Style {
  bold?: boolean;
  italics?: boolean;
  link?: boolean;
}

function text(
  value: string,
  style: Style = {},
  extra: { font?: string; color?: string } = {},
): TextRun {
  return new TextRun({
    text: value,
    bold: style.bold,
    italics: style.italics,
    style: style.link ? "Hyperlink" : undefined,
    ...extra,
  });
}

/** Text that may run over several lines, each break kept. */
function lines(value: string, style: Style = {}, extra: { color?: string } = {}): TextRun[] {
  return value.split("\n").map(
    (line, i) =>
      new TextRun({
        text: line,
        bold: style.bold,
        italics: style.italics,
        break: i > 0 ? 1 : undefined,
        ...extra,
      }),
  );
}

/** A picture's size, scaled down to fit within `width` by `height`. */
export function fit(image: { width: number; height: number }, width: number, height: number) {
  const w = image.width > 0 ? image.width : width;
  const h = image.height > 0 ? image.height : height;
  const scale = Math.min(1, width / w, height / h);
  return { width: Math.round(w * scale), height: Math.round(h * scale) };
}

function imageRun(picture: Picture, alt: string, width: number, height: number): ImageRun {
  const description = alt || picture.image.alt;
  return new ImageRun({
    type: picture.type,
    data: picture.data,
    transformation: fit(picture.image, width, height),
    altText: { name: `Picture ${picture.image.id}`, description, title: description },
  });
}

/** The words of a line as runs: bold, italic, code, links that may be
    followed, and kept pictures small within it. */
export function runs(parts: Inline[], ctx: Context, style: Style = {}): ParagraphChild[] {
  return parts.flatMap((part): ParagraphChild[] => {
    switch (part.type) {
      case "text":
        return [text(part.text, style)];
      case "code":
        return [text(part.text, style, { font: MONO })];
      case "strong":
        return runs(part.children, ctx, { ...style, bold: true });
      case "em":
        return runs(part.children, ctx, { ...style, italics: true });
      case "link": {
        const safe = safeHref(part.href);
        if (!safe) return runs(part.children, ctx, style);
        return [
          new ExternalHyperlink({
            link: safe,
            children: runs(part.children, ctx, { ...style, link: true }),
          }),
        ];
      }
      case "image": {
        const id = imageId(part.src);
        const picture = id === undefined ? undefined : ctx.pictures.get(id);
        if (picture) return [imageRun(picture, part.alt, PAGE_WIDTH, INLINE_HEIGHT)];
        return part.alt ? [text(part.alt, { ...style, italics: true })] : [];
      }
    }
  });
}

function line(source: string, ctx: Context, style: Style = {}): ParagraphChild[] {
  return runs(parseInline(source), ctx, style);
}

/** A kept picture across the page, with its credit under it; nothing
    for one that did not load. */
export function picture(name: string | undefined, alt: string, ctx: Context): Paragraph[] {
  const id = imageId(name);
  const kept = id === undefined ? undefined : ctx.pictures.get(id);
  if (!kept) return [];
  const out = [new Paragraph({ children: [imageRun(kept, alt, PAGE_WIDTH, PAGE_HEIGHT)] })];
  const host = sourceHost(kept.image.source);
  const credit = [kept.image.credit, host].filter(Boolean).join(" · ");
  if (credit) out.push(new Paragraph({ children: [text(credit, {}, { color: GREY })] }));
  return out;
}

function table(head: string[], rows: string[][], ctx: Context): Table {
  const columns = Math.max(head.length, ...rows.map((r) => r.length), 1);
  const cell = (source: string | undefined, style: Style) =>
    new TableCell({ children: [new Paragraph({ children: line(source ?? "", ctx, style) })] });
  const row = (cells: string[], style: Style, header = false) =>
    new TableRow({
      tableHeader: header,
      children: Array.from({ length: columns }, (_, i) => cell(cells[i], style)),
    });
  return new Table({
    width: { size: TABLE_WIDTH, type: WidthType.DXA },
    columnWidths: Array.from({ length: columns }, () => Math.floor(TABLE_WIDTH / columns)),
    rows: [row(head, { bold: true }, true), ...rows.map((r) => row(r, {}))],
  });
}

function block(b: Block, ctx: Context): FileChild[] {
  switch (b.type) {
    case "heading":
      return [new Paragraph({ heading: HEADINGS[b.level - 1], children: line(b.text, ctx) })];
    case "paragraph":
      return [new Paragraph({ children: line(b.text, ctx) })];
    case "quote":
      return [
        new Paragraph({
          children: line(b.text, ctx, { italics: true }),
          indent: { left: 720 },
          border: {
            left: { style: BorderStyle.SINGLE, size: 12, color: "BBBBBB", space: 8 },
          },
        }),
      ];
    case "code":
      return [
        new Paragraph({
          children: b.text.split("\n").map(
            (l, i) =>
              new TextRun({
                text: l,
                font: MONO,
                size: 20,
                break: i > 0 ? 1 : undefined,
              }),
          ),
          shading: { type: ShadingType.CLEAR, fill: "F2F2F2", color: "auto" },
        }),
      ];
    case "rule":
      return [
        new Paragraph({
          children: [],
          border: {
            bottom: { style: BorderStyle.SINGLE, size: 6, color: "BBBBBB", space: 1 },
          },
        }),
      ];
    case "image":
      if (imageId(b.src) !== undefined) return picture(b.src, b.alt, ctx);
      return b.alt ? [new Paragraph({ children: [text(b.alt, { italics: true })] })] : [];
    case "table":
      return [table(b.head, b.rows, ctx)];
    case "list": {
      const reference = b.ordered ? ctx.lists.numbered(b.start) : "bullets";
      return b.items.map((item) =>
        item.checked === undefined
          ? new Paragraph({
              children: line(item.text, ctx),
              numbering: { reference, level: 0 },
            })
          : checkItem(item.checked, line(item.text, ctx)),
      );
    }
  }
}

function checkItem(checked: boolean, children: ParagraphChild[]): Paragraph {
  return new Paragraph({
    children: [text(checked ? "☑ " : "☐ "), ...children],
    indent: { left: 360 },
  });
}

/** A document's markdown, block by block. */
export function markdownBody(source: string, ctx: Context): FileChild[] {
  return parseMarkdown(source).flatMap((b) => block(b, ctx));
}

function heading(value: string, n: 1 | 2 | 3 = 1): Paragraph {
  return new Paragraph({ heading: HEADINGS[n - 1], children: [text(value)] });
}

function plain(value: string, style: Style = {}, extra: { color?: string } = {}): Paragraph {
  return new Paragraph({ children: lines(value, style, extra) });
}

function joined(parts: (string | false | undefined | null)[]): string {
  return parts.filter(Boolean).join(" · ");
}

function sessionCell(s: Session): TableCell {
  return new TableCell({
    children: [
      plain(s.title, { bold: true }),
      ...(s.details ? [plain(s.details, {}, { color: GREY })] : []),
    ],
  });
}

function sessions(list: Session[]): Table {
  const head = ["Day", "Session", "Type", "Planned", "Status"];
  const widths = [1500, 3300, 1200, 1700, 1300];
  const cell = (value: string, style: Style = {}) =>
    new TableCell({ children: [plain(value, style)] });
  return new Table({
    width: { size: TABLE_WIDTH, type: WidthType.DXA },
    columnWidths: widths,
    rows: [
      new TableRow({ tableHeader: true, children: head.map((h) => cell(h, { bold: true })) }),
      ...list.map(
        (s) =>
          new TableRow({
            children: [
              cell(planDay(s.date)),
              sessionCell(s),
              cell(TYPE_LABELS[s.type] ?? s.type),
              cell(measures(s)),
              cell(outcome(s) ?? "Planned"),
            ],
          }),
      ),
    ],
  });
}

/** A training plan: the race, the coach's newest note, the weeks with
    their sessions, and the notes. */
export function planBody(plan: Plan, ctx: Context): FileChild[] {
  const g = plan.goal;
  const out: FileChild[] = [
    plain(g.race, { bold: true }),
    plain(joined([planDay(g.date), km(g.distance_km), g.target_time && `goal ${g.target_time}`])),
  ];
  const newest = plan.updates?.[0];
  if (newest) {
    out.push(
      heading("From your coach", 2),
      plain(planDay(newest.date), {}, { color: GREY }),
      plain(newest.note),
    );
  }
  plan.weeks.forEach((w, i) => {
    const total = weekTotals(w).planned;
    out.push(
      heading(`Week ${i + 1}`, 2),
      plain(joined([weekRange(w), w.focus, total > 0 && km(total)])),
    );
    out.push(w.sessions.length === 0 ? plain("Nothing planned.") : sessions(w.sessions));
  });
  if (plan.notes) out.push(heading("Notes", 2), ...markdownBody(plan.notes, ctx));
  return out;
}

function linked(title: string, link: string | undefined, style: Style): ParagraphChild {
  const safe = link && safeHref(link);
  return safe
    ? new ExternalHyperlink({ link: safe, children: [text(title, { ...style, link: true })] })
    : text(title, style);
}

function stay(s: Stay): Paragraph {
  const tail = joined([s.booked && "Booked", s.confirmation && `Ref ${s.confirmation}`]);
  return new Paragraph({
    numbering: { reference: "bullets", level: 0 },
    children: [
      linked(s.name, s.link, { bold: true }),
      text(
        `, ${s.place}: ${span(s.check_in, s.check_out)} (${nights(s)})${tail ? ` · ${tail}` : ""}`,
      ),
    ],
  });
}

function item(i: Item, ctx: Context): Paragraph[] {
  const head: ParagraphChild[] = [];
  if (i.time) head.push(text(`${i.time}  `, { bold: true }));
  head.push(
    text(`${ITEM_LABELS[i.type] ?? i.type}: `, { italics: true }),
    linked(i.title, i.link, { bold: true }),
  );
  if (i.booked) head.push(text(" · Booked"));
  const out = [new Paragraph({ children: head })];
  const indent = { left: 360 };
  if (i.details) out.push(new Paragraph({ children: lines(i.details), indent }));
  const where: ParagraphChild[] = [];
  if (i.location)
    where.push(
      new ExternalHyperlink({
        link: mapsLink(i.location),
        children: [text(i.location, { link: true })],
      }),
    );
  if (i.cost) {
    if (where.length) where.push(text(" · "));
    where.push(text(`${i.booked ? "" : "≈ "}${money(i.cost.amount, i.cost.currency)}`));
  }
  if (where.length) out.push(new Paragraph({ children: where, indent }));
  out.push(...picture(i.image, "", ctx));
  return out;
}

/** A trip: where and when, who, the stays, each day with its plan, the
    packing list with its ticks, the budget and the notes. */
export function tripBody(trip: Trip, ctx: Context): FileChild[] {
  const out: FileChild[] = [...picture(trip.image, "", ctx)];
  out.push(
    plain(trip.destination, { bold: true }),
    plain(`${span(trip.start, trip.end)} · ${days(tripLength(trip))}`),
  );
  if (trip.travellers?.length) out.push(plain(`Travellers: ${travellers(trip.travellers)}`));
  if (trip.stays?.length) out.push(heading("Stays", 2), ...trip.stays.map(stay));
  for (const d of trip.days) {
    out.push(heading(joined([dayLabel(d.date), d.place]), 2));
    if (d.title) out.push(plain(d.title, { italics: true }));
    out.push(...picture(d.image, "", ctx));
    if (d.items.length === 0) out.push(plain("Nothing planned."));
    for (const i of d.items) out.push(...item(i, ctx));
  }
  if (trip.packing?.length) {
    const packed = trip.packing.filter((p) => p.packed).length;
    out.push(heading(`Packing · ${packed} of ${trip.packing.length}`, 2));
    for (const [group, items] of packingGroups(trip.packing)) {
      if (group) out.push(heading(group, 3));
      out.push(...items.map((p) => checkItem(p.packed, [text(p.item)])));
    }
  }
  if (trip.budget) {
    out.push(heading(`Budget · ${trip.budget.currency}`, 2));
    const totals = costTotals(trip);
    if (totals.length) {
      out.push(
        plain(`Costs listed: ${totals.map((t) => money(t.amount, t.currency)).join(" · ")}`),
      );
    }
    if (trip.budget.notes) out.push(plain(trip.budget.notes));
  }
  if (trip.notes) out.push(heading("Notes", 2), ...markdownBody(trip.notes, ctx));
  return out;
}

function inlinePictures(parts: Inline[], into: string[]): void {
  for (const part of parts) {
    if (part.type === "image") into.push(part.src);
    else if ("children" in part) inlinePictures(part.children, into);
  }
}

/** Every picture a piece of markdown names, inline or on its own line. */
export function markdownPictures(source: string): string[] {
  const names: string[] = [];
  const of = (t: string) => inlinePictures(parseInline(t), names);
  for (const b of parseMarkdown(source)) {
    switch (b.type) {
      case "image":
        names.push(b.src);
        break;
      case "heading":
      case "paragraph":
      case "quote":
        of(b.text);
        break;
      case "list":
        b.items.forEach((i) => of(i.text));
        break;
      case "table":
        [b.head, ...b.rows].flat().forEach(of);
        break;
    }
  }
  return names;
}

/** The pictures the content names, by `niles-image:N` or by address. */
export function picturesOf(kind: string, content: unknown): string[] {
  if (kind === "document") return markdownPictures(markdownOf(content));
  if (kind === "training_plan") return markdownPictures(planOf(content)?.notes ?? "");
  if (kind === "trip") {
    const trip = tripOf(content);
    if (!trip) return [];
    return [
      trip.image,
      ...trip.days.flatMap((d) => [d.image, ...d.items.map((i) => i.image)]),
      ...markdownPictures(trip.notes ?? ""),
    ].filter((n): n is string => Boolean(n));
  }
  return [];
}

/**
 * The pictures `names` name that the artifact keeps, fetched through
 * Niles. Only a whole `niles-image:N` is a name: a web address in their
 * place is never asked for. One that fails to load is left out.
 */
export async function loadPictures(
  names: string[],
  images: ArtifactImage[] | undefined,
  loadImage: WordSource["loadImage"],
): Promise<Map<number, Picture>> {
  const kept = new Map((images ?? []).map((i) => [i.id, i]));
  const wanted = [...new Set(names.map(imageId))].flatMap((id) => {
    const image = id === undefined ? undefined : kept.get(id);
    const type = image && TYPES[image.mime];
    return image && type ? [{ image, type }] : [];
  });
  const loaded = await Promise.allSettled(
    wanted.map(async ({ image, type }): Promise<Picture> => {
      const blob = await loadImage(image.id);
      return { image, type, data: new Uint8Array(await blob.arrayBuffer()) };
    }),
  );
  return new Map(
    loaded.flatMap((r) => (r.status === "fulfilled" ? [[r.value.image.id, r.value] as const] : [])),
  );
}

/** What goes under the title, for each kind a Word copy is made of. */
export function bodyOf(kind: string, content: unknown, ctx: Context): FileChild[] {
  if (kind === "training_plan") {
    const plan = planOf(content);
    return plan ? planBody(plan, ctx) : [plain("This plan cannot be read.")];
  }
  if (kind === "trip") {
    const trip = tripOf(content);
    return trip ? tripBody(trip, ctx) : [plain("This trip cannot be read.")];
  }
  return markdownBody(markdownOf(content), ctx);
}

/** The artifact as a .docx, its title the document's. */
export async function wordFile(source: WordSource): Promise<Blob> {
  const pictures = await loadPictures(
    picturesOf(source.kind, source.content),
    source.images,
    source.loadImage,
  );
  const ctx: Context = { pictures, lists: new Lists() };
  const children = [
    new Paragraph({ heading: HeadingLevel.TITLE, children: [text(source.title)] }),
    ...bodyOf(source.kind, source.content, ctx),
  ];
  const doc = new Document({
    title: source.title,
    creator: "Niles",
    styles: {
      default: {
        document: {
          run: { font: "Calibri", size: 22 },
          paragraph: { spacing: { after: 120 } },
        },
      },
    },
    numbering: ctx.lists.numbering,
    sections: [{ children }],
  });
  return Packer.toBlob(doc);
}
