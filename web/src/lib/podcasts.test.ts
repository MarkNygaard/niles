import { describe, expect, it } from "vitest";
import type { PodcastsTest } from "@/lib/api";
import {
  DEFAULT_KEEP_DAYS,
  DEFAULT_REGION,
  DEFAULT_VOICES,
  isEnglishVoice,
  keepDays,
  kokoroAddress,
  r2Endpoint,
  statusLine,
  testLines,
  voiceChoices,
  voiceLabel,
} from "./podcasts";

describe("defaults", () => {
  it("twins the config's defaults", () => {
    expect(DEFAULT_VOICES).toEqual(["af_heart", "am_michael"]);
    expect(DEFAULT_REGION).toBe("auto");
    expect(DEFAULT_KEEP_DAYS).toBe(14);
  });
});

describe("isEnglishVoice", () => {
  it("takes Kokoro's English ids and nothing else", () => {
    expect(isEnglishVoice("af_heart")).toBe(true);
    expect(isEnglishVoice("bm_george")).toBe(true);
    // Other languages share the alphabet but not the shape's meaning,
    // and zip files speak no language at all.
    expect(isEnglishVoice("zm_yunjian")).toBe(false);
    expect(isEnglishVoice("heart")).toBe(false);
    expect(isEnglishVoice("")).toBe(false);
  });
});

describe("voiceLabel", () => {
  it("names the voice and its accent from the id", () => {
    expect(voiceLabel("af_heart")).toBe("Heart · American woman");
    expect(voiceLabel("bm_george")).toBe("George · British man");
  });

  it("says the id itself when it does not know the voice", () => {
    expect(voiceLabel("af_newvoice_2")).toBe("Newvoice 2 · American woman");
    expect(voiceLabel("custom")).toBe("custom");
  });
});

describe("voiceChoices", () => {
  const VOICES = ["af_heart", "am_michael", "bf_emma", "bm_george", "zm_yunjian"];

  it("leaves out the non-English voices and the other host's voice", () => {
    expect(voiceChoices(VOICES, "af_heart", "am_michael")).toEqual([
      "af_heart",
      "bf_emma",
      "bm_george",
    ]);
  });

  it("keeps a current voice Kokoro no longer lists", () => {
    expect(voiceChoices(VOICES, "af_gone", "am_michael")).toEqual([
      "af_gone",
      "af_heart",
      "bf_emma",
      "bm_george",
    ]);
  });

  it("sorts", () => {
    expect(voiceChoices(["bm_george", "af_heart", "bf_emma"], "af_heart", "bf_emma")).toEqual([
      "af_heart",
      "bm_george",
    ]);
  });
});

describe("kokoroAddress", () => {
  it("adds http to a bare host and strips /v1 and a trailing slash", () => {
    expect(kokoroAddress("kokoro.ns.svc:8880/v1/")).toBe("http://kokoro.ns.svc:8880");
    expect(kokoroAddress("kokoro.ns.svc:8880")).toBe("http://kokoro.ns.svc:8880");
    expect(kokoroAddress("http://kokoro.ns.svc:8880/v1")).toBe("http://kokoro.ns.svc:8880");
  });

  it("keeps an https address as the one typed", () => {
    expect(kokoroAddress("https://kokoro.example.com/v1/")).toBe("https://kokoro.example.com");
  });

  it("is empty for nothing typed", () => {
    expect(kokoroAddress("  ")).toBe("");
  });
});

describe("r2Endpoint", () => {
  it("adds https to a bare host and strips a trailing slash", () => {
    expect(r2Endpoint("abc123.r2.cloudflarestorage.com/")).toBe(
      "https://abc123.r2.cloudflarestorage.com",
    );
    expect(r2Endpoint("https://abc123.r2.cloudflarestorage.com")).toBe(
      "https://abc123.r2.cloudflarestorage.com",
    );
  });

  it("is empty for nothing typed", () => {
    expect(r2Endpoint("")).toBe("");
  });
});

describe("keepDays", () => {
  it("takes an integer from 1 to 365", () => {
    expect(keepDays("1")).toBe(1);
    expect(keepDays("14")).toBe(14);
    expect(keepDays("365")).toBe(365);
  });

  it("refuses the rest", () => {
    expect(keepDays("0")).toBeNull();
    expect(keepDays("366")).toBeNull();
    expect(keepDays("14.5")).toBeNull();
    expect(keepDays("fortnight")).toBeNull();
    expect(keepDays("")).toBeNull();
  });
});

describe("statusLine", () => {
  const done = (result: PodcastsTest) => statusLine({ state: "done", result });

  const WORKING: PodcastsTest = {
    configured: true,
    missing: [],
    kokoro: { ok: true, error: null },
    storage: { ok: true, error: null },
  };

  it("is checking while the test runs", () => {
    expect(statusLine({ state: "running" }).text).toBe("Checking…");
    expect(statusLine({ state: "idle" }).text).toBe("Checking…");
  });

  it("says Working only when both parts are OK", () => {
    expect(done(WORKING)).toEqual({ tone: "ok", text: "Working" });
    expect(
      done({ ...WORKING, kokoro: { ok: false, error: "Kokoro did not answer." } }),
    ).toEqual({ tone: "bad", text: "Not working" });
    expect(
      done({ ...WORKING, storage: { ok: false, error: "the bucket refused the key" } }),
    ).toEqual({ tone: "bad", text: "Not working" });
  });

  it("names what is missing when it could not run", () => {
    const status = done({
      configured: false,
      missing: ["R2 endpoint", "bucket"],
      kokoro: null,
      storage: null,
    });
    expect(status).toEqual({ tone: "bad", text: "Needs its R2 endpoint, bucket" });
  });

  it("says why the test itself failed", () => {
    expect(statusLine({ state: "failed", message: "request failed (502)" })).toEqual({
      tone: "bad",
      text: "Not reachable: request failed (502)",
    });
  });
});

describe("testLines", () => {
  it("says OK for each part that passed", () => {
    expect(
      testLines({
        configured: true,
        missing: [],
        kokoro: { ok: true, error: null },
        storage: { ok: true, error: null },
      }),
    ).toEqual(["Kokoro: OK.", "Storage: OK."]);
  });

  it("names each failing part and why", () => {
    expect(
      testLines({
        configured: true,
        missing: [],
        kokoro: { ok: true, error: null },
        storage: { ok: false, error: "Uploading a test file: the bucket refused the key" },
      })[1],
    ).toBe("Storage: Uploading a test file: the bucket refused the key");
    expect(
      testLines({
        configured: true,
        missing: [],
        kokoro: { ok: false, error: "Kokoro did not answer within 45s." },
        storage: { ok: true, error: null },
      })[0],
    ).toBe("Kokoro: Kokoro did not answer within 45s.");
  });
});
