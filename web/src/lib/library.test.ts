/**
 * The library helpers: matching repos whatever their spelling, Browse's
 * two shelves, the count and stars lines, and a panel's standing.
 */

import { describe, expect, it } from "vitest";
import type { LibraryEntry, PluginLibrary, PluginSummary } from "@/lib/api";
import {
  countLine,
  entryMatches,
  installedFrom,
  sameRepo,
  shelvesOf,
  standingOf,
  starsLine,
} from "@/lib/library";

let seq = 0;

/** One library entry, with sane defaults and `over` on top. */
function entry(over: Partial<LibraryEntry> = {}): LibraryEntry {
  seq += 1;
  return {
    repo: `https://github.com/jane/add-on-${seq}`,
    owner: "jane",
    standing: "listed",
    removed_reason: null,
    name: `Add-on ${seq}`,
    description: "Does a thing.",
    version: "0.1.0",
    published_at: null,
    installs: 1,
    downloads: 10,
    stars: 1,
    logo: null,
    readme: null,
    permissions: [],
    min_niles: "0.2.0",
    needs_newer_niles: false,
    blocked: null,
    ...over,
  };
}

/** One installed plugin, with `over` on top. */
function plugin(over: Partial<PluginSummary> = {}): PluginSummary {
  return {
    repo: null,
    ...over,
  } as PluginSummary;
}

/** A library holding `entries`. */
function library(...entries: LibraryEntry[]): PluginLibrary {
  return { generated_at: null, cannot: "It cannot see your conversations.", addons: entries };
}

describe("sameRepo", () => {
  it("ignores case, a trailing slash and .git", () => {
    const repo = "https://github.com/Jane/Niles-Spotprice";
    for (const other of [
      "https://github.com/jane/niles-spotprice",
      "https://github.com/Jane/Niles-Spotprice/",
      "https://github.com/Jane/Niles-Spotprice.git",
      "https://github.com/Jane/Niles-Spotprice.git/",
    ]) {
      expect(sameRepo(repo, other)).toBe(true);
    }
    expect(sameRepo(repo, "https://github.com/jane/niles-pool")).toBe(false);
  });
});

describe("installedFrom", () => {
  it("finds by repo whatever the case, and ignores plugins with no repo", () => {
    const spot = entry({ repo: "https://github.com/jane/niles-spotprice" });
    const installed = plugin({ repo: "https://github.com/Jane/Niles-Spotprice" });
    const noRepo = plugin({ repo: null });
    expect(installedFrom(spot, [noRepo, installed])).toBe(installed);
    expect(installedFrom(spot, [noRepo])).toBeUndefined();
  });
});

describe("shelvesOf", () => {
  it("leaves removed ones out and splits and sorts the two kinds", () => {
    const gone = entry({ name: "aaa", standing: "removed" });
    const beta = entry({ name: "beta", standing: "unreviewed" });
    const Alpha = entry({ name: "Alpha" });
    const aardvark = entry({ name: "aardvark" });
    const { listed, unreviewed } = shelvesOf(library(gone, beta, Alpha, aardvark), "");
    expect(listed.map((e) => e.name)).toEqual(["aardvark", "Alpha"]);
    expect(unreviewed.map((e) => e.name)).toEqual(["beta"]);
    expect(listed).not.toContain(gone);
  });

  it("filters by name, description and owner", () => {
    const byName = entry({ name: "Spot prices" });
    const byDescription = entry({ description: "Pool and spa" });
    const byOwner = entry({ owner: "energia", name: "x", description: "x" });
    const lib = library(byName, byDescription, byOwner);
    expect(shelvesOf(lib, "spot").listed).toEqual([byName]);
    expect(shelvesOf(lib, "pool").listed).toEqual([byDescription]);
    expect(shelvesOf(lib, "energia").listed).toEqual([byOwner]);
    expect(shelvesOf(lib, "  ").listed).toHaveLength(3);
  });
});

describe("countLine and starsLine", () => {
  it("counts installs in words", () => {
    expect(countLine(entry({ installs: 1 }))).toBe("1 install");
    expect(countLine(entry({ installs: 1234 }))).toBe("1,234 installs");
  });

  it("falls back to downloads, labelled as downloads", () => {
    expect(countLine(entry({ installs: null, downloads: 120 }))).toBe("120 downloads");
  });

  it("stars use the singular and the plural", () => {
    expect(starsLine(1)).toBe("1 star");
    expect(starsLine(1234)).toBe("1,234 stars");
  });
});

describe("standingOf", () => {
  const repo = "https://github.com/jane/niles-spotprice";

  it("is null with no repo, and null with no library", () => {
    expect(standingOf(plugin({ repo: null }), library())).toBeNull();
    expect(standingOf(plugin({ repo }), undefined)).toBeNull();
  });

  it("says Listed", () => {
    const lib = library(entry({ repo }));
    expect(standingOf(plugin({ repo }), lib)).toEqual({ label: "Listed", note: null });
  });

  it("says Unreviewed for an entry the topic found", () => {
    const unreviewed = library(entry({ repo, standing: "unreviewed" }));
    expect(standingOf(plugin({ repo }), unreviewed)).toEqual({ label: "Unreviewed", note: null });
  });

  it("is null for a repo the library doesn't hold", () => {
    const other = library(entry({ repo: "https://github.com/jane/niles-pool" }));
    expect(standingOf(plugin({ repo }), library())).toBeNull();
    expect(standingOf(plugin({ repo }), other)).toBeNull();
  });

  it("says No longer listed, with the reason and that it keeps working", () => {
    const reason = "Its author archived it";
    const lib = library(entry({ repo, standing: "removed", removed_reason: reason }));
    expect(standingOf(plugin({ repo }), lib)).toEqual({
      label: "No longer listed",
      note: "Taken off the add-on library's list. Its author archived it. It keeps working.",
    });
    const noReason = library(entry({ repo, standing: "removed", removed_reason: null }));
    expect(standingOf(plugin({ repo }), noReason)).toEqual({
      label: "No longer listed",
      note: "Taken off the add-on library's list. It keeps working.",
    });
  });

  it("drops a reason on a listed entry", () => {
    const lib = library(entry({ repo, removed_reason: "Ignored." }));
    expect(standingOf(plugin({ repo }), lib)).toEqual({ label: "Listed", note: null });
  });
});

describe("entryMatches", () => {
  it("matches the repo path too", () => {
    const e = entry({ repo: "https://github.com/jane/niles-spotprice" });
    expect(entryMatches(e, "spotprice")).toBe(true);
    expect(entryMatches(e, "jane/niles")).toBe(true);
    expect(entryMatches(e, "pool")).toBe(false);
  });
});
