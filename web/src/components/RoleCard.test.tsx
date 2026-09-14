import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { RoleCard, usableFor } from "@/components/RoleCard";
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

  it("says what is answering today rather than claiming nothing is", () => {
    // Somebody whose config still carries its own endpoint is not
    // misconfigured. Telling them nothing is set up while speech works
    // would be worse than saying nothing.
    setup("stt", { current: undefined, fallbackHost: "api.groq.com" });
    expect(screen.getByText(/api\.groq\.com/)).toBeInTheDocument();
  });
});
