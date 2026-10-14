import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { IntegrationsPage } from "@/components/IntegrationsPage";
import type { IntegrationsPageProps } from "@/components/IntegrationsPage";
import { api } from "@/lib/api";
import { matchesQuery, statusOf } from "@/lib/integrations";
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
  kind: "service",
  group: ESSENTIALS,
  importance: "required",
  icon: "network",
  secret_keys: ["mqtt.username", "mqtt.password"],
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
  extra: Partial<IntegrationsPageProps> = {},
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
      {...extra}
    />,
  );
  return { onChange, onReset, onSecretsChanged };
}

afterEach(() => {
  vi.restoreAllMocks();
});

function tab(name: "Connected" | "Available") {
  return screen.getByRole("tab", { name: new RegExp(`^${name}`) });
}

function search(text: string) {
  fireEvent.change(screen.getByRole("searchbox", { name: "Search integrations" }), {
    target: { value: text },
  });
}

describe("IntegrationsPage tabs", () => {
  it("lands on Connected when something is added", () => {
    setup([{ ...GROQ, added: true }, TADO]);
    expect(tab("Connected")).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("button", { name: "Configure Groq" })).toBeInTheDocument();
    expect(screen.queryByText("tado°")).toBeNull();
  });

  it("lands on Available on a new install", () => {
    // Someone setting Niles up for the first time starts from what they
    // can add, not from an empty list.
    setup([GROQ, TADO]);
    expect(tab("Available")).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("button", { name: "Connect Groq" })).toBeInTheDocument();
  });

  it("counts each side in its tab", () => {
    setup([{ ...GROQ, added: true }, CEREBRAS, TADO]);
    expect(tab("Connected")).toHaveAccessibleName("Connected 1");
    expect(tab("Available")).toHaveAccessibleName("Available 2");
  });

  it("offers what is not added under Available", () => {
    setup([{ ...GROQ, added: true }, TADO]);
    fireEvent.click(tab("Available"));
    expect(screen.getByRole("button", { name: "Connect tado°" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect Groq" })).toBeNull();
  });

  it("says so when everything is connected", () => {
    setup([{ ...GROQ, added: true }]);
    fireEvent.click(tab("Available"));
    expect(screen.getByText("Everything Niles knows is connected.")).toBeInTheDocument();
  });
});

describe("IntegrationsPage", () => {
  it("says what is missing from an integration added without a key", () => {
    // A row that looks done and answers nothing is worse than one that
    // says what is left to do.
    setup([{ ...GROQ, added: true }]);
    expect(screen.getByText("Needs its API key")).toBeInTheDocument();
  });

  it("says it works once the key is there", () => {
    setup([{ ...GROQ, added: true }], {
      writable: true,
      secrets: [{ ...SECRETS.secrets[0], source: "stored" }],
    });
    expect(screen.getByText("Working")).toBeInTheDocument();
    expect(screen.queryByText("Needs its API key")).toBeNull();
  });

  it("adds a provider without asking for its endpoint, and opens it", () => {
    // Which is the whole difference from the pair of text boxes this
    // replaces: Niles already knows where Groq's API lives, and typing
    // a name and a URL let you invent a provider that cannot work.
    const { onChange } = setup([GROQ]);
    fireEvent.click(screen.getByRole("button", { name: "Connect Groq" }));
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
    expect(screen.getByLabelText("groq API key")).toBeInTheDocument();
  });

  it("keeps the providers already added when adding another", () => {
    const { onChange } = setup([{ ...GROQ, added: true }, CEREBRAS]);
    fireEvent.click(tab("Available"));
    fireEvent.click(screen.getByRole("button", { name: "Connect Cerebras" }));
    const value = onChange.mock.calls[0][1][0].value as { name: string }[];
    expect(value.map((p) => p.name)).toEqual(["groq", "cerebras"]);
  });

  it("keeps the key out of the list and behind the row", () => {
    // The list answers "what is connected". A key is setup detail, and
    // true at a glance is not the same as useful at a glance.
    setup([{ ...GROQ, added: true }]);
    expect(screen.queryByLabelText("groq API key")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Configure Groq" }));
    expect(screen.getByLabelText("groq API key")).toBeInTheDocument();
  });

  it("says what it does and what it takes in its dialog, once", () => {
    setup([{ ...GROQ, added: true }]);
    fireEvent.click(screen.getByRole("button", { name: "Configure Groq" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getAllByText(GROQ.blurb)).toHaveLength(1);
    expect(within(dialog).getByText(GROQ.needs)).toBeInTheDocument();
  });

  it("removes a provider by leaving it out of the list", () => {
    const { onChange } = setup([{ ...GROQ, added: true }]);
    fireEvent.click(screen.getByRole("button", { name: "Configure Groq" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove Groq/ }));
    expect(onChange).toHaveBeenCalledWith("providers", [
      { path: "providers", value: [] },
    ]);
  });

  it("says where to start when nothing is connected", () => {
    setup([GROQ, TADO]);
    fireEvent.click(tab("Connected"));
    expect(screen.getByText(/Nothing connected yet/)).toBeInTheDocument();
  });

  it("asks for the console's address and key when UniFi is added", () => {
    const { onChange } = setup([UNIFI], UNIFI_KEY);
    fireEvent.click(screen.getByRole("button", { name: "Connect UniFi" }));

    // Adding it writes nothing: an empty address is not a console.
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Console address")).toBeTruthy();
    expect(screen.getByText("UniFi console API key")).toBeTruthy();
  });

  it("saves the address when you leave the field", () => {
    const { onChange } = setup([{ ...UNIFI, added: true }], UNIFI_KEY, "");
    expect(screen.getByText("Needs its address")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Configure UniFi" }));
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
    fireEvent.click(screen.getByRole("button", { name: "Configure UniFi" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove UniFi/ }));

    expect(onChange).toHaveBeenCalledWith("presence.unifi", [
      { path: "presence.unifi.host", value: "" },
    ]);
  });

  it("switches Claude Code on and asks for its token", () => {
    const { onChange } = setup([CLAUDE], CLAUDE_TOKEN);
    fireEvent.click(screen.getByRole("button", { name: "Connect Claude Code" }));

    expect(onChange).toHaveBeenCalledWith("integrations.claude_code", [
      { path: "integrations.claude_code.enabled", value: true },
    ]);
    expect(screen.getByText(/claude setup-token/, { selector: "div, span, label" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: /Sonnet/ })).toHaveAttribute("aria-checked", "true");
  });

  it("picks Claude Code's model", () => {
    const { onChange } = setup([{ ...CLAUDE, added: true }], CLAUDE_TOKEN);
    fireEvent.click(screen.getByRole("button", { name: "Configure Claude Code" }));
    fireEvent.click(screen.getByRole("radio", { name: /Opus/ }));

    expect(onChange).toHaveBeenCalledWith("integrations.claude_code", [
      { path: "integrations.claude_code.model", value: "opus" },
    ]);
  });

  it("switches Claude Code off without removing it", () => {
    const { onChange } = setup([{ ...CLAUDE, added: true }], CLAUDE_TOKEN);
    fireEvent.click(screen.getByRole("button", { name: "Configure Claude Code" }));
    fireEvent.click(screen.getByRole("switch"));

    expect(onChange).toHaveBeenCalledWith("integrations.claude_code", [
      { path: "integrations.claude_code.enabled", value: false },
    ]);
  });

  it("removing Claude Code takes the section away", () => {
    const { onChange, onReset } = setup([{ ...CLAUDE, added: true }], CLAUDE_TOKEN);
    fireEvent.click(screen.getByRole("button", { name: "Configure Claude Code" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove Claude Code/ }));

    expect(onReset).toHaveBeenCalledWith("integrations.claude_code", [
      "integrations.claude_code",
    ]);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("switches nemlig.com on and asks for both halves of the login", () => {
    const { onChange } = setup([NEMLIG], NEMLIG_LOGIN);
    fireEvent.click(screen.getByRole("button", { name: "Connect nemlig.com" }));
    expect(onChange).toHaveBeenCalledWith("integrations.nemlig", [
      { path: "integrations.nemlig.enabled", value: true },
    ]);
    expect(screen.getByText("nemlig.com email")).toBeTruthy();
    expect(screen.getByText("nemlig.com password")).toBeTruthy();
  });

  it("switches nemlig.com off without forgetting the login", () => {
    const { onChange, onReset } = setup([{ ...NEMLIG, added: true }], NEMLIG_LOGIN);
    fireEvent.click(screen.getByRole("button", { name: "Configure nemlig.com" }));
    fireEvent.click(screen.getByRole("switch"));
    expect(onChange).toHaveBeenCalledWith("integrations.nemlig", [
      { path: "integrations.nemlig.enabled", value: false },
    ]);
    expect(onReset).not.toHaveBeenCalled();
  });

  it("removing nemlig.com takes the section away", () => {
    const { onReset } = setup([{ ...NEMLIG, added: true }], NEMLIG_LOGIN);
    fireEvent.click(screen.getByRole("button", { name: "Configure nemlig.com" }));
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

  it("offers what is not added under the same groups, Essentials first", () => {
    setup([MQTT, GROQ, CEREBRAS, UNIFI, TADO, NEMLIG, CLAUDE]);
    const headings = screen
      .getAllByRole("heading", { level: 3 })
      .map((h) => h.textContent);
    expect(headings).toEqual([
      "Essentials",
      "Presence & climate",
      "Shopping",
      "Assistants & tools",
    ]);
    // The group says it is one of each, not every one of them.
    expect(screen.getByRole("region", { name: "Essentials" })).toHaveTextContent(
      "Niles needs one speech-to-text provider and one AI provider.",
    );
  });

  it("shows each service's own logo, and an icon for the broker", () => {
    setup([MQTT, GROQ, NEMLIG]);
    const logo = (name: string) =>
      screen
        .getByRole("button", { name: `Connect ${name}` })
        .parentElement?.querySelector("[data-logo]");
    expect(logo("Groq")).toHaveAttribute("data-logo", "brand");
    expect(logo("nemlig.com")).toHaveAttribute("data-logo", "brand");
    expect(logo("Zigbee & MQTT")).toHaveAttribute("data-logo", "icon");
  });

  it("badges Required where it applies", () => {
    setup([MQTT, GROQ, CEREBRAS, UNIFI, TADO]);
    const row = (name: string) =>
      screen.getByRole("button", { name: `Connect ${name}` }).parentElement!;
    expect(row("Zigbee & MQTT")).toHaveTextContent("Required");
    expect(row("Groq")).toHaveTextContent("Required");
    expect(row("UniFi")).not.toHaveTextContent("Required");
    expect(row("tado°")).not.toHaveTextContent("Required");
  });

  it("stops calling the other providers required once a role is covered", () => {
    // Groq answers and transcribes, so Cerebras is a choice, not a gap.
    setup([{ ...GROQ, added: true }, CEREBRAS]);
    fireEvent.click(tab("Available"));
    const cerebras = screen.getByRole("button", { name: "Connect Cerebras" }).parentElement!;
    expect(cerebras).not.toHaveTextContent("Required");
    expect(screen.getByRole("region", { name: "Essentials" })).toHaveTextContent(
      "AI: Groq",
    );
  });

  it("puts the more important ones first within a group", () => {
    setup([TADO, UNIFI]);
    const presence = screen.getByRole("region", { name: "Presence & climate" });
    const names = within(presence)
      .getAllByRole("button")
      .map((b) => b.getAttribute("aria-label"));
    expect(names).toEqual(["Connect UniFi", "Connect tado°"]);
  });

  it("sets the broker up from its row, asking for its address first", () => {
    setup([MQTT]);
    fireEvent.click(screen.getByRole("button", { name: "Connect Zigbee & MQTT" }));
    expect(screen.getByLabelText("Broker address")).toBeInTheDocument();
  });

  it("sets the broker up in its own dialog, login and all", () => {
    // Once, here: there is no Settings page for it as well.
    const withLogin: SecretsReport = {
      writable: true,
      secrets: [
        { key: "mqtt.username", label: "Zigbee2MQTT broker username", source: "unset" },
        { key: "mqtt.password", label: "Zigbee2MQTT broker password", source: "unset" },
      ],
    };
    const { onChange } = setup([{ ...MQTT, added: true }], withLogin, undefined, {
      mqtt: { host: "10.0.0.2" },
      mqttDefaults: { port: 1883, z2m_prefix: "zigbee2mqtt", client_id: "niles" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Configure Zigbee & MQTT" }));
    const host = screen.getByLabelText("Broker address");
    expect(host).toHaveValue("10.0.0.2");
    expect(screen.getByLabelText("Port")).toHaveAttribute("placeholder", "1883");
    expect(screen.getByLabelText("Zigbee2MQTT broker password")).toBeInTheDocument();
    expect(screen.getByText(/take effect when Niles restarts/)).toBeInTheDocument();

    fireEvent.change(host, { target: { value: "mosquitto.local" } });
    fireEvent.blur(host);
    expect(onChange).toHaveBeenCalledWith("mqtt", [
      { path: "mqtt.host", value: "mosquitto.local" },
    ]);
    const port = screen.getByLabelText("Port");
    fireEvent.change(port, { target: { value: "8883" } });
    fireEvent.blur(port);
    expect(onChange).toHaveBeenCalledWith("mqtt", [{ path: "mqtt.port", value: 8883 }]);
  });
});

describe("IntegrationsPage search", () => {
  it("is not a box a browser fills in with a saved login", () => {
    setup([{ ...GROQ, added: true }]);
    const box = screen.getByRole("searchbox", { name: "Search integrations" });
    expect(box).toHaveAttribute("autocomplete", "off");
    expect(box).toHaveAttribute("name", "integration-search");
  });

  const EVERYTHING = [MQTT, { ...GROQ, added: true }, CEREBRAS, UNIFI, TADO, NEMLIG, CLAUDE];

  it("narrows the tab to what matches, hiding empty groups", () => {
    setup(EVERYTHING);
    fireEvent.click(tab("Available"));
    // "Who is home" is in both presence blurbs and nowhere else.
    search("WHO IS HOME");
    expect(screen.getByRole("button", { name: "Connect UniFi" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect tado°" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect nemlig.com" })).toBeNull();
    expect(
      screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent),
    ).toEqual(["Presence & climate"]);
  });

  it("finds by group name", () => {
    setup(EVERYTHING);
    fireEvent.click(tab("Available"));
    search("shopping");
    expect(screen.getByRole("button", { name: "Connect nemlig.com" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect UniFi" })).toBeNull();
  });

  it("says when nothing matches", () => {
    setup(EVERYTHING);
    search("  zigbee2000 ");
    expect(screen.getByText("Nothing matches “zigbee2000”.")).toBeInTheDocument();
    expect(screen.queryAllByRole("region")).toHaveLength(0);
  });

  it("points to a match on the other tab", () => {
    setup(EVERYTHING);
    search("cerebras");
    expect(screen.getByText("Nothing matches “cerebras”.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cerebras is under Available" }));
    expect(tab("Available")).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("button", { name: "Connect Cerebras" })).toBeInTheDocument();
  });
});

describe("matchesQuery", () => {
  it("matches label, blurb and group, ignoring case", () => {
    expect(matchesQuery(GROQ, "groq")).toBe(true);
    expect(matchesQuery(GROQ, "LANGUAGE MODELS")).toBe(true);
    expect(matchesQuery(GROQ, "essent")).toBe(true);
    expect(matchesQuery(GROQ, "  ")).toBe(true);
    expect(matchesQuery(GROQ, "thermostat")).toBe(false);
  });
});

describe("statusOf", () => {
  const unset = (key: string) => ({ key, label: key, source: "unset" as const });
  const stored = (key: string) => ({ key, label: key, source: "stored" as const });

  it("says a provider without its key needs one", () => {
    const status = statusOf(
      { ...GROQ, added: true },
      { secrets: { writable: true, secrets: [unset("provider.groq.api_key")] } },
    );
    expect(status).toEqual({ ok: false, text: "Needs its API key" });
  });

  it("says a finished one is working", () => {
    const status = statusOf(
      { ...GROQ, added: true },
      { secrets: { writable: true, secrets: [stored("provider.groq.api_key")] } },
    );
    expect(status).toEqual({ ok: true, text: "Working" });
  });

  it("says switched off before anything else is missing", () => {
    const status = statusOf(
      { ...NEMLIG, added: true },
      { nemlig: { enabled: false }, secrets: NEMLIG_LOGIN },
    );
    expect(status).toEqual({ ok: false, text: "Switched off" });
  });

  it("names what each kind is waiting for", () => {
    expect(statusOf({ ...TADO, added: true }, {}).text).toBe("Not signed in to tado° yet");
    expect(statusOf({ ...UNIFI, added: true }, { unifiHost: " " }).text).toBe(
      "Needs its address",
    );
    expect(
      statusOf({ ...TADO, id: "linear", label: "Linear", added: true }, { linear: { team: "" } })
        .text,
    ).toBe("Needs a team");
    expect(
      statusOf(
        { ...TADO, id: "lg_tv", label: "LG TV", added: true },
        {
          tv: { configured: true, paired: false, mac: null, room: null, status: null, error: null },
        },
      ).text,
    ).toBe("TV not paired yet");
    expect(
      statusOf(
        { ...TADO, id: "spotify", label: "Spotify", added: true },
        {
          secrets: {
            writable: true,
            secrets: [
              stored("integrations.spotify.client_id"),
              unset("integrations.spotify.client_secret"),
            ],
          },
        },
      ).text,
    ).toBe("Needs its client ID and secret");
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
    fireEvent.click(screen.getByRole("button", { name: "Configure nemlig.com" }));
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
    fireEvent.click(screen.getByRole("button", { name: "Configure nemlig.com" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove nemlig.com/ }));
    expect(await screen.findByText("no database")).toBeInTheDocument();
    expect(onReset).not.toHaveBeenCalled();
  });

  it("says nothing about keys when it has none saved", () => {
    setup([{ ...NEMLIG, added: true }], NEMLIG_LOGIN);
    fireEvent.click(screen.getByRole("button", { name: "Configure nemlig.com" }));
    expect(screen.queryByText(/saved keys are deleted/)).toBeNull();
  });
});
