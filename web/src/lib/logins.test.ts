import { describe, expect, it } from "vitest";
import { ADDED_A_LOGIN, originOf, sitesOf } from "@/lib/logins";

describe("originOf", () => {
  it("writes an IDN in its punycode form", () => {
    expect(originOf("bücher.example")).toBe("https://xn--bcher-kva.example");
  });

  it("drops the path and query, and lowercases the host", () => {
    expect(originOf("Shop.Example/login?x=1")).toBe("https://shop.example");
  });

  it("reads http as https", () => {
    expect(originOf("http://shop.example")).toBe("https://shop.example");
  });

  it("refuses another scheme, another port and an empty string", () => {
    expect(originOf("ftp://x.example")).toBeNull();
    expect(originOf("https://shop.example:8443")).toBeNull();
    expect(originOf("")).toBeNull();
  });
});

describe("sitesOf", () => {
  it("trims and drops blanks", () => {
    expect(sitesOf(" https://shop.example \n\nlogin.shop.example\n")).toEqual([
      "https://shop.example",
      "login.shop.example",
    ]);
  });
});

describe("ADDED_A_LOGIN", () => {
  it("is the fixed words the next turn starts from", () => {
    expect(ADDED_A_LOGIN).toBe("Added a login");
  });
});
