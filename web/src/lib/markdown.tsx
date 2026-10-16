import type { ReactNode } from "react";
import { ArtifactPicture, imageId, useArtifactImage } from "@/lib/artifactImages";
import { cn } from "@/lib/utils";

/**
 * A document's markdown, parsed here into elements rather than into
 * HTML. Whatever the model wrote, it can only ever be text in the page:
 * a `<script>` in it is shown as the word `<script>`. Only what a
 * document needs is read (headings, paragraphs, lists, checklists,
 * simple tables, quotes, code, bold, italic and links), and anything
 * else is a paragraph of its own text.
 *
 * A picture is one the artifact keeps, `![alt](niles-image:12)`, drawn
 * from Niles (see `lib/artifactImages.tsx`). A picture by its web
 * address is never fetched: it would tell that site who is reading,
 * and when. Its alt is shown in its place.
 */
export type Block =
  | { type: "heading"; level: number; text: string }
  | { type: "paragraph"; text: string }
  | { type: "list"; ordered: boolean; start: number; items: ListItem[] }
  | { type: "table"; head: string[]; rows: string[][] }
  | { type: "code"; text: string }
  | { type: "quote"; text: string }
  | { type: "image"; alt: string; src: string }
  | { type: "rule" };

export interface ListItem {
  text: string;
  /** For a checklist item. */
  checked?: boolean;
  /** Its line in the source, which is what a tick changes. */
  line: number;
}

const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const CHECK = /^\s*[-*+]\s+\[([ xX])\]\s+(.*)$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBER = /^\s*(\d+)[.)]\s+(.*)$/;
const RULE = /^\s*([-*_])(\s*\1){2,}\s*$/;
const TABLE_ROW = /^\s*\|.*\|\s*$/;
const DIVIDER = /^\s*\|?(\s*:?-+:?\s*\|)+\s*(:?-+:?\s*)?\|?\s*$/;
/** A line that is only a picture. */
const IMAGE_LINE = /^!\[([^\]]*)\]\(([^)\s]+)\)$/;

function lines(source: string): string[] {
  return source.replace(/\r\n?/g, "\n").split("\n");
}

function cells(row: string): string[] {
  return row
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
}

function listKind(line: string): "bullet" | "number" | null {
  if (BULLET.test(line)) return "bullet";
  if (NUMBER.test(line)) return "number";
  return null;
}

/** Whether a line starts a block of its own, ending a paragraph. */
function startsBlock(line: string, next: string | undefined): boolean {
  const t = line.trim();
  return (
    t.startsWith("```") ||
    t.startsWith(">") ||
    HEADING.test(t) ||
    IMAGE_LINE.test(t) ||
    RULE.test(line) ||
    listKind(line) !== null ||
    (TABLE_ROW.test(line) && next !== undefined && DIVIDER.test(next))
  );
}

export function parseMarkdown(source: string): Block[] {
  const all = lines(source);
  const blocks: Block[] = [];
  let i = 0;
  while (i < all.length) {
    const line = all[i];
    const t = line.trim();
    if (!t) {
      i++;
      continue;
    }
    if (t.startsWith("```")) {
      const body: string[] = [];
      i++;
      while (i < all.length && !all[i].trim().startsWith("```")) body.push(all[i++]);
      i++;
      blocks.push({ type: "code", text: body.join("\n") });
      continue;
    }
    const heading = HEADING.exec(t);
    if (heading) {
      blocks.push({ type: "heading", level: heading[1].length, text: heading[2] });
      i++;
      continue;
    }
    if (RULE.test(line)) {
      blocks.push({ type: "rule" });
      i++;
      continue;
    }
    const image = IMAGE_LINE.exec(t);
    if (image) {
      blocks.push({ type: "image", alt: image[1], src: image[2] });
      i++;
      continue;
    }
    if (TABLE_ROW.test(line) && i + 1 < all.length && DIVIDER.test(all[i + 1])) {
      const head = cells(line);
      const rows: string[][] = [];
      i += 2;
      while (i < all.length && TABLE_ROW.test(all[i])) rows.push(cells(all[i++]));
      blocks.push({ type: "table", head, rows });
      continue;
    }
    const kind = listKind(line);
    if (kind) {
      const items: ListItem[] = [];
      const start = kind === "number" ? Number(NUMBER.exec(line)![1]) : 1;
      while (i < all.length && all[i].trim()) {
        const here = all[i];
        if (listKind(here) === kind) {
          const check = CHECK.exec(here);
          if (check) {
            items.push({ text: check[2], checked: check[1] !== " ", line: i });
          } else {
            const m = (kind === "bullet" ? BULLET : NUMBER).exec(here)!;
            items.push({ text: m[kind === "bullet" ? 1 : 2], line: i });
          }
        } else if (listKind(here) || startsBlock(here, all[i + 1])) {
          break;
        } else {
          // A line that carries on the item above it.
          items[items.length - 1].text += ` ${here.trim()}`;
        }
        i++;
      }
      blocks.push({ type: "list", ordered: kind === "number", start, items });
      continue;
    }
    if (t.startsWith(">")) {
      const quoted: string[] = [];
      while (i < all.length && all[i].trim().startsWith(">")) {
        quoted.push(all[i++].trim().replace(/^>\s?/, ""));
      }
      blocks.push({ type: "quote", text: quoted.join(" ") });
      continue;
    }
    const paragraph: string[] = [t];
    i++;
    while (i < all.length && all[i].trim() && !startsBlock(all[i], all[i + 1])) {
      paragraph.push(all[i++].trim());
    }
    blocks.push({ type: "paragraph", text: paragraph.join(" ") });
  }
  return blocks;
}

/** The markdown with the checklist item on `line` ticked, or unticked. */
export function toggleCheck(source: string, line: number): string {
  const all = lines(source);
  const here = all[line];
  if (here === undefined || !CHECK.test(here)) return source;
  all[line] = here.replace(/\[([ xX])\]/, (_, mark: string) => (mark === " " ? "[x]" : "[ ]"));
  return all.join("\n");
}

/** Links go only where a link should: the web, or a mail. */
export function safeHref(href: string): string | undefined {
  return /^(https?:\/\/|mailto:)/i.test(href.trim()) ? href.trim() : undefined;
}

const INLINE =
  /`([^`]+)`|!\[([^\]]*)\]\(([^)\s]+)\)|\[([^\]]+)\]\(([^)\s]+)\)|\*\*(.+?)\*\*|__(.+?)__|\*([^*\s][^*]*?)\*/g;

/** A picture within a line of text: small, and without its credit,
    which a picture on a line of its own has. */
function InlinePicture({ alt, src }: { alt: string; src: string }) {
  const kept = useArtifactImage(src);
  if (!kept) return alt ? <span className="text-muted-foreground italic">{alt}</span> : null;
  return (
    <img
      src={kept.src}
      alt={alt || kept.image.alt}
      loading="lazy"
      decoding="async"
      referrerPolicy="no-referrer"
      className="inline-block max-h-40 rounded-md align-middle"
    />
  );
}

/** Bold, italic, code and links within a line, as elements. */
export function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let key = 0;
  for (const m of text.matchAll(INLINE)) {
    const at = m.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    const [, code, alt, src, label, href, bold, bold2, italic] = m;
    if (alt !== undefined) {
      out.push(<InlinePicture key={key++} alt={alt} src={src} />);
    } else if (code !== undefined) {
      out.push(
        <code key={key++} className="bg-muted rounded px-1 py-0.5 font-mono text-[0.85em]">
          {code}
        </code>,
      );
    } else if (label !== undefined) {
      const safe = safeHref(href);
      out.push(
        safe ? (
          <a
            key={key++}
            href={safe}
            target="_blank"
            rel="noopener noreferrer nofollow"
            className="text-primary underline underline-offset-2"
          >
            {inline(label)}
          </a>
        ) : (
          <span key={key++}>{inline(label)}</span>
        ),
      );
    } else if (bold !== undefined || bold2 !== undefined) {
      out.push(<strong key={key++}>{inline(bold ?? bold2)}</strong>);
    } else if (italic !== undefined) {
      out.push(<em key={key++}>{inline(italic)}</em>);
    }
    last = at + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** The words of a line without their markup: for a reason, or a label. */
export function plainText(text: string): string {
  return text
    .replace(/!\[([^\]]*)\]\([^)\s]+\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)\s]+\)/g, "$1")
    .replace(/[*_`]/g, "")
    .trim();
}

export interface MarkdownProps {
  source: string;
  /** A checklist item ticked or unticked. Without it, the boxes are
      shown and cannot be changed. */
  onToggle?: (item: ListItem, checked: boolean) => void;
  disabled?: boolean;
}

export function Markdown({ source, onToggle, disabled }: MarkdownProps) {
  const blocks = parseMarkdown(source);
  return (
    <div className="flex flex-col gap-3 text-sm leading-relaxed break-words">
      {blocks.map((block, i) => (
        <BlockView key={i} block={block} onToggle={onToggle} disabled={disabled} />
      ))}
    </div>
  );
}

function BlockView({
  block,
  onToggle,
  disabled,
}: {
  block: Block;
  onToggle?: MarkdownProps["onToggle"];
  disabled?: boolean;
}) {
  switch (block.type) {
    case "heading": {
      // The panel's own title is the page's heading; a document's start
      // under it.
      const Tag = `h${Math.min(block.level + 2, 6)}` as "h3";
      return (
        <Tag
          className={cn(
            "font-heading leading-snug font-medium",
            block.level === 1 ? "text-lg" : block.level === 2 ? "text-base" : "text-sm",
          )}
        >
          {inline(block.text)}
        </Tag>
      );
    }
    case "paragraph":
      return <p>{inline(block.text)}</p>;
    case "quote":
      return (
        <blockquote className="border-border text-muted-foreground border-l-2 pl-3">
          {inline(block.text)}
        </blockquote>
      );
    case "code":
      return (
        <pre className="bg-muted overflow-x-auto rounded-md p-3 font-mono text-xs">
          {block.text}
        </pre>
      );
    case "rule":
      return <hr className="border-border" />;
    case "image":
      return imageId(block.src) !== undefined ? (
        <ArtifactPicture name={block.src} alt={block.alt} />
      ) : block.alt ? (
        <p className="text-muted-foreground italic">{block.alt}</p>
      ) : null;
    case "table":
      return (
        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-left text-sm">
            <thead>
              <tr>
                {block.head.map((cell, i) => (
                  <th key={i} className="border-border border-b px-2 py-1 font-medium">
                    {inline(cell)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((cell, c) => (
                    <td key={c} className="border-border border-b px-2 py-1 align-top">
                      {inline(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "list": {
      const checklist = block.items.every((item) => item.checked !== undefined);
      const items = block.items.map((item) =>
        item.checked === undefined ? (
          <li key={item.line}>{inline(item.text)}</li>
        ) : (
          <li key={item.line} className="list-none">
            <label className="flex items-start gap-2">
              <input
                type="checkbox"
                checked={item.checked}
                disabled={disabled || !onToggle}
                onChange={(e) => onToggle?.(item, e.target.checked)}
                className="accent-primary mt-1 size-4 shrink-0"
              />
              <span className={cn(item.checked && "text-muted-foreground line-through")}>
                {inline(item.text)}
              </span>
            </label>
          </li>
        ),
      );
      const listClass = cn("flex flex-col gap-1", checklist ? "pl-0" : "pl-5");
      return block.ordered ? (
        <ol start={block.start} className={cn(listClass, !checklist && "list-decimal")}>
          {items}
        </ol>
      ) : (
        <ul className={cn(listClass, !checklist && "list-disc")}>{items}</ul>
      );
    }
  }
}
