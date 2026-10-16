import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
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
