import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { svgToReact } from "@/lib/svg";

const svg = (body: string, attributes = "") =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"${attributes}>${body}</svg>`;

function drawn(text: string) {
  const element = svgToReact(text, "size-6");
  return element ? render(element).container : null;
}

describe("svgToReact", () => {
  it("rebuilds an allowed logo inline, sized by the page", () => {
    const container = drawn(
      svg('<g><path d="M1 1h22" stroke="currentColor" stroke-width="2"/></g>', ' width="48"'),
    )!;
    const root = container.querySelector("svg")!;
    expect(root).toHaveAttribute("viewBox", "0 0 24 24");
    expect(root).toHaveClass("size-6");
    expect(root).not.toHaveAttribute("width");
    expect(container.querySelector("path")).toHaveAttribute("stroke-width", "2");
    expect(container.querySelector("path")).toHaveAttribute("stroke", "currentColor");
  });

  it("refuses anything outside the allowlist rather than drawing part of it", () => {
    for (const bad of [
      svg("<script>alert(1)</script>"),
      svg('<path d="M0 0" onclick="alert(1)"/>'),
      svg('<a href="https://evil.example"><path d="M0 0"/></a>'),
      svg('<path d="M0 0" fill="url(https://evil.example/x)"/>'),
      svg('<image href="https://evil.example/x.png"/>'),
      svg("<foreignObject><div/></foreignObject>"),
      '<html xmlns="http://www.w3.org/1999/xhtml"/>',
      "not xml at all <",
    ]) {
      expect(svgToReact(bad), bad).toBeNull();
    }
  });
});
