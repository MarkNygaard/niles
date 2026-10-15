import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import {
  EFFORTS,
  RoleCard,
  choices,
  effortToSave,
  effortValue,
  modelForProvider,
  modelsOf,
  offeredFor,
  startingProvider,
  usableFor,
} from "@/components/RoleCard";
import type { Integration, SecretsReport } from "@/lib/api";

const SYSTEM = {
  id: "system",
  label: "System",
  blurb: "",
  requirement: null,
};

const ELEVENLABS: Integration = {
  id: "elevenlabs",
  label: "ElevenLabs",
  blurb: "Hears what you say.",
  needs: "An ElevenLabs account and an API key.",
  kind: "provider",
  group: SYSTEM,
  importance: "required",
  system: true,
  default_for: ["stt"],
  icon: "audio-lines",
  secret_keys: ["provider.elevenlabs.api_key"],
  base_url: "https://api.elevenlabs.io/v1",
  serves: ["stt"],
  models: { stt: ["scribe_v2"] },
  added: false,
  secret_key: "provider.elevenlabs.api_key",
};

const CEREBRAS: Integration = {
  ...ELEVENLABS,
  id: "cerebras",
  label: "Cerebras",
  blurb: "The fastest answers.",
  needs: "A Cerebras account and an API key.",
  default_for: ["llm"],
  icon: "brain",
  secret_keys: ["provider.cerebras.api_key"],
  base_url: "https://api.cerebras.ai/v1",
  serves: ["llm"],
  models: { llm: ["gpt-oss-120b", "qwen-3.8-27b"] },
  secret_key: "provider.cerebras.api_key",
};

/** A second thinking provider, to have something to pick between. */
const OTHER: Integration = {
  ...CEREBRAS,
  id: "other",
  label: "Other",
  default_for: [],
  secret_keys: ["provider.other.api_key"],
  models: { llm: ["other-large"] },
  secret_key: "provider.other.api_key",
};

const BROKER: Integration = {
  ...ELEVENLABS,
  id: "mqtt",
  label: "Zigbee & MQTT",
  kind: "service",
  default_for: [],
  serves: [],
  models: {},
  secret_key: null,
};

const KEYS: SecretsReport = {
  writable: true,
  secrets: [ELEVENLABS, CEREBRAS, OTHER].map((p) => ({
    key: p.secret_key!,
    label: `${p.label} API key`,
    source: "unset" as const,
  })),
};

function setup(
  role: "stt" | "llm",
  props: Partial<React.ComponentProps<typeof RoleCard>> = {},
) {
  const onSave = vi.fn();
  render(
    <RoleCard
      role={role}
      title={role === "stt" ? "Hearing" : "Thinking"}
      description="…"
      providers={[ELEVENLABS, CEREBRAS, OTHER, BROKER]}
      model=""
      secrets={KEYS}
      onSave={onSave}
      {...props}
    />,
  );
  return { onSave };
}

function radio(name: string) {
  return screen.getByRole("radio", { name });
}

describe("usableFor", () => {
  it("offers hearing only what can hear", () => {
    // Cerebras has no transcription endpoint. Offering it would turn a
    // 404 from somebody else's server into the way you find that out.
    expect(usableFor([ELEVENLABS, CEREBRAS, BROKER], "stt").map((p) => p.id)).toEqual([
      "elevenlabs",
    ]);
  });

  it("offers thinking only what can think, and never a service", () => {
    expect(usableFor([ELEVENLABS, CEREBRAS, OTHER, BROKER], "llm").map((p) => p.id)).toEqual([
      "cerebras",
      "other",
    ]);
  });
});

describe("startingProvider", () => {
  const usable = [CEREBRAS, OTHER];

  it("is the one the role names", () => {
    expect(startingProvider(usable, "llm", "other", false)).toBe("other");
  });

  it("is the role's default when it names none", () => {
    // What Niles is using anyway, shown ready for its key.
    expect(startingProvider(usable, "llm", undefined, false)).toBe("cerebras");
  });

  it("is the default when the role names one Niles no longer ships", () => {
    // A stored Groq setup: Niles fell back, and the page says what it
    // fell back to rather than a provider it cannot show.
    expect(startingProvider(usable, "llm", "groq", false)).toBe("cerebras");
  });

  it("is nothing for a role on an endpoint of its own", () => {
    expect(startingProvider(usable, "llm", undefined, true)).toBe("");
  });
});

describe("choices", () => {
  it("keeps a configured model the list has never heard of", () => {
    // The shipped list goes stale the week a provider adds something,
    // and a dropdown that cannot express the value already in the
    // config would silently offer to change it.
    expect(choices(["scribe_v2"], "scribe_v3")).toEqual(["scribe_v2", "scribe_v3"]);
  });

  it("does not list a configured model twice", () => {
    expect(choices(["a", "b"], "b")).toEqual(["a", "b"]);
  });

  it("is empty for a provider nothing is known about", () => {
    expect(choices(undefined, undefined)).toEqual([]);
  });
});

describe("RoleCard", () => {
  it("shows the default provider chosen before anybody has chosen", () => {
    setup("stt");
    expect(radio("ElevenLabs")).toHaveAttribute("aria-checked", "true");
    expect(screen.getByLabelText("ElevenLabs API key")).toBeInTheDocument();
  });

  it("offers nothing to save until something changed", () => {
    setup("llm");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("sets a provider up by picking it, typing its key and saving once", () => {
    // Picking is setting up: no separate step to add the provider first.
    const { onSave } = setup("llm");
    fireEvent.click(radio("Other"));
    expect(radio("Other")).toHaveAttribute("aria-checked", "true");
    fireEvent.change(screen.getByLabelText("Other API key"), {
      target: { value: " sk-other " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith({
      provider: "other",
      // The model follows the provider: a name is not portable.
      model: "other-large",
      effort: null,
      key: "sk-other",
    });
  });

  it("saves a key typed for the default provider, with the model it uses", () => {
    // A new install: ElevenLabs is already what hears, so its key is
    // the only thing missing.
    const { onSave } = setup("stt");
    fireEvent.change(screen.getByLabelText("ElevenLabs API key"), {
      target: { value: "el-key" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith({
      provider: "elevenlabs",
      model: "scribe_v2",
      effort: null,
      key: "el-key",
    });
  });

  it("puts a role that fell back from Groq on the default's own model", () => {
    // The stored model was Groq's, which ElevenLabs does not serve and
    // Niles is not running; saving it under ElevenLabs would break
    // hearing the moment the key went in.
    const { onSave } = setup("stt", { current: "groq", model: "whisper-large-v3-turbo" });
    expect(radio("ElevenLabs")).toHaveAttribute("aria-checked", "true");
    fireEvent.change(screen.getByLabelText("ElevenLabs API key"), {
      target: { value: "el-key" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ provider: "elevenlabs", model: "scribe_v2" }),
    );
  });

  it("does not ask for a key that comes from the environment", () => {
    setup("stt", {
      secrets: {
        writable: true,
        secrets: [{ ...KEYS.secrets[0], source: "environment" }],
      },
    });
    expect(screen.queryByLabelText("ElevenLabs API key")).toBeNull();
    expect(screen.getByText(/environment variable/)).toBeInTheDocument();
  });

  it("says a saved key is saved", () => {
    setup("stt", {
      secrets: { writable: true, secrets: [{ ...KEYS.secrets[0], source: "stored" }] },
    });
    expect(screen.getByText("saved here")).toBeInTheDocument();
    expect(screen.getByLabelText("ElevenLabs API key")).toHaveAttribute(
      "placeholder",
      "Replace it…",
    );
  });

  it("asks how hard to think only when thinking", () => {
    setup("stt");
    expect(screen.queryByRole("combobox", { name: /reasoning effort/ })).toBeNull();
  });

  it("falls back to a box for a provider nothing is known about", () => {
    setup("llm", { providers: [{ ...CEREBRAS, models: {} }] });
    expect(screen.getByRole("textbox", { name: /model/i })).toBeInTheDocument();
  });

  it("offers a list rather than a box when the models are known", () => {
    // Nobody remembers model names, and a box you have to guess at is a
    // box that gets a wrong answer typed into it.
    setup("llm");
    expect(screen.queryByRole("textbox", { name: /model/i })).toBeNull();
  });

  it("says what is answering today rather than claiming nothing is", () => {
    // Somebody whose config still carries its own endpoint is not
    // misconfigured.
    setup("stt", { fallbackHost: "whisper.local" });
    expect(screen.getByText(/whisper\.local/)).toBeInTheDocument();
    expect(radio("ElevenLabs")).toHaveAttribute("aria-checked", "false");
  });
});

describe("how hard to think", () => {
  it("offers the provider's own default first", () => {
    // Niles must not choose on behalf of a provider nobody asked
    // about: not every model takes the field, and one that does not
    // answers 400.
    expect(EFFORTS[0].label).toBe("Provider default");
    expect(effortToSave(EFFORTS[0].value)).toBeNull();
  });

  it("offers only what a model will accept", () => {
    expect(EFFORTS.map((e) => e.value).slice(1)).toEqual(["low", "medium", "high"]);
  });

  it("shows an unset value as the default rather than as blank", () => {
    expect(effortValue(undefined)).toBe(EFFORTS[0].value);
    expect(effortValue("  ")).toBe(EFFORTS[0].value);
  });

  it("shows a configured value as itself", () => {
    expect(effortValue("low")).toBe("low");
    expect(effortToSave("low")).toBe("low");
  });
});

describe("the model list follows the provider", () => {
  const MODELS = modelsOf([CEREBRAS, OTHER], "llm");

  it("reads each provider's models for the role", () => {
    expect(MODELS).toEqual({
      cerebras: ["gpt-oss-120b", "qwen-3.8-27b"],
      other: ["other-large"],
    });
    expect(modelsOf([CEREBRAS], "stt")).toEqual({});
  });

  it("offers a provider only its own models", () => {
    // The bug this replaced: picking a second provider still listed the
    // first one's models.
    expect(offeredFor(MODELS, "other", "cerebras", "gpt-oss-120b")).toEqual(["other-large"]);
  });

  it("still shows a configured model the shipped list has not caught up with", () => {
    expect(offeredFor(MODELS, "cerebras", "cerebras", "gpt-oss-200b")).toEqual([
      "gpt-oss-120b",
      "qwen-3.8-27b",
      "gpt-oss-200b",
    ]);
  });

  it("moves the model to the new provider's first one", () => {
    expect(modelForProvider(MODELS, "other")).toBe("other-large");
    expect(modelForProvider(MODELS, "mystery")).toBe("");
    expect(modelForProvider(undefined, "cerebras")).toBe("");
  });
});
