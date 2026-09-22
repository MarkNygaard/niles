import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import {
  EFFORTS,
  RoleCard,
  choices,
  effortToSave,
  effortValue,
  usableFor,
} from "@/components/RoleCard";
import type { Provider } from "@/lib/api";

const GROQ: Provider = {
  name: "groq",
  base_url: "https://api.groq.com/openai/v1",
  serves: ["stt", "llm"],
};

const CEREBRAS: Provider = {
  name: "cerebras",
  base_url: "https://api.cerebras.ai/v1",
  serves: ["llm"],
};

function setup(
  role: "stt" | "llm",
  props: Partial<React.ComponentProps<typeof RoleCard>> = {},
) {
  const onSave = vi.fn();
  render(
    <RoleCard
      role={role}
      title={role === "stt" ? "Speech-to-text" : "Language model"}
      description="…"
      providers={[GROQ, CEREBRAS]}
      current="groq"
      model="openai/gpt-oss-20b"
      onSave={onSave}
      {...props}
    />,
  );
  return { onSave };
}

// The rule lives in a function rather than being read off the rendered
// options, because Base UI's Select popup hangs jsdom outright and
// cannot be opened at all. Testing the rule beats testing nothing.
describe("usableFor", () => {
  it("offers only providers that can do the job", () => {
    // A language-only provider has no speech endpoint. Offering it
    // would turn a 404 from somebody else's server into the way you
    // find that out.
    expect(usableFor([GROQ, CEREBRAS], "stt").map((p) => p.name)).toEqual([
      "groq",
    ]);
  });

  it("offers both where both apply", () => {
    expect(usableFor([GROQ, CEREBRAS], "llm").map((p) => p.name)).toEqual([
      "groq",
      "cerebras",
    ]);
  });

  it("treats a provider that says nothing as able to do anything", () => {
    const bare: Provider = {
      name: "somewhere",
      base_url: "https://example.test/v1",
    };
    expect(usableFor([bare], "stt")).toEqual([bare]);
  });
});

describe("choices", () => {
  it("offers what the provider is known to serve", () => {
    expect(choices(["whisper-large-v3-turbo", "whisper-large-v3"])).toEqual([
      "whisper-large-v3-turbo",
      "whisper-large-v3",
    ]);
  });

  it("keeps a configured model the list has never heard of", () => {
    // The shipped list goes stale the week a provider adds something,
    // and a dropdown that cannot express the value already in the
    // config would silently offer to change it.
    expect(choices(["whisper-large-v3-turbo"], "whisper-next")).toEqual([
      "whisper-large-v3-turbo",
      "whisper-next",
    ]);
  });

  it("does not list a configured model twice", () => {
    expect(choices(["a", "b"], "b")).toEqual(["a", "b"]);
  });

  it("is empty for a provider nothing is known about", () => {
    // Which is what makes the box fall back to being typed in.
    expect(choices(undefined, undefined)).toEqual([]);
  });
});

describe("RoleCard", () => {
  it("saves the provider and the model as one change", () => {
    // They cannot move separately: a model name is not portable, so
    // half a change is a request that fails at the next transcription.
    const { onSave } = setup("llm", { current: "cerebras" });
    fireEvent.change(screen.getByRole("textbox", { name: /model/i }), {
      target: { value: "llama3.1-8b" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith({
      provider: "cerebras",
      model: "llama3.1-8b",
      // Untouched, and saved as unset rather than omitted: the row is
      // written whole, so leaving it out would keep a stale value.
      effort: null,
    });
  });

  it("offers nothing to save until something changed", () => {
    setup("llm");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("says which model is running when none is written down", () => {
    // The box was blank while Niles was happily using its default,
    // which made the page look like it was asking for something it
    // already had.
    setup("stt", { model: "", defaultModel: "whisper-large-v3-turbo" });
    expect(screen.getByText(/what Niles ships with/)).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: /model/i })).toHaveAttribute(
      "placeholder",
      "whisper-large-v3-turbo",
    );
  });

  it("says where to go when nothing can do the job", () => {
    setup("stt", { providers: [CEREBRAS], current: undefined, model: "" });
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.getByText(/Add one under Integrations/)).toBeInTheDocument();
  });

  it("offers a list rather than a box when the models are known", () => {
    // The whole point of putting them in the catalogue: nobody
    // remembers `distil-whisper-large-v3-en`, and a box you have to
    // guess at is a box that gets a wrong answer typed into it.
    setup("stt", { models: { groq: ["whisper-large-v3-turbo"] } });
    expect(screen.queryByRole("textbox", { name: /model/i })).toBeNull();
  });

  it("falls back to a box for a provider nothing is known about", () => {
    setup("stt", { models: {} });
    expect(screen.getByRole("textbox", { name: /model/i })).toBeInTheDocument();
  });

  it("says what is answering today rather than claiming nothing is", () => {
    // Somebody whose config still carries its own endpoint is not
    // misconfigured. Telling them nothing is set up while speech works
    // would be worse than saying nothing.
    setup("stt", { current: undefined, fallbackHost: "api.groq.com" });
    expect(screen.getByText(/api\.groq\.com/)).toBeInTheDocument();
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
    // `none` is deliberately absent: two models take it and the rest
    // reject it.
    expect(EFFORTS.map((e) => e.value).slice(1)).toEqual([
      "low",
      "medium",
      "high",
    ]);
  });

  it("shows an unset value as the default rather than as blank", () => {
    expect(effortValue(undefined)).toBe(EFFORTS[0].value);
    expect(effortValue("")).toBe(EFFORTS[0].value);
    expect(effortValue("  ")).toBe(EFFORTS[0].value);
  });

  it("shows a configured value as itself", () => {
    expect(effortValue("low")).toBe("low");
    expect(effortToSave("low")).toBe("low");
  });
});
