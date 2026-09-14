import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { RoleCard } from "@/components/RoleCard";
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

function setup(role: "stt" | "llm", providers: Provider[] = [GROQ, CEREBRAS]) {
  const onSave = vi.fn();
  render(
    <RoleCard
      role={role}
      title={role === "stt" ? "Speech-to-text" : "Language model"}
      description="…"
      providers={providers}
      current="groq"
      model="openai/gpt-oss-20b"
      onSave={onSave}
    />,
  );
  return { onSave };
}

describe("RoleCard", () => {
  it("offers only providers that can do the job", () => {
    // A language-only provider has no speech endpoint. Offering it
    // would turn a 404 from somebody else's server into the way you
    // find that out.
    setup("stt");
    const select = screen.getByRole("combobox", { name: /provider/i });
    expect(select).toHaveTextContent("groq");
    expect(select).not.toHaveTextContent("cerebras");
  });

  it("offers both where both apply", () => {
    setup("llm");
    const select = screen.getByRole("combobox", { name: /provider/i });
    expect(select).toHaveTextContent("groq");
    expect(select).toHaveTextContent("cerebras");
  });

  it("treats a provider that says nothing as able to do anything", () => {
    setup("stt", [{ name: "somewhere", base_url: "https://example.test/v1" }]);
    expect(
      screen.getByRole("combobox", { name: /provider/i }),
    ).toHaveTextContent("somewhere");
  });

  it("saves the provider and the model as one change", () => {
    // They cannot move separately: a model name is not portable, so
    // half a change is a request that fails at the next transcription.
    const { onSave } = setup("llm");
    fireEvent.change(screen.getByRole("combobox", { name: /provider/i }), {
      target: { value: "cerebras" },
    });
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

  it("does not offer un-picking once something is picked", () => {
    // The blank used to be an option called "From the config", which
    // read as a choice. It is not one — it means the endpoint written
    // into this role's own section, which is a leftover rather than
    // something to select.
    setup("llm");
    const select = screen.getByRole("combobox", { name: /provider/i });
    expect(select).not.toHaveTextContent("Pick one");
    expect(select).not.toHaveTextContent("From the config");
  });

  it("asks you to pick when nothing is picked yet", () => {
    render(
      <RoleCard
        role="llm"
        title="Language model"
        description="…"
        providers={[GROQ]}
        model=""
        onSave={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("combobox", { name: /provider/i }),
    ).toHaveTextContent("Pick one");
  });

  it("says where to go when nothing can do the job", () => {
    render(
      <RoleCard
        role="stt"
        title="Speech-to-text"
        description="…"
        providers={[CEREBRAS]}
        model=""
        onSave={vi.fn()}
      />,
    );
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.getByText(/Add one under Integrations/)).toBeInTheDocument();
  });

  it("says what is answering today rather than claiming nothing is", () => {
    // Somebody whose config still carries its own endpoint is not
    // misconfigured. Telling them nothing is set up while speech works
    // would be worse than saying nothing.
    render(
      <RoleCard
        role="stt"
        title="Speech-to-text"
        description="…"
        providers={[GROQ]}
        model="whisper-large-v3-turbo"
        fallbackHost="api.groq.com"
        onSave={vi.fn()}
      />,
    );
    expect(screen.getByText(/api\.groq\.com/)).toBeInTheDocument();
  });
});
