import { describe, expect, it } from "vitest";
import { pairingLine } from "./TvPanel";
import type { TvInfo } from "@/lib/api";

function tv(over: Partial<TvInfo>): TvInfo {
  return {
    configured: true,
    paired: false,
    mac: null,
    room: null,
    status: null,
    error: null,
    ...over,
  };
}

describe("pairingLine", () => {
  it("asks for the address before anything else", () => {
    expect(pairingLine(undefined)).toMatch(/address/);
    expect(pairingLine(tv({ configured: false }))).toMatch(/address/);
  });

  it("says when it is not paired", () => {
    expect(pairingLine(tv({}))).toBe("Not paired yet.");
  });

  it("says what it will be woken by, or that it cannot be", () => {
    expect(pairingLine(tv({ paired: true, mac: "a8:23:fe:01:02:03" }))).toBe(
      "Paired. Woken by a8:23:fe:01:02:03.",
    );
    expect(pairingLine(tv({ paired: true }))).toMatch(/cannot turn it on/);
  });
});
