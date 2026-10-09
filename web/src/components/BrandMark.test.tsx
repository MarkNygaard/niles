import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { BrandMark } from "./BrandMark";

describe("BrandMark", () => {
  it.each(["groq", "cerebras", "elevenlabs", "tado", "unifi", "claude_code", "nemlig", "linear", "sonos", "spotify"])(
    "draws %s's logo rather than a letter",
    (id) => {
      const { container } = render(<BrandMark id={id} label={id} />);
      expect(container.querySelector("svg")).not.toBeNull();
    },
  );

  it("falls back to the first letter for a service with no logo", () => {
    const { container } = render(<BrandMark id="somebody_new" label="Somebody" />);
    expect(container.querySelector("svg")).toBeNull();
    expect(container.textContent).toBe("S");
  });
});
