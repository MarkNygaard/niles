import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ArtifactImagesProvider } from "@/lib/artifactImages";
import { Markdown, parseMarkdown, plainText, safeHref, toggleCheck } from "@/lib/markdown";

const PLAN = `# Thailand

Two weeks in **March**.

## Before we go
- [x] Book flights
- [ ] Ferry to Koh Tao
  from Chumphon

1. Bangkok
2. Koh Tao

| Day | Where |
|-----|-------|
| 1 | Bangkok |

---
> Pack light.`;

describe("parseMarkdown", () => {
  it("reads the blocks a document has", () => {
    const blocks = parseMarkdown(PLAN);
    expect(blocks.map((b) => b.type)).toEqual([
      "heading",
      "paragraph",
      "heading",
      "list",
      "list",
      "table",
      "rule",
      "quote",
    ]);
  });

  it("reads a checklist with each item's line, carrying a wrapped line on", () => {
    const list = parseMarkdown(PLAN)[3];
    expect(list).toEqual({
      type: "list",
      ordered: false,
      start: 1,
      items: [
        { text: "Book flights", checked: true, line: 5 },
        { text: "Ferry to Koh Tao from Chumphon", checked: false, line: 6 },
      ],
    });
  });

  it("reads a table's head and rows", () => {
    expect(parseMarkdown(PLAN)[5]).toEqual({
      type: "table",
      head: ["Day", "Where"],
      rows: [["1", "Bangkok"]],
    });
  });
});

describe("toggleCheck", () => {
  it("ticks and unticks the item on that line and nothing else", () => {
    const ticked = toggleCheck(PLAN, 6);
    expect(ticked.split("\n")[6]).toBe("- [x] Ferry to Koh Tao");
    expect(ticked.split("\n")[5]).toBe("- [x] Book flights");
    expect(toggleCheck(ticked, 5).split("\n")[5]).toBe("- [ ] Book flights");
  });

  it("leaves a line that is not a checklist item as it is", () => {
    expect(toggleCheck(PLAN, 0)).toBe(PLAN);
    expect(toggleCheck(PLAN, 999)).toBe(PLAN);
  });
});

describe("Markdown", () => {
  it("shows raw HTML as the text it is", () => {
    const { container } = render(
      <Markdown source={'<script>alert(1)</script>\n\n<img src=x onerror="alert(2)">'} />,
    );
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText("<script>alert(1)</script>")).toBeInTheDocument();
  });

  it("links only to the web and to mail", () => {
    render(
      <Markdown source="[the map](https://maps.example) and [bad](javascript:alert(1)) and [mail](mailto:a@b.c)" />,
    );
    expect(screen.getByRole("link", { name: "the map" })).toHaveAttribute(
      "href",
      "https://maps.example",
    );
    expect(screen.getByRole("link", { name: "mail" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "bad" })).toBeNull();
    expect(screen.getByText("bad")).toBeInTheDocument();
    expect(safeHref("javascript:alert(1)")).toBeUndefined();
  });

  it("draws bold, italic and code as elements", () => {
    const { container } = render(<Markdown source="**bold** and *soft* and `code`" />);
    expect(container.querySelector("strong")).toHaveTextContent("bold");
    expect(container.querySelector("em")).toHaveTextContent("soft");
    expect(container.querySelector("code")).toHaveTextContent("code");
  });

  it("ticks a checklist item", () => {
    const toggle = vi.fn();
    render(<Markdown source={PLAN} onToggle={toggle} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Ferry to Koh Tao from Chumphon" }));
    expect(toggle).toHaveBeenCalledWith(expect.objectContaining({ line: 6 }), true);
  });

  it("shows the boxes without letting them change when there is nowhere to save", () => {
    render(<Markdown source={PLAN} />);
    expect(screen.getByRole("checkbox", { name: "Book flights" })).toBeDisabled();
  });
});

describe("plainText", () => {
  it("is the words without their markup", () => {
    expect(plainText("**Book** [flights](https://x.example)")).toBe("Book flights");
  });
});

describe("pictures", () => {
  const PALACE = {
    id: 12,
    mime: "image/jpeg",
    width: 1600,
    height: 1067,
    size: 210_000,
    alt: "The Grand Palace",
    credit: "Photo: J. Doe",
    source: "https://commons.wikimedia.org/wiki/File:Palace.jpg",
    created_at: "2026-10-16T10:00:00Z",
  };

  function shown(source: string) {
    return render(
      <ArtifactImagesProvider artifact={7} images={[PALACE]}>
        <Markdown source={source} />
      </ArtifactImagesProvider>,
    );
  }

  it("reads a picture on a line of its own as a block", () => {
    expect(parseMarkdown("Before\n![The palace](niles-image:12)\nAfter")).toEqual([
      { type: "paragraph", text: "Before" },
      { type: "image", alt: "The palace", src: "niles-image:12" },
      { type: "paragraph", text: "After" },
    ]);
  });

  it("draws a kept picture from Niles, with its credit and its page", () => {
    const { container } = shown("![The palace at dawn](niles-image:12)");
    const img = screen.getByRole("img", { name: "The palace at dawn" });
    expect(img.getAttribute("src")).toBe("/me/artifacts/7/images/12");
    expect(img.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(container.querySelector("figcaption")?.textContent).toBe(
      "Photo: J. Doe · commons.wikimedia.org",
    );
    expect(screen.getByRole("link", { name: "commons.wikimedia.org" }).getAttribute("href")).toBe(
      PALACE.source,
    );
  });

  it("never fetches a picture by its web address, and shows its alt instead", () => {
    const { container } = shown(
      "![A tracking pixel](https://tracker.example/p.gif)\n\nSee ![the gate](http://x.example/g.jpg) and ![](//x.example/h.jpg).",
    );
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toContain("A tracking pixel");
    expect(container.textContent).toContain("the gate");
    expect(container.innerHTML).not.toContain("x.example");
  });

  it("shows nothing for a picture the artifact no longer keeps", () => {
    const { container } = shown("![Gone](niles-image:99)\n\nText ![inline](niles-image:12)");
    const imgs = container.querySelectorAll("img");
    expect(imgs).toHaveLength(1);
    expect(imgs[0].getAttribute("src")).toBe("/me/artifacts/7/images/12");
    expect(container.querySelector("figure")).toBeNull();
  });

  it("is its alt in plain text", () => {
    expect(plainText("See ![the gate](niles-image:3) first")).toBe("See the gate first");
  });
});
