import { describe, expect, it } from "vitest";
import { hashQuery, routeOf } from "@/lib/route";

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

  it("leaves what a link asks of the page out of the route", () => {
    expect(routeOf("#/chat?artifact=12")).toBe("/chat");
  });
});

describe("hashQuery", () => {
  it("reads what follows the ? in the hash", () => {
    expect(hashQuery("#/chat?artifact=12").get("artifact")).toBe("12");
  });

  it("is empty with nothing after the route", () => {
    expect(hashQuery("#/chat").get("artifact")).toBeNull();
    expect(hashQuery("").get("artifact")).toBeNull();
  });
});
