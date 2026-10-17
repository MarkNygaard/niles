import { describe, expect, it } from "vitest";
import { podcastOf, podcastText } from "./podcast";
import type { Podcast } from "./podcast";

const EPISODE: Podcast = {
  topic: "Why the sky is blue",
  minutes: 5,
  hosts: ["Heart", "Michael"],
  lines: [
    { host: "Heart", text: "Welcome back. Today: why is the sky blue?" },
    { host: "Michael", text: "Short answer — sunlight scattering off the air itself." },
  ],
  sources: [
    { title: "Rayleigh scattering, explained", url: "https://example.com/rayleigh" },
    { title: "Colour of the sky", url: "https://example.org/sky" },
  ],
};

describe("podcastOf", () => {
  it("reads a valid podcast", () => {
    expect(podcastOf(EPISODE)).toEqual(EPISODE);
  });

  it("gives null for a malformed one", () => {
    expect(podcastOf(null)).toBeNull();
    expect(podcastOf("a podcast")).toBeNull();
    expect(podcastOf({ ...EPISODE, topic: 5 })).toBeNull();
    expect(podcastOf({ ...EPISODE, minutes: "5" })).toBeNull();
    expect(podcastOf({ ...EPISODE, hosts: ["Heart"] })).toBeNull();
    expect(podcastOf({ ...EPISODE, hosts: ["Heart", 5] })).toBeNull();
    expect(podcastOf({ ...EPISODE, lines: [{ host: "Heart" }] })).toBeNull();
    expect(podcastOf({ ...EPISODE, sources: [{ title: "A" }] })).toBeNull();
    expect(podcastOf({ ...EPISODE, sources: "none" })).toBeNull();
  });
});

describe("podcastText", () => {
  it("lists the topic, the hosts, the script and the sources", () => {
    expect(podcastText(EPISODE)).toBe(
      [
        "Why the sky is blue",
        "5 min · Heart and Michael",
        "",
        "Heart: Welcome back. Today: why is the sky blue?",
        "Michael: Short answer — sunlight scattering off the air itself.",
        "",
        "Sources:",
        "- Rayleigh scattering, explained (https://example.com/rayleigh)",
        "- Colour of the sky (https://example.org/sky)",
      ].join("\n"),
    );
  });
});
