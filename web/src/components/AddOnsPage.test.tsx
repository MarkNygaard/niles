import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AddOnsPage } from "@/components/AddOnsPage";
import type { AddOnsPageProps } from "@/components/AddOnsPage";
import { api } from "@/lib/api";
import { matchesQuery, statusOf } from "@/lib/integrations";
import { DEFAULT_KEEP_DAYS } from "@/lib/podcasts";
import type { Integration, IntegrationGroup, SecretsReport, ToolServer } from "@/lib/api";

const SYSTEM: IntegrationGroup = {
  id: "system",
  label: "System",
  blurb: "What Niles needs.",
  requirement: "Niles needs one hearing provider and one thinking provider.",
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

const CEREBRAS: Integration = {
  id: "cerebras",
  label: "Cerebras",
  blurb: "Works out the answer.",
  needs: "A Cerebras account and an API key.",
  kind: "provider",
  group: SYSTEM,
  importance: "required",
  system: true,
  default_for: ["llm"],
  icon: "brain",
  secret_keys: ["provider.cerebras.api_key"],
  base_url: "https://api.cerebras.ai/v1",
  serves: ["llm"],
  added: false,
  secret_key: "provider.cerebras.api_key",
};

const ELEVENLABS: Integration = {
  ...CEREBRAS,
  id: "elevenlabs",
  label: "ElevenLabs",
  blurb: "Hears what you say.",
  default_for: ["stt"],
  secret_keys: ["provider.elevenlabs.api_key"],
  base_url: "https://api.elevenlabs.io/v1",
  serves: ["stt"],
  secret_key: "provider.elevenlabs.api_key",
};

const MQTT: Integration = {
  id: "mqtt",
  label: "Zigbee & MQTT",
  blurb: "How Niles reaches your lights.",
  needs: "An MQTT broker.",
  kind: "service",
  group: SYSTEM,
  importance: "required",
  system: true,
  default_for: [],
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
  system: false,
  default_for: [],
  icon: "thermometer",
  secret_keys: [],
  base_url: null,
  serves: [],
  added: false,
  secret_key: null,
};

const UNIFI: Integration = {
  ...TADO,
  id: "unifi",
  label: "UniFi",
  blurb: "Who is home, from the Wi-Fi.",
  needs: "A UniFi console.",
  importance: "recommended",
  icon: "wifi",
  secret_keys: ["presence.unifi.api_key"],
  secret_key: "presence.unifi.api_key",
};

function unifiKey(source: "stored" | "unset" | "environment" = "unset"): SecretsReport {
  return {
    writable: true,
    secrets: [{ key: "presence.unifi.api_key", label: "UniFi console API key", source }],
  };
}

const CLAUDE: Integration = {
  ...TADO,
  id: "claude_code",
  label: "Claude Code",
  blurb: "Answers the chat with Claude, on your Claude subscription.",
  needs: "A Claude subscription.",
  group: ASSISTANTS,
  icon: "bot",
  secret_keys: ["integrations.claude_code.oauth_token"],
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

const CHATGPT: Integration = {
  ...CLAUDE,
  id: "chatgpt",
  label: "ChatGPT",
  blurb: "ChatGPT answers the chat and handles new rules.",
  needs: "A paid ChatGPT subscription.",
  secret_keys: [
    "integrations.chatgpt.access_token",
    "integrations.chatgpt.refresh_token",
    "integrations.chatgpt.account_id",
    "integrations.chatgpt.expires_at",
  ],
  secret_key: "integrations.chatgpt.access_token",
};

function chatgptSignIn(source: "stored" | "unset"): SecretsReport {
  return {
    writable: true,
    secrets: CHATGPT.secret_keys.map((key) => ({ key, label: key, hint: "chatgpt.com", source })),
  };
}

const NEMLIG: Integration = {
  ...TADO,
  id: "nemlig",
  label: "nemlig.com",
  blurb: "Groceries delivered.",
  needs: "A nemlig.com account.",
  group: SHOPPING,
  icon: "shopping-basket",
  secret_keys: ["integrations.nemlig.username", "integrations.nemlig.password"],
  secret_key: "integrations.nemlig.password",
};

const NEMLIG_LOGIN: SecretsReport = {
  writable: true,
  secrets: [
    { key: "integrations.nemlig.username", label: "nemlig.com email", hint: "nemlig.com", source: "unset" },
    { key: "integrations.nemlig.password", label: "nemlig.com password", hint: "nemlig.com", source: "unset" },
  ],
};

const LINEAR: Integration = {
  ...TADO,
  id: "linear",
  label: "Linear",
  blurb: "Tasks.",
  needs: "A Linear API key.",
  icon: "list-todo",
  secret_keys: ["tool_servers.linear.api_key"],
  secret_key: "tool_servers.linear.api_key",
};

const WEB_SEARCH: Integration = {
  ...TADO,
  id: "web_search",
  label: "Web search (SearXNG)",
  blurb: "Your own SearXNG.",
  needs: "A SearXNG instance.",
  group: ASSISTANTS,
  icon: "search",
};

const PODCASTS: Integration = {
  ...TADO,
  id: "podcasts",
  label: "Podcasts",
  blurb: "A researched two-host episode, voiced and kept at your place.",
  needs: "Kokoro and a private Cloudflare R2 bucket.",
  group: ASSISTANTS,
  icon: "podcast",
  secret_keys: [
    "integrations.podcasts.access_key_id",
    "integrations.podcasts.secret_access_key",
  ],
  secret_key: "integrations.podcasts.access_key_id",
};

const PODCASTS_KEYS: SecretsReport = {
  writable: true,
  secrets: [
    {
      key: "integrations.podcasts.access_key_id",
      label: "R2 access key ID",
      hint: "cloudflare.com",
      source: "stored",
    },
    {
      key: "integrations.podcasts.secret_access_key",
      label: "R2 secret access key",
      hint: "cloudflare.com",
      source: "stored",
    },
  ],
};

const PODCASTS_SET = {
  kokoro_url: "http://kokoro.ns.svc:8880",
  endpoint: "https://abc123.r2.cloudflarestorage.com",
  bucket: "niles-podcasts",
};

function setup(
  integrations: Integration[],
  secrets: SecretsReport = unifiKey(),
  unifiHost?: string,
  extra: Partial<AddOnsPageProps> = {},
) {
  const onChange = vi.fn();
  const onReset = vi.fn();
  const onSecretsChanged = vi.fn();
  render(
    <AddOnsPage
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
  fireEvent.change(screen.getByRole("searchbox", { name: "Search add-ons" }), {
    target: { value: text },
  });
}

const UNIFI_ADDED = { ...UNIFI, added: true };

describe("AddOnsPage tabs", () => {
  it("lands on Connected when something is added", () => {
    setup([UNIFI_ADDED, TADO], unifiKey(), "192.168.1.1");
    expect(tab("Connected")).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("button", { name: "Configure UniFi" })).toBeInTheDocument();
    expect(screen.queryByText("tado°")).toBeNull();
  });

  it("lands on Available on a new install", () => {
    // Someone setting Niles up for the first time starts from what they
    // can add, not from an empty list.
    setup([UNIFI, TADO]);
    expect(tab("Available")).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("button", { name: "Connect UniFi" })).toBeInTheDocument();
  });

  it("counts each side in its tab", () => {
    setup([UNIFI_ADDED, NEMLIG, TADO]);
    expect(tab("Connected")).toHaveAccessibleName("Connected 1");
    expect(tab("Available")).toHaveAccessibleName("Available 2");
  });

  it("offers what is not added under Available", () => {
    setup([UNIFI_ADDED, TADO]);
    fireEvent.click(tab("Available"));
    expect(screen.getByRole("button", { name: "Connect tado°" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect UniFi" })).toBeNull();
  });

  it("says so when every add-on is connected", () => {
    setup([UNIFI_ADDED]);
    fireEvent.click(tab("Available"));
    expect(screen.getByText("Every add-on Niles ships is connected.")).toBeInTheDocument();
  });
});

describe("AddOnsPage and the base system", () => {
  it("leaves out the broker and the voice providers: they are under System", () => {
    setup([MQTT, ELEVENLABS, CEREBRAS, TADO]);
    expect(tab("Available")).toHaveAccessibleName("Available 1");
    expect(screen.queryByText("Zigbee & MQTT")).toBeNull();
    expect(screen.queryByText("ElevenLabs")).toBeNull();
    expect(screen.queryByText("Cerebras")).toBeNull();
    expect(screen.queryByRole("region", { name: "System" })).toBeNull();
  });

  it("does not count a set-up system as a connected add-on", () => {
    // A house with only its broker and providers has no add-ons yet,
    // and lands where it can add one.
    setup([{ ...MQTT, added: true }, { ...CEREBRAS, added: true }, TADO]);
    expect(tab("Connected")).toHaveAccessibleName("Connected 0");
    expect(tab("Available")).toHaveAttribute("aria-selected", "true");
  });

  it("finds nothing of the system when searched for", () => {
    setup([MQTT, CEREBRAS, UNIFI, TADO]);
    search("cerebras");
    expect(screen.getByText("Nothing matches “cerebras”.")).toBeInTheDocument();
  });
});

describe("AddOnsPage", () => {
  it("says what is missing from an add-on added without its key", () => {
    // A row that looks done and does nothing is worse than one that
    // says what is left to do.
    setup([UNIFI_ADDED], unifiKey("unset"), "192.168.1.1");
    expect(screen.getByText("Needs its API key")).toBeInTheDocument();
  });

  it("says it works once the key is there", () => {
    setup([UNIFI_ADDED], unifiKey("stored"), "192.168.1.1");
    expect(screen.getByText("Working")).toBeInTheDocument();
    expect(screen.queryByText("Needs its API key")).toBeNull();
  });

  it("keeps the key out of the list and behind the row", () => {
    // The list answers "what is connected". A key is setup detail.
    setup([UNIFI_ADDED], unifiKey(), "192.168.1.1");
    expect(screen.queryByLabelText("UniFi console API key")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Configure UniFi" }));
    expect(screen.getByLabelText("UniFi console API key")).toBeInTheDocument();
  });

  it("says what it does and what it takes in its dialog, once", () => {
    setup([UNIFI_ADDED], unifiKey(), "192.168.1.1");
    fireEvent.click(screen.getByRole("button", { name: "Configure UniFi" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getAllByText(UNIFI.blurb)).toHaveLength(1);
    expect(within(dialog).getByText(UNIFI.needs)).toBeInTheDocument();
  });

  it("says where to start when nothing is connected", () => {
    setup([UNIFI, TADO]);
    fireEvent.click(tab("Connected"));
    expect(screen.getByText(/No add-ons yet/)).toBeInTheDocument();
  });

  it("asks for the console's address and key when UniFi is added", () => {
    const { onChange } = setup([UNIFI], unifiKey());
    fireEvent.click(screen.getByRole("button", { name: "Connect UniFi" }));

    // Adding it writes nothing: an empty address is not a console.
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Console address")).toBeTruthy();
    expect(screen.getByText("UniFi console API key")).toBeTruthy();
  });

  it("saves the address when you leave the field", () => {
    const { onChange } = setup([UNIFI_ADDED], unifiKey(), "");
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
    const { onChange } = setup([UNIFI_ADDED], unifiKey(), "192.168.1.1");
    fireEvent.click(screen.getByRole("button", { name: "Configure UniFi" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));

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
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));

    expect(onReset).toHaveBeenCalledWith("integrations.claude_code", [
      "integrations.claude_code",
    ]);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("connects Linear to Linear's own MCP server and asks for its key", () => {
    const { onChange } = setup([LINEAR], {
      writable: true,
      secrets: [{ key: "tool_servers.linear.api_key", label: "Linear API key", source: "unset" }],
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect Linear" }));
    expect(onChange).toHaveBeenCalledWith("tool_servers.linear", [
      {
        path: "tool_servers.linear",
        value: {
          url: "https://mcp.linear.app/mcp",
          name: "Linear",
          keywords: ["linear", "issue", "issues", "task", "tasks", "ticket", "tickets"],
        },
      },
    ]);
    expect(screen.getByText("Linear API key")).toBeTruthy();
  });

  it("asks for the Linear webhook's signing secret and says where to send it", () => {
    setup([{ ...LINEAR, added: true }], {
      writable: true,
      secrets: [
        { key: "tool_servers.linear.api_key", label: "Linear API key", source: "stored" },
        {
          key: "linear.webhook_secret",
          label: "Linear webhook signing secret",
          source: "unset",
        },
      ],
    });
    fireEvent.click(screen.getByRole("button", { name: "Configure Linear" }));
    expect(screen.getByLabelText("Linear webhook signing secret")).toBeTruthy();
    expect(screen.getByText(`${window.location.origin}/webhooks/linear`)).toBeTruthy();
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
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));
    expect(onReset).toHaveBeenCalledWith("integrations.nemlig", ["integrations.nemlig"]);
  });
});

describe("AddOnsPage web search", () => {
  it("asks for the address when it is added, and writes nothing yet", () => {
    const tested = vi.spyOn(api, "testWebSearch");
    const { onChange } = setup([WEB_SEARCH], unifiKey());
    fireEvent.click(screen.getByRole("button", { name: "Connect Web search (SearXNG)" }));
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByLabelText("SearXNG address")).toBeTruthy();
    expect(screen.getByText(/ChatGPT can't search the web without SearXNG/)).toBeTruthy();
    expect(tested).not.toHaveBeenCalled();
  });

  it("is in use on its row once it has an address, from the app or a ConfigMap", () => {
    vi.spyOn(api, "testWebSearch").mockResolvedValue({
      configured: true,
      web: { ok: true, results: 3, error: null },
      images: { ok: true, results: 0, error: null },
    });
    setup([{ ...WEB_SEARCH, added: true }], unifiKey(), undefined, {
      webSearchAddress: "https://search.example/search",
    });
    const row = screen.getByRole("button", { name: "Configure Web search (SearXNG)" });
    expect(within(row).getByText("In use")).toBeTruthy();
  });

  it("says when the one added is not reachable", async () => {
    vi.spyOn(api, "testWebSearch").mockResolvedValue({
      configured: true,
      web: { ok: false, results: 0, error: "SearXNG did not answer within 15s." },
      images: { ok: false, results: 0, error: "SearXNG did not answer within 15s." },
    });
    setup([{ ...WEB_SEARCH, added: true }], unifiKey(), undefined, {
      webSearchAddress: "https://search.example/search",
    });
    fireEvent.click(screen.getByRole("button", { name: "Configure Web search (SearXNG)" }));
    expect(
      await screen.findByText("Not reachable: SearXNG did not answer within 15s."),
    ).toBeTruthy();
  });

  it("removing it empties the address, so Claude goes back to its own search", () => {
    vi.spyOn(api, "testWebSearch").mockResolvedValue({
      configured: true,
      web: { ok: true, results: 3, error: null },
      images: { ok: true, results: 2, error: null },
    });
    const { onChange, onReset } = setup([{ ...WEB_SEARCH, added: true }], unifiKey(), undefined, {
      webSearchAddress: "https://search.example/search",
    });
    fireEvent.click(screen.getByRole("button", { name: "Configure Web search (SearXNG)" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));
    expect(onChange).toHaveBeenCalledWith("web_search", [
      { path: "web_search.base_url", value: "" },
    ]);
    expect(onReset).not.toHaveBeenCalled();
  });
});

describe("AddOnsPage assistants", () => {
  it("switches ChatGPT on and opens it to sign in", () => {
    const { onChange } = setup([CHATGPT], chatgptSignIn("unset"), undefined, {
      chatgptStatus: { signed_in: false, email: null, expires_at: null, writable: true },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect ChatGPT" }));
    expect(onChange).toHaveBeenCalledWith("integrations.chatgpt", [
      { path: "integrations.chatgpt.enabled", value: true },
    ]);
    expect(screen.getByRole("button", { name: /Sign in with ChatGPT/ })).toBeInTheDocument();
  });

  it("says ChatGPT is not signed in until it is", () => {
    setup([{ ...CHATGPT, added: true }], chatgptSignIn("unset"));
    expect(screen.getByText("Not signed in")).toBeInTheDocument();
  });

  it("picks ChatGPT's model", () => {
    const { onChange } = setup([{ ...CHATGPT, added: true }], chatgptSignIn("stored"), undefined, {
      chatgpt: { enabled: true, model: "gpt-5.6-terra" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Configure ChatGPT" }));
    expect(screen.getByRole("radio", { name: /GPT-5.6 Terra/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    fireEvent.click(screen.getByRole("radio", { name: /GPT-6 Astra/ }));
    expect(onChange).toHaveBeenCalledWith("integrations.chatgpt", [
      { path: "integrations.chatgpt.model", value: "gpt-6-astra" },
    ]);
  });

  it("removing ChatGPT signs it out and takes the section away", async () => {
    const clear = vi.spyOn(api, "clearSecret").mockResolvedValue(undefined);
    const { onReset } = setup([{ ...CHATGPT, added: true }], chatgptSignIn("stored"));
    fireEvent.click(screen.getByRole("button", { name: "Configure ChatGPT" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove add-on" }));
    await waitFor(() =>
      expect(onReset).toHaveBeenCalledWith("integrations.chatgpt", ["integrations.chatgpt"]),
    );
    expect(clear.mock.calls.map(([key]) => key).sort()).toEqual([...CHATGPT.secret_keys].sort());
  });

  it("leaves which one answers to Voice & AI", () => {
    // The choice includes Niles's own model, which is not an add-on, so
    // it lives with the rest of the voice.
    setup([{ ...CLAUDE, added: true }, { ...CHATGPT, added: true }], CLAUDE_TOKEN);
    expect(screen.queryByRole("radiogroup", { name: "Which assistant answers" })).toBeNull();
    expect(
      within(screen.getByRole("region", { name: "Assistants & tools" })).getByText(
        /chosen under System, in Voice & AI/,
      ),
    ).toBeInTheDocument();
  });
});

describe("AddOnsPage groups", () => {
  it("lists what is set up under its group's heading, in the order given", () => {
    setup(
      [{ ...MQTT, added: true }, UNIFI_ADDED, { ...NEMLIG, added: true }, TADO],
      unifiKey(),
      "192.168.1.1",
    );
    const headings = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);
    expect(headings).toEqual(["Presence & climate", "Shopping"]);

    const presence = screen.getByRole("region", { name: "Presence & climate" });
    expect(within(presence).getByText("UniFi")).toBeInTheDocument();
    // Not added, so not listed — its group shows only what is.
    expect(within(presence).queryByText("tado°")).toBeNull();
  });

  it("offers what is not added under the same groups, GitHub last", () => {
    setup([MQTT, ELEVENLABS, CEREBRAS, UNIFI, TADO, NEMLIG, CLAUDE]);
    const headings = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);
    expect(headings).toEqual([
      "Presence & climate",
      "Shopping",
      "Assistants & tools",
      // Last, under a heading of its own.
      "From GitHub",
    ]);
    expect(screen.getByRole("region", { name: "From GitHub" })).toHaveTextContent(
      "Add-on from GitHub",
    );
  });

  it("shows each service's own logo, and an icon where there is none", () => {
    setup([UNIFI, NEMLIG, { ...TADO, id: "something_new", label: "Something new", icon: "speaker" }]);
    const logo = (name: string) =>
      screen
        .getByRole("button", { name: `Connect ${name}` })
        .parentElement?.querySelector("[data-logo]");
    expect(logo("UniFi")).toHaveAttribute("data-logo", "brand");
    expect(logo("nemlig.com")).toHaveAttribute("data-logo", "brand");
    expect(logo("Something new")).toHaveAttribute("data-logo", "icon");
  });

  it("calls none of them Required: what Niles needs is under System", () => {
    setup([UNIFI, TADO, NEMLIG]);
    for (const name of ["UniFi", "tado°", "nemlig.com"]) {
      expect(
        screen.getByRole("button", { name: `Connect ${name}` }).parentElement!,
      ).not.toHaveTextContent("Required");
    }
  });

  it("puts the more important ones first within a group", () => {
    setup([TADO, UNIFI]);
    const presence = screen.getByRole("region", { name: "Presence & climate" });
    const names = within(presence)
      .getAllByRole("button")
      .map((b) => b.getAttribute("aria-label"));
    expect(names).toEqual(["Connect UniFi", "Connect tado°"]);
  });
});

describe("AddOnsPage search", () => {
  it("is not a box a browser fills in with a saved login", () => {
    setup([UNIFI_ADDED]);
    const box = screen.getByRole("searchbox", { name: "Search add-ons" });
    expect(box).toHaveAttribute("autocomplete", "off");
    expect(box).toHaveAttribute("name", "addon-search");
  });

  const EVERYTHING = [MQTT, CEREBRAS, UNIFI_ADDED, TADO, NEMLIG, CLAUDE];

  it("narrows the tab to what matches, hiding empty groups", () => {
    setup(EVERYTHING);
    fireEvent.click(tab("Available"));
    // "Who is home" is in tado's blurb and the presence group's name.
    search("WHO IS HOME");
    expect(screen.getByRole("button", { name: "Connect tado°" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect nemlig.com" })).toBeNull();
    expect(screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent)).toEqual([
      "Presence & climate",
      "From GitHub",
    ]);
  });

  it("finds by group name", () => {
    setup(EVERYTHING);
    fireEvent.click(tab("Available"));
    search("shopping");
    expect(screen.getByRole("button", { name: "Connect nemlig.com" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect tado°" })).toBeNull();
  });

  it("says when nothing matches", () => {
    setup(EVERYTHING);
    search("  zigbee2000 ");
    expect(screen.getByText("Nothing matches “zigbee2000”.")).toBeInTheDocument();
    expect(screen.queryAllByRole("region")).toHaveLength(0);
  });

  it("points to a match on the other tab", () => {
    setup(EVERYTHING);
    search("nemlig");
    expect(screen.getByText("Nothing matches “nemlig”.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "nemlig.com is under Available" }));
    expect(tab("Available")).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("button", { name: "Connect nemlig.com" })).toBeInTheDocument();
  });
});

describe("matchesQuery", () => {
  it("matches label, blurb and group, ignoring case", () => {
    expect(matchesQuery(UNIFI, "unifi")).toBe(true);
    expect(matchesQuery(UNIFI, "THE WI-FI")).toBe(true);
    expect(matchesQuery(UNIFI, "climate")).toBe(true);
    expect(matchesQuery(UNIFI, "  ")).toBe(true);
    expect(matchesQuery(UNIFI, "thermostat")).toBe(false);
  });
});

describe("statusOf", () => {
  const unset = (key: string) => ({ key, label: key, source: "unset" as const });
  const stored = (key: string) => ({ key, label: key, source: "stored" as const });

  it("says one without its key needs one", () => {
    const status = statusOf(
      { ...CEREBRAS, added: true },
      { secrets: { writable: true, secrets: [unset("provider.cerebras.api_key")] } },
    );
    expect(status).toEqual({ ok: false, text: "Needs its API key" });
  });

  it("says a finished one is working", () => {
    const status = statusOf(
      { ...CEREBRAS, added: true },
      { secrets: { writable: true, secrets: [stored("provider.cerebras.api_key")] } },
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
    expect(statusOf(UNIFI_ADDED, { unifiHost: " " }).text).toBe("Needs its address");
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

describe("statusOf Linear", () => {
  const key = (source: "stored" | "unset") => ({
    writable: true,
    secrets: [{ key: "tool_servers.linear.api_key", label: "Linear API key", source }],
  });
  const server = (status: ToolServer["status"], message: string | null = null): ToolServer => ({
    id: "linear",
    status,
    message,
    tools: [],
  });
  const added = { ...LINEAR, added: true };

  it("is working only once Linear has answered", () => {
    // A key that is merely there said Working for "test".
    expect(statusOf(added, { secrets: key("stored"), toolServers: [server("connecting")] }).text).toBe(
      "Connecting…",
    );
    expect(statusOf(added, { secrets: key("stored"), toolServers: [server("working")] })).toEqual({
      ok: true,
      text: "Working",
    });
  });

  it("says why Linear refused", () => {
    expect(
      statusOf(added, {
        secrets: key("stored"),
        toolServers: [server("failed", "401: the key was refused")],
      }).text,
    ).toBe("401: the key was refused");
  });

  it("asks for a key before anything else", () => {
    expect(statusOf(added, { secrets: key("unset"), toolServers: [server("needs_key")] }).text).toBe(
      "Needs its API key",
    );
  });
});

describe("statusOf ChatGPT", () => {
  const added = { ...CHATGPT, added: true };

  it("is working once signed in", () => {
    expect(statusOf(added, { secrets: chatgptSignIn("stored") })).toEqual({
      ok: true,
      text: "Working",
    });
  });

  it("is not signed in without the sign-in", () => {
    expect(statusOf(added, { secrets: chatgptSignIn("unset") }).text).toBe("Not signed in");
  });

  it("is switched off before anything else", () => {
    expect(
      statusOf(added, { secrets: chatgptSignIn("unset"), chatgpt: { enabled: false } }).text,
    ).toBe("Switched off");
  });
});

describe("AddOnsPage removing", () => {
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
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));

    await waitFor(() =>
      expect(onReset).toHaveBeenCalledWith("integrations.nemlig", ["integrations.nemlig"]),
    );
    expect(clear).toHaveBeenCalledTimes(1);
    expect(clear).toHaveBeenCalledWith("integrations.nemlig.password");
    expect(onSecretsChanged).toHaveBeenCalled();
  });

  it("takes Linear out of the config, not only its key", async () => {
    // Its key went and the section stayed, so the row never left
    // Connected.
    vi.spyOn(api, "clearSecret").mockResolvedValue(undefined);
    const { onReset } = setup([{ ...LINEAR, added: true }], {
      writable: true,
      secrets: [
        { key: "tool_servers.linear.api_key", label: "Linear API key", source: "stored" },
      ],
    });
    fireEvent.click(screen.getByRole("button", { name: "Configure Linear" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));
    await waitFor(() =>
      expect(onReset).toHaveBeenCalledWith("tool_servers.linear", ["tool_servers.linear"]),
    );
  });

  it("deletes the keys before the add-on goes", async () => {
    // So a key that cannot be deleted stops the removal rather than
    // being stranded by it.
    const order: string[] = [];
    vi.spyOn(api, "clearSecret").mockImplementation(async (key) => {
      order.push(`clear ${key}`);
    });
    const { onChange } = setup([UNIFI_ADDED], unifiKey("stored"), "192.168.1.1");
    onChange.mockImplementation((row: string) => order.push(`change ${row}`));
    fireEvent.click(screen.getByRole("button", { name: "Configure UniFi" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));
    await waitFor(() =>
      expect(order).toEqual(["clear presence.unifi.api_key", "change presence.unifi"]),
    );
  });

  it("keeps it when a key cannot be deleted, and says why", async () => {
    vi.spyOn(api, "clearSecret").mockRejectedValue(new Error("no database"));
    const { onReset } = setup([{ ...NEMLIG, added: true }], {
      writable: true,
      secrets: NEMLIG_LOGIN.secrets.map((s) => ({ ...s, source: "stored" as const })),
    });
    fireEvent.click(screen.getByRole("button", { name: "Configure nemlig.com" }));
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));
    expect(await screen.findByText("no database")).toBeInTheDocument();
    expect(onReset).not.toHaveBeenCalled();
  });

  it("says nothing about keys when it has none saved", () => {
    setup([{ ...NEMLIG, added: true }], NEMLIG_LOGIN);
    fireEvent.click(screen.getByRole("button", { name: "Configure nemlig.com" }));
    expect(screen.queryByText(/saved keys are deleted/)).toBeNull();
  });
});

describe("AddOnsPage podcasts", () => {
  // The panel asks the server for the voice list, so these render with
  // a query client like the app has.
  function showPodcasts(integration: Integration, secrets: SecretsReport, podcasts?: object) {
    const onChange = vi.fn();
    const onReset = vi.fn();
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <AddOnsPage
          integrations={[integration]}
          secrets={secrets}
          podcasts={podcasts}
          onChange={onChange}
          onReset={onReset}
          onSecretsChanged={vi.fn()}
          onTadoChanged={vi.fn()}
        />
      </QueryClientProvider>,
    );
    return { onChange, onReset };
  }

  it("says its row is waiting for the Kokoro address, then Set up", () => {
    showPodcasts({ ...PODCASTS, added: true }, unifiKey(), {});
    expect(screen.getByText("Needs its Kokoro address")).toBeInTheDocument();
    cleanup();

    showPodcasts({ ...PODCASTS, added: true }, PODCASTS_KEYS, PODCASTS_SET);
    expect(screen.getByText("Set up")).toBeInTheDocument();
    expect(screen.queryByText("Needs its Kokoro address")).toBeNull();
  });

  it("names what is missing along the way", () => {
    // Everything but the keys: the row says which half is left.
    const unset: SecretsReport = {
      writable: true,
      secrets: PODCASTS_KEYS.secrets.map((s) => ({ ...s, source: "unset" as const })),
    };
    showPodcasts({ ...PODCASTS, added: true }, unset, PODCASTS_SET);
    expect(screen.getByText("Needs its access key")).toBeInTheDocument();
    cleanup();

    showPodcasts(
      { ...PODCASTS, added: true },
      PODCASTS_KEYS,
      { kokoro_url: PODCASTS_SET.kokoro_url },
    );
    expect(screen.getByText("Needs its R2 bucket")).toBeInTheDocument();
  });

  it("adds it with a persistable retention setting, and opens it to be filled in", () => {
    const { onChange } = showPodcasts(PODCASTS, unifiKey());
    fireEvent.click(screen.getByRole("button", { name: "Connect Podcasts" }));
    expect(onChange).toHaveBeenCalledWith("integrations.podcasts", [
      { path: "integrations.podcasts.keep_days", value: DEFAULT_KEEP_DAYS },
    ]);
    expect(screen.getByLabelText("Kokoro address")).toBeTruthy();
  });

  it("removing it clears both stored keys, then resets the section", async () => {
    const clear = vi.spyOn(api, "clearSecret").mockResolvedValue(undefined);
    const { onReset } = showPodcasts({ ...PODCASTS, added: true }, PODCASTS_KEYS, PODCASTS_SET);
    fireEvent.click(screen.getByRole("button", { name: "Configure Podcasts" }));
    expect(screen.getByText("Its saved keys are deleted too.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Remove add-on/ }));

    await waitFor(() =>
      expect(onReset).toHaveBeenCalledWith("integrations.podcasts", ["integrations.podcasts"]),
    );
    expect(clear.mock.calls.map(([key]) => key).sort()).toEqual([...PODCASTS.secret_keys].sort());
  });
});
