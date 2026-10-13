import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { IntegrationsPage } from "@/components/IntegrationsPage";
import { api } from "@/lib/api";
import type { Integration, IntegrationGroup, SecretsReport } from "@/lib/api";

const ESSENTIALS: IntegrationGroup = {
  id: "essentials",
  label: "Essentials",
  blurb: "What Niles needs.",
  requirement: "Niles needs one speech-to-text provider and one AI provider.",
};
const PRESENCE: IntegrationGroup = {
  id: "presence",
  label: "Presence & climate",
  blurb: "Who is home.",
  requirement: null,
};
const SHOPPING: IntegrationGroup = {
  id: "shopping",
  label: "Shopping",
  blurb: "Groceries.",
  requirement: null,
};
const ASSISTANTS: IntegrationGroup = {
  id: "assistants",
  label: "Assistants & tools",
  blurb: "For the technically minded.",
  requirement: null,
};

const GROQ: Integration = {
  id: "groq",
  label: "Groq",
  blurb: "Speech-to-text and language models.",
  needs: "A Groq account and an API key.",
  kind: "provider",
  group: ESSENTIALS,
  importance: "required",
  icon: "sparkles",
  secret_keys: ["provider.groq.api_key"],
  settings_page: null,
  base_url: "https://api.groq.com/openai/v1",
  serves: ["stt", "llm"],
  added: false,
  secret_key: "provider.groq.api_key",
};

const CEREBRAS: Integration = {
  ...GROQ,
  id: "cerebras",
  label: "Cerebras",
  blurb: "Works out the answer.",
  needs: "A Cerebras account and an API key.",
  icon: "brain",
  secret_keys: ["provider.cerebras.api_key"],
  base_url: "https://api.cerebras.ai/v1",
  serves: ["llm"],
  secret_key: "provider.cerebras.api_key",
};

const MQTT: Integration = {
  id: "mqtt",
  label: "Zigbee & MQTT",
  blurb: "How Niles reaches your lights.",
  needs: "An MQTT broker.",
  kind: "setting",
  group: ESSENTIALS,
  importance: "required",
  icon: "network",
  secret_keys: [],
  settings_page: { id: "mqtt", label: "Zigbee & MQTT" },
  base_url: null,
  serves: [],
  added: false,
  secret_key: null,
};

const TADO: Integration = {
  id: "tado",
  label: "tado°",
  blurb: "Who is home.",
  needs: "tado° thermostats.",
  kind: "service",
  group: PRESENCE,
  importance: "optional",
  icon: "thermometer",
  secret_keys: [],
  settings_page: null,
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

const UNIFI: Integration = {
  id: "unifi",
  label: "UniFi",
  blurb: "Who is home, from the Wi-Fi.",
  needs: "A UniFi console.",
  kind: "service",
  group: PRESENCE,
  importance: "recommended",
  icon: "wifi",
  secret_keys: ["presence.unifi.api_key"],
  settings_page: null,
  base_url: null,
  serves: [],
  added: false,
  secret_key: "presence.unifi.api_key",
};

const UNIFI_KEY: SecretsReport = {
  writable: true,
  secrets: [
    {
      key: "presence.unifi.api_key",
      label: "UniFi console API key",
      source: "unset",
    },
  ],
};

const CLAUDE: Integration = {
  id: "claude_code",
  label: "Claude Code",
  blurb: "Answers the chat with Claude, on your Claude subscription.",
  needs: "A Claude subscription.",
  kind: "service",
  group: ASSISTANTS,
  importance: "optional",
  icon: "bot",
  secret_keys: ["integrations.claude_code.oauth_token"],
  settings_page: null,
  base_url: null,
  serves: [],
  added: false,
  secret_key: "integrations.claude_code.oauth_token",
};

const CLAUDE_TOKEN: SecretsReport = {
  writable: true,
  secrets: [
    {
      key: "integrations.claude_code.oauth_token",
      label: "Claude Code token, from `claude setup-token`",
      hint: "claude.ai",
      source: "unset",
    },
  ],
};

const NEMLIG: Integration = {
  id: "nemlig",
  label: "nemlig.com",
  blurb: "Groceries delivered.",
  needs: "A nemlig.com account.",
  kind: "service",
  group: SHOPPING,
  importance: "optional",
  icon: "shopping-basket",
  secret_keys: ["integrations.nemlig.username", "integrations.nemlig.password"],
  settings_page: null,
  base_url: null,
  serves: [],
  added: false,
  secret_key: "integrations.nemlig.password",
};

const NEMLIG_LOGIN: SecretsReport = {
  writable: true,
  secrets: [
    { key: "integrations.nemlig.username", label: "nemlig.com email", hint: "nemlig.com", source: "unset" },
    { key: "integrations.nemlig.password", label: "nemlig.com password", hint: "nemlig.com", source: "unset" },
  ],
};

function setup(
  integrations: Integration[],
  secrets: SecretsReport = SECRETS,
  unifiHost?: string,
  onOpenSettings?: (section: string) => void,
) {
  const onChange = vi.fn();
  const onReset = vi.fn();
  const onSecretsChanged = vi.fn();
  render(
    <IntegrationsPage
      integrations={integrations}
      secrets={secrets}
      unifiHost={unifiHost}
      onChange={onChange}
      onReset={onReset}
      onSecretsChanged={onSecretsChanged}
      onTadoChanged={vi.fn()}
      onOpenSettings={onOpenSettings}
    />,
  );
  return { onChange, onReset, onSecretsChanged };
}

afterEach(() => {
  vi.restoreAllMocks();
});

function openAddList() {
  fireEvent.click(screen.getByRole("button", { name: /Add integration/ }));
}

describe("IntegrationsPage", () => {
  it("shows only what is set up", () => {
    // The point of the catalogue: a hundred entries later this is still
    // one row per thing you actually use.
    setup([{ ...GROQ, added: true }, TADO]);
    expect(screen.getByText("Groq")).toBeInTheDocument();
    expect(screen.queryByText("tado°")).toBeNull();
  });

  it("marks an integration added without a key as unfinished", () => {
    // A row that looks done and answers nothing is worse than one that
    // says what is left to do.
    setup([{ ...GROQ, added: true }]);
    expect(screen.getByText(/not finished/)).toBeInTheDocument();
  });

  it("says nothing is left to do once the key is there", () => {
    setup([{ ...GROQ, added: true }], {
      writable: true,
      secrets: [{ ...SECRETS.secrets[0], source: "stored" }],
    });
    expect(screen.queryByText(/not finished/)).toBeNull();
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

  it("keeps the key out of the list and behind Configure", () => {
    // The list answers "what is connected". A key is setup detail, and
    // true at a glance is not the same as useful at a glance.
    setup([{ ...GROQ, added: true }]);
    expect(screen.queryByLabelText("groq API key")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Configure Groq" }));
    expect(screen.getByLabelText("groq API key")).toBeInTheDocument();
  });

  it("removes a provider by leaving it out of the list", () => {
    const { onChange } = setup([{ ...GROQ, added: true }]);
    fireEvent.click(screen.getByRole("button", { name: "Configure Groq" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove Groq/ }));
    expect(onChange).toHaveBeenCalledWith("providers", [
      { path: "providers", value: [] },
    ]);
  });

  it("says what connecting is for when nothing is", () => {
    setup([GROQ, TADO]);
    expect(screen.getByText(/Nothing yet/)).toBeInTheDocument();
  });

  it("says so when everything is already set up", () => {
    setup([{ ...GROQ, added: true }]);
    openAddList();
    expect(screen.getByText(/already set up/)).toBeInTheDocument();
  });

  it("asks for the console's address and key when UniFi is added", () => {
    const { onChange } = setup([UNIFI], UNIFI_KEY);
    openAddList();
    fireEvent.click(screen.getByRole("button", { name: /UniFi/ }));

    // Adding it writes nothing: an empty address is not a console.
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Console address")).toBeTruthy();
    expect(screen.getByText("UniFi console API key")).toBeTruthy();
  });

  it("saves the address when you leave the field", () => {
    const { onChange } = setup([{ ...UNIFI, added: true }], UNIFI_KEY, "");
    fireEvent.click(screen.getByRole("button", { name: /UniFi/ }));
    const field = screen.getByLabelText("Console address");
    fireEvent.change(field, { target: { value: " 192.168.1.1 " } });
    fireEvent.blur(field);

    expect(onChange).toHaveBeenCalledWith("presence.unifi", [
      { path: "presence.unifi.host", value: "192.168.1.1" },
    ]);
  });

  it("removing UniFi forgets the console", () => {
    const { onChange } = setup(
      [{ ...UNIFI, added: true }],
      UNIFI_KEY,
      "192.168.1.1",
    );
    fireEvent.click(screen.getByRole("button", { name: /UniFi/ }));
    fireEvent.click(screen.getByRole("button", { name: /Remove UniFi/ }));

    expect(onChange).toHaveBeenCalledWith("presence.unifi", [
      { path: "presence.unifi.host", value: "" },
    ]);
  });

  it("switches Claude Code on and asks for its token", () => {
    const { onChange } = setup([CLAUDE], CLAUDE_TOKEN);
    openAddList();
    fireEvent.click(screen.getByRole("button", { name: /Claude Code/ }));

    expect(onChange).toHaveBeenCalledWith("integrations.claude_code", [
      { path: "integrations.claude_code.enabled", value: true },
    ]);
    expect(screen.getByText(/claude setup-token/, { selector: "div, span, label" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: /Sonnet/ })).toHaveAttribute("aria-checked", "true");
  });

  it("picks Claude Code's model", () => {
    const { onChange } = setup([{ ...CLAUDE, added: true }], CLAUDE_TOKEN);
    fireEvent.click(screen.getByRole("button", { name: /Claude Code/ }));
    fireEvent.click(screen.getByRole("radio", { name: /Opus/ }));

    expect(onChange).toHaveBeenCalledWith("integrations.claude_code", [
      { path: "integrations.claude_code.model", value: "opus" },
    ]);
  });

  it("switches Claude Code off without removing it", () => {
    const { onChange } = setup([{ ...CLAUDE, added: true }], CLAUDE_TOKEN);
    fireEvent.click(screen.getByRole("button", { name: /Claude Code/ }));
    fireEvent.click(screen.getByRole("switch"));

    expect(onChange).toHaveBeenCalledWith("integrations.claude_code", [
      { path: "integrations.claude_code.enabled", value: false },
    ]);
  });

  it("removing Claude Code takes the section away", () => {
    const { onChange, onReset } = setup([{ ...CLAUDE, added: true }], CLAUDE_TOKEN);
    fireEvent.click(screen.getByRole("button", { name: /Claude Code/ }));
    fireEvent.click(screen.getByRole("button", { name: /Remove Claude Code/ }));

    expect(onReset).toHaveBeenCalledWith("integrations.claude_code", [
      "integrations.claude_code",
    ]);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("switches nemlig.com on and asks for both halves of the login", () => {
    const { onChange } = setup([NEMLIG], NEMLIG_LOGIN);
    openAddList();
    fireEvent.click(screen.getByRole("button", { name: /nemlig.com/ }));
    expect(onChange).toHaveBeenCalledWith("integrations.nemlig", [
      { path: "integrations.nemlig.enabled", value: true },
    ]);
    expect(screen.getByText("nemlig.com email")).toBeTruthy();
    expect(screen.getByText("nemlig.com password")).toBeTruthy();
  });

  it("switches nemlig.com off without forgetting the login", () => {
    const { onChange, onReset } = setup([{ ...NEMLIG, added: true }], NEMLIG_LOGIN);
    fireEvent.click(screen.getByRole("button", { name: /nemlig.com/ }));
    fireEvent.click(screen.getByRole("switch"));
    expect(onChange).toHaveBeenCalledWith("integrations.nemlig", [
      { path: "integrations.nemlig.enabled", value: false },
    ]);
    expect(onReset).not.toHaveBeenCalled();
  });

  it("removing nemlig.com takes the section away", () => {
    const { onReset } = setup([{ ...NEMLIG, added: true }], NEMLIG_LOGIN);
    fireEvent.click(screen.getByRole("button", { name: /nemlig.com/ }));
    fireEvent.click(screen.getByRole("button", { name: /Remove nemlig.com/ }));
    expect(onReset).toHaveBeenCalledWith("integrations.nemlig", ["integrations.nemlig"]);
  });
});

describe("IntegrationsPage groups", () => {
  it("lists what is set up under its group's heading, in the order given", () => {
    setup(
      [
        { ...GROQ, added: true },
        { ...UNIFI, added: true },
        { ...NEMLIG, added: true },
        TADO,
      ],
      UNIFI_KEY,
      "192.168.1.1",
    );
    const headings = screen
      .getAllByRole("heading", { level: 3 })
      .map((h) => h.textContent);
    expect(headings).toEqual(["Essentials", "Presence & climate", "Shopping"]);

    const presence = screen.getByRole("region", { name: "Presence & climate" });
    expect(within(presence).getByText("UniFi")).toBeInTheDocument();
    // Not added, so not listed — its group shows only what is.
    expect(within(presence).queryByText("tado°")).toBeNull();
    expect(
      within(screen.getByRole("region", { name: "Essentials" })).getByText("Groq"),
    ).toBeInTheDocument();
  });

  it("sends the broker's Configure to its Settings page", () => {
    const onOpenSettings = vi.fn();
    setup([{ ...MQTT, added: true }], SECRETS, undefined, onOpenSettings);
    fireEvent.click(screen.getByRole("button", { name: "Configure Zigbee & MQTT" }));
    expect(onOpenSettings).toHaveBeenCalledWith("mqtt");
  });
});

describe("IntegrationsPage gallery", () => {
  const EVERYTHING = [MQTT, { ...GROQ, added: true }, CEREBRAS, UNIFI, TADO, NEMLIG, CLAUDE];

  function gallery() {
    openAddList();
    return screen.getByRole("dialog", { name: "Add an integration" });
  }

  it("opens as cards under the same groups, Essentials first", () => {
    setup(EVERYTHING);
    const dialog = gallery();
    const headings = within(dialog)
      .getAllByRole("heading", { level: 3 })
      .map((h) => h.textContent);
    expect(headings).toEqual([
      "Essentials",
      "Presence & climate",
      "Shopping",
      "Assistants & tools",
    ]);
    // What a newcomer needs to choose: what it does, and what it takes.
    const nemlig = within(dialog).getByRole("button", { name: /nemlig.com/ });
    expect(nemlig).toHaveTextContent("Groceries delivered.");
    expect(nemlig).toHaveTextContent("You need: A nemlig.com account.");
  });

  it("marks what is already added, and opens its settings", () => {
    setup(EVERYTHING);
    const groq = within(gallery()).getByRole("button", { name: /Groq/ });
    expect(groq).toHaveTextContent("Added");
    fireEvent.click(groq);
    expect(screen.getByLabelText("groq API key")).toBeInTheDocument();
  });

  it("badges Required and Recommended where they apply", () => {
    setup([MQTT, GROQ, CEREBRAS, UNIFI, TADO], SECRETS, undefined, vi.fn());
    const dialog = gallery();
    const card = (name: RegExp) => within(dialog).getByRole("button", { name });
    expect(card(/Zigbee & MQTT/)).toHaveTextContent("Required");
    expect(card(/Groq/)).toHaveTextContent("Required");
    expect(card(/UniFi/)).toHaveTextContent("Recommended");
    expect(card(/tado°/)).not.toHaveTextContent(/Required|Recommended/);
    // The group says it is one of each, not every one of them.
    expect(dialog).toHaveTextContent(
      "Niles needs one speech-to-text provider and one AI provider.",
    );
  });

  it("stops calling the other providers required once a role is covered", () => {
    // Groq answers and transcribes, so Cerebras is a choice, not a gap.
    setup([{ ...GROQ, added: true }, CEREBRAS]);
    const cerebras = within(gallery()).getByRole("button", { name: /Cerebras/ });
    expect(cerebras).not.toHaveTextContent("Required");
  });

  it("puts the more important cards first within a group", () => {
    setup([TADO, UNIFI]);
    const dialog = gallery();
    const presence = within(dialog).getByRole("region", { name: "Presence & climate" });
    const names = within(presence)
      .getAllByRole("button")
      .map((b) => b.textContent ?? "");
    expect(names[0]).toMatch(/^UniFi/);
    expect(names[1]).toMatch(/^tado°/);
  });

  it("opens the setup flow for the card picked", () => {
    const { onChange } = setup([UNIFI, NEMLIG], {
      writable: true,
      secrets: [...UNIFI_KEY.secrets, ...NEMLIG_LOGIN.secrets],
    });
    fireEvent.click(within(gallery()).getByRole("button", { name: /nemlig.com/ }));
    expect(screen.queryByRole("dialog", { name: "Add an integration" })).toBeNull();
    expect(onChange).toHaveBeenCalledWith("integrations.nemlig", [
      { path: "integrations.nemlig.enabled", value: true },
    ]);
    expect(screen.getByText("nemlig.com password")).toBeInTheDocument();
    expect(screen.queryByLabelText("Console address")).toBeNull();
  });

  it("takes the broker's card to its Settings page", () => {
    const onOpenSettings = vi.fn();
    setup([MQTT], SECRETS, undefined, onOpenSettings);
    fireEvent.click(within(gallery()).getByRole("button", { name: /Zigbee & MQTT/ }));
    expect(onOpenSettings).toHaveBeenCalledWith("mqtt");
  });

  it("says where the broker is set up when it cannot go there itself", () => {
    // A button that did nothing would be worse than a card that says.
    setup([MQTT]);
    const dialog = gallery();
    expect(within(dialog).queryByRole("button", { name: /Zigbee & MQTT/ })).toBeNull();
    expect(dialog).toHaveTextContent("In Settings → Zigbee & MQTT");
  });
});

describe("IntegrationsPage removing", () => {
  it("deletes the keys it saved, and leaves ones from the environment", async () => {
    const clear = vi.spyOn(api, "clearSecret").mockResolvedValue(undefined);
    const { onReset, onSecretsChanged } = setup([{ ...NEMLIG, added: true }], {
      writable: true,
      secrets: [
        { ...NEMLIG_LOGIN.secrets[0], source: "environment" },
        { ...NEMLIG_LOGIN.secrets[1], source: "stored" },
      ],
    });
    fireEvent.click(screen.getByRole("button", { name: /nemlig.com/ }));
    expect(screen.getByText("Its saved keys are deleted too.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Remove nemlig.com/ }));

    await waitFor(() =>
      expect(onReset).toHaveBeenCalledWith("integrations.nemlig", ["integrations.nemlig"]),
    );
    expect(clear).toHaveBeenCalledTimes(1);
    expect(clear).toHaveBeenCalledWith("integrations.nemlig.password");
    expect(onSecretsChanged).toHaveBeenCalled();
  });

  it("deletes a provider's key before the provider, while the server still accepts it", async () => {
    const order: string[] = [];
    vi.spyOn(api, "clearSecret").mockImplementation(async (key) => {
      order.push(`clear ${key}`);
    });
    const { onChange } = setup([{ ...GROQ, added: true }], {
      writable: true,
      secrets: [{ ...SECRETS.secrets[0], source: "stored" }],
    });
    onChange.mockImplementation((row: string) => order.push(`change ${row}`));
    fireEvent.click(screen.getByRole("button", { name: "Configure Groq" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove Groq/ }));
    await waitFor(() =>
      expect(order).toEqual(["clear provider.groq.api_key", "change providers"]),
    );
  });

  it("keeps it when a key cannot be deleted, and says why", async () => {
    vi.spyOn(api, "clearSecret").mockRejectedValue(new Error("no database"));
    const { onReset } = setup([{ ...NEMLIG, added: true }], {
      writable: true,
      secrets: NEMLIG_LOGIN.secrets.map((s) => ({ ...s, source: "stored" as const })),
    });
    fireEvent.click(screen.getByRole("button", { name: /nemlig.com/ }));
    fireEvent.click(screen.getByRole("button", { name: /Remove nemlig.com/ }));
    expect(await screen.findByText("no database")).toBeInTheDocument();
    expect(onReset).not.toHaveBeenCalled();
  });

  it("says nothing about keys when it has none saved", () => {
    setup([{ ...NEMLIG, added: true }], NEMLIG_LOGIN);
    fireEvent.click(screen.getByRole("button", { name: /nemlig.com/ }));
    expect(screen.queryByText(/saved keys are deleted/)).toBeNull();
  });
});
