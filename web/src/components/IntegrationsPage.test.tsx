import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { IntegrationsPage } from "@/components/IntegrationsPage";
import type { Integration, SecretsReport } from "@/lib/api";

const GROQ: Integration = {
  id: "groq",
  label: "Groq",
  blurb: "Speech-to-text and language models.",
  kind: "provider",
  base_url: "https://api.groq.com/openai/v1",
  serves: ["stt", "llm"],
  added: false,
  secret_key: "provider.groq.api_key",
};

const TADO: Integration = {
  id: "tado",
  label: "tado°",
  blurb: "Who is home.",
  kind: "service",
  base_url: null,
  serves: [],
  added: false,
  secret_key: null,
};

const SECRETS: SecretsReport = {
  writable: true,
  secrets: [
    {
      key: "provider.groq.api_key",
      label: "groq API key",
      hint: "api.groq.com",
      source: "unset",
    },
  ],
};

function setup(integrations: Integration[], secrets = SECRETS) {
  const onChange = vi.fn();
  render(
    <IntegrationsPage
      integrations={integrations}
      secrets={secrets}
      onChange={onChange}
      onSecretsChanged={vi.fn()}
      onTadoChanged={vi.fn()}
    />,
  );
  return { onChange };
}

function openAddList() {
  fireEvent.click(screen.getByRole("button", { name: /Add integration/ }));
}

describe("IntegrationsPage", () => {
  it("shows only what is set up", () => {
    // The point of the catalogue: a hundred entries later this is still
    // one card per thing you actually use.
    setup([{ ...GROQ, added: true }, TADO]);
    expect(screen.getByText("Groq")).toBeInTheDocument();
    expect(screen.queryByText("tado°")).toBeNull();
  });

  it("offers the rest behind the add button", () => {
    setup([{ ...GROQ, added: true }, TADO]);
    openAddList();
    expect(screen.getByRole("button", { name: /tado°/ })).toBeInTheDocument();
  });

  it("adds a provider without asking for its endpoint", () => {
    // Which is the whole difference from the pair of text boxes this
    // replaces: Niles already knows where Groq's API lives, and typing
    // a name and a URL let you invent a provider that cannot work.
    const { onChange } = setup([GROQ]);
    openAddList();
    fireEvent.click(screen.getByRole("button", { name: /Groq/ }));
    expect(onChange).toHaveBeenCalledWith("providers", [
      {
        path: "providers",
        value: [
          {
            name: "groq",
            base_url: "https://api.groq.com/openai/v1",
            serves: ["stt", "llm"],
          },
        ],
      },
    ]);
  });

  it("keeps the providers already added when adding another", () => {
    const { onChange } = setup([
      { ...GROQ, added: true },
      {
        ...GROQ,
        id: "cerebras",
        label: "Cerebras",
        base_url: "https://api.cerebras.ai/v1",
        serves: ["llm"],
        added: false,
      },
    ]);
    openAddList();
    fireEvent.click(screen.getByRole("button", { name: /Cerebras/ }));
    const value = onChange.mock.calls[0][1][0].value as { name: string }[];
    expect(value.map((p) => p.name)).toEqual(["groq", "cerebras"]);
  });

  it("asks for the key on the integration's own card", () => {
    setup([{ ...GROQ, added: true }]);
    expect(screen.getByLabelText("groq API key")).toBeInTheDocument();
  });

  it("removes a provider by leaving it out of the list", () => {
    const { onChange } = setup([{ ...GROQ, added: true }]);
    fireEvent.click(screen.getByRole("button", { name: "Remove Groq" }));
    expect(onChange).toHaveBeenCalledWith("providers", [
      { path: "providers", value: [] },
    ]);
  });

  it("says what connecting is for when nothing is", () => {
    setup([GROQ, TADO]);
    expect(screen.getByText(/Nothing connected yet/)).toBeInTheDocument();
  });

  it("says so when everything is already set up", () => {
    setup([{ ...GROQ, added: true }]);
    openAddList();
    expect(screen.getByText(/already set up/)).toBeInTheDocument();
  });
});
