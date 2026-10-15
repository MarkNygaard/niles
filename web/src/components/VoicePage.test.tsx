import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import {
  VoicePage,
  assistantOptions,
  assistantShown,
  providerRows,
  roleEntries,
} from "@/components/VoicePage";
import type { VoicePageProps } from "@/components/VoicePage";
import type { Integration, SecretsReport } from "@/lib/api";

const SYSTEM = { id: "system", label: "System", blurb: "", requirement: null };
const ASSISTANTS = { id: "assistants", label: "Assistants & tools", blurb: "", requirement: null };

const ELEVENLABS: Integration = {
  id: "elevenlabs",
  label: "ElevenLabs",
  blurb: "Hears what you say.",
  needs: "An ElevenLabs account.",
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
  default_for: ["llm"],
  secret_keys: ["provider.cerebras.api_key"],
  base_url: "https://api.cerebras.ai/v1",
  serves: ["llm"],
  models: { llm: ["gpt-oss-120b"] },
  secret_key: "provider.cerebras.api_key",
};

const CLAUDE: Integration = {
  ...ELEVENLABS,
  id: "claude_code",
  label: "Claude Code",
  kind: "service",
  group: ASSISTANTS,
  importance: "optional",
  system: false,
  default_for: [],
  secret_keys: ["integrations.claude_code.oauth_token"],
  base_url: null,
  serves: [],
  models: {},
  secret_key: "integrations.claude_code.oauth_token",
};

const CHATGPT: Integration = {
  ...CLAUDE,
  id: "chatgpt",
  label: "ChatGPT",
  secret_keys: ["integrations.chatgpt.access_token"],
  secret_key: "integrations.chatgpt.access_token",
};

function keys(source: "stored" | "unset"): SecretsReport {
  return {
    writable: true,
    secrets: [
      "provider.elevenlabs.api_key",
      "provider.cerebras.api_key",
      "integrations.claude_code.oauth_token",
      "integrations.chatgpt.access_token",
    ].map((key) => ({ key, label: key, source })),
  };
}

describe("assistantOptions", () => {
  it("is Niles's own model alone until an assistant is set up", () => {
    expect(assistantOptions({ integrations: [CLAUDE, CHATGPT] }).map((o) => o.id)).toEqual([
      "none",
    ]);
  });

  it("adds each one that is added, switched on and has its credentials", () => {
    const options = assistantOptions({
      integrations: [
        { ...CLAUDE, added: true },
        { ...CHATGPT, added: true },
      ],
      secrets: keys("stored"),
      chatgptSignedIn: true,
    });
    expect(options.map((o) => o.label)).toEqual(["Niles's own model", "Claude", "ChatGPT"]);
  });

  it("leaves out one that is switched off or cannot run", () => {
    const added = [
      { ...CLAUDE, added: true },
      { ...CHATGPT, added: true },
    ];
    expect(
      assistantOptions({ integrations: added, secrets: keys("unset") }).map((o) => o.id),
    ).toEqual(["none"]);
    expect(
      assistantOptions({
        integrations: added,
        secrets: keys("stored"),
        claudeCode: { enabled: false },
        chatgptSignedIn: false,
      }).map((o) => o.id),
    ).toEqual(["none"]);
  });
});

describe("assistantShown", () => {
  const both = [
    { id: "none" as const },
    { id: "claude_code" as const },
    { id: "chatgpt" as const },
  ];

  it("is neither when neither is chosen, whatever is set up", () => {
    expect(assistantShown("none", both)).toBe("none");
  });

  it("is Claude when nothing is written, as Niles decides it", () => {
    expect(assistantShown(undefined, both)).toBe("claude_code");
    expect(assistantShown("chatgpt", both)).toBe("chatgpt");
  });

  it("is the other one when the preferred one is not set up", () => {
    expect(assistantShown("claude_code", [{ id: "none" }, { id: "chatgpt" }])).toBe("chatgpt");
  });

  it("is Niles's own model when neither is set up", () => {
    expect(assistantShown("claude_code", [{ id: "none" }])).toBe("none");
  });
});

describe("roleEntries", () => {
  it("declares a provider that is not declared yet, keeping the others", () => {
    const entries = roleEntries(
      "stt",
      { provider: "elevenlabs", model: "scribe_v2", effort: null },
      {},
      [ELEVENLABS, { ...CEREBRAS, added: true }],
    );
    expect(entries).toEqual([
      {
        path: "providers",
        value: [
          { name: "cerebras", base_url: "https://api.cerebras.ai/v1", serves: ["llm"] },
          { name: "elevenlabs", base_url: "https://api.elevenlabs.io/v1", serves: ["stt"] },
        ],
      },
      { path: "stt.provider", value: "elevenlabs" },
      { path: "stt.model", value: "scribe_v2" },
    ]);
  });

  it("writes nothing when only the key changed", () => {
    // The key goes to the secret store; a config write as well would be
    // a revision that changed nothing.
    expect(
      roleEntries(
        "llm",
        { provider: "cerebras", model: "gpt-oss-120b", effort: null, key: "k" },
        { provider: "cerebras", model: "gpt-oss-120b" },
        [{ ...CEREBRAS, added: true }],
      ),
    ).toEqual([]);
  });

  it("writes how hard to think as unset rather than leaving it stale", () => {
    expect(
      roleEntries(
        "llm",
        { provider: "cerebras", model: "gpt-oss-120b", effort: null },
        { provider: "cerebras", model: "gpt-oss-120b", effort: "high" },
        [{ ...CEREBRAS, added: true }],
      ),
    ).toEqual([{ path: "llm.reasoning_effort", value: null }]);
  });

  it("reads the declared providers off the catalogue", () => {
    expect(providerRows([ELEVENLABS, { ...CEREBRAS, added: true }, CLAUDE])).toEqual([
      { name: "cerebras", base_url: "https://api.cerebras.ai/v1", serves: ["llm"] },
    ]);
  });
});

function setup(extra: Partial<VoicePageProps> = {}) {
  const onSaveRole = vi.fn();
  const onAssistant = vi.fn();
  render(
    <VoicePage
      integrations={[ELEVENLABS, CEREBRAS, CLAUDE, CHATGPT]}
      stt={{}}
      llm={{}}
      defaultModels={{ stt: "scribe_v2", llm: "gpt-oss-120b" }}
      secrets={keys("unset")}
      gaps={[]}
      requireName={false}
      language={null}
      speaking={null}
      onSaveRole={onSaveRole}
      onRequireName={vi.fn()}
      onAssistant={onAssistant}
      {...extra}
    />,
  );
  return { onSaveRole, onAssistant };
}

describe("VoicePage", () => {
  it("leads with what is still to set up on a new install", () => {
    setup({
      gaps: [
        {
          path: "stt.api_key",
          severity: "degraded",
          consequence: "Niles can't hear yet: choose and set up a hearing provider.",
        },
      ],
    });
    expect(screen.getByText(/Niles can't hear yet/)).toBeInTheDocument();
  });

  it("has a job for each of hearing, thinking, speaking and the assistant", () => {
    setup();
    for (const job of ["Hearing", "Thinking", "Speaking", "Assistant"]) {
      expect(screen.getAllByText(job).length).toBeGreaterThan(0);
    }
    expect(
      within(screen.getByRole("radiogroup", { name: "Hearing provider" })).getAllByRole("radio"),
    ).toHaveLength(1);
  });

  it("chooses neither assistant by writing none", () => {
    const { onAssistant } = setup({
      integrations: [ELEVENLABS, CEREBRAS, { ...CLAUDE, added: true }],
      secrets: keys("stored"),
    });
    const choice = screen.getByRole("radiogroup", { name: "Which assistant answers" });
    expect(within(choice).getByRole("radio", { name: "Claude" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    fireEvent.click(within(choice).getByRole("radio", { name: "Niles's own model" }));
    expect(onAssistant).toHaveBeenCalledWith("none");
  });

  it("says where to set an assistant up when none is", () => {
    setup();
    expect(screen.getByText(/Set up Claude Code or ChatGPT under Add-ons/)).toBeInTheDocument();
  });

  it("saves a role with the key typed beside its provider", () => {
    const { onSaveRole } = setup();
    fireEvent.change(screen.getByLabelText("provider.cerebras.api_key"), {
      target: { value: "cb" },
    });
    fireEvent.click(
      screen
        .getAllByRole("button", { name: "Save" })
        .find((b) => !(b as HTMLButtonElement).disabled)!,
    );
    expect(onSaveRole).toHaveBeenCalledWith("llm", {
      provider: "cerebras",
      model: "gpt-oss-120b",
      effort: null,
      key: "cb",
    });
  });
});
