import { describe, expect, it } from "vitest";
import { routeOf } from "@/lib/route";

describe("routeOf", () => {
  it("reads the page off the hash", () => {
    expect(routeOf("#/me/settings")).toBe("/me/settings");
  });

  it("is the house with no hash at all", () => {
    // The installed app starts at `/`, with nothing after it.
    expect(routeOf("")).toBe("/");
    expect(routeOf("#")).toBe("/");
    expect(routeOf("#/")).toBe("/");
  });

  it("ignores a trailing slash", () => {
    expect(routeOf("#/me/")).toBe("/me");
  });

  it("treats an in-page anchor as no route", () => {
    expect(routeOf("#section")).toBe("/");
  });
});
