import { describe, expect, it } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Avatar, initials } from "@/components/Avatar";

describe("initials", () => {
  it("takes both parts of a dotted address", () => {
    expect(initials("mark.nygaard@hotmail.com")).toBe("MN");
  });

  it("falls back to the first two letters", () => {
    expect(initials("majse@example.com")).toBe("MA");
  });

  it("handles the other separators people use", () => {
    expect(initials("mark_nygaard@example.com")).toBe("MN");
    expect(initials("mark-nygaard@example.com")).toBe("MN");
  });

  it("has something to draw when nobody is signed in", () => {
    expect(initials(undefined)).toBe("·");
  });
});

describe("Avatar", () => {
  it("falls back to the letters when the picture will not load", () => {
    const { container } = render(
      <Avatar email="mark.nygaard@example.com" avatarUrl="https://example.invalid/a.png" />,
    );
    fireEvent.error(container.querySelector("img")!);
    expect(container.querySelector("img")).toBeNull();
    expect(container).toHaveTextContent("MN");
  });
});
